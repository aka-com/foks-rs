use crate::{DeadlineEnvelope, Decimal, Domain, Error, Result, Version};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockSample {
    domain: Domain,
    nanos: Decimal,
}
impl ClockSample {
    pub fn domain(&self) -> &Domain {
        &self.domain
    }
    pub fn nanos(&self) -> u64 {
        self.nanos.0
    }
}

/// Read the platform clock and its domain. No wall-clock fallback or cached
/// namespace identity is permitted. Acquisition failures remain explicit.
pub fn sample() -> Result<ClockSample> {
    platform_sample()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn nanos(clock: nix::time::ClockId) -> Result<Decimal> {
    let t = nix::time::clock_gettime(clock).map_err(|e| Error::Unavailable(e.to_string()))?;
    let seconds = u64::try_from(t.tv_sec()).map_err(|_| Error::Range)?;
    let sub = u64::try_from(t.tv_nsec()).map_err(|_| Error::Range)?;
    if sub >= 1_000_000_000 {
        return Err(Error::Range);
    }
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(sub))
        .map(Decimal)
        .ok_or(Error::Range)
}

#[cfg(target_os = "linux")]
fn linux_domain() -> Result<Domain> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    let f = std::fs::File::open("/proc/sys/kernel/random/boot_id")
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    let mut bytes = Vec::with_capacity(38);
    f.take(38)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    if bytes.len() != 37 || bytes[36] != b'\n' {
        return Err(Error::Range);
    }
    let text = std::str::from_utf8(&bytes[..36]).map_err(|_| Error::Range)?;
    let boot = crate::BootId::parse(text)?;
    let metadata = std::fs::metadata("/proc/thread-self/ns/time")
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    Ok(Domain(crate::DomainValue::Linux {
        boot,
        device: Decimal(metadata.dev()),
        inode: Decimal(metadata.ino()),
    }))
}
#[cfg(target_os = "linux")]
fn platform_sample() -> Result<ClockSample> {
    let domain = linux_domain()?;
    let nanos = nanos(nix::time::ClockId::CLOCK_MONOTONIC)?;
    if linux_domain()? != domain {
        return Err(Error::DomainMismatch);
    }
    Ok(ClockSample { domain, nanos })
}
#[cfg(target_os = "macos")]
fn platform_sample() -> Result<ClockSample> {
    use sysctl::Sysctl;
    let text = sysctl::Ctl::new("kern.bootsessionuuid")
        .and_then(|c| c.value_string())
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    if text.len() != 36 {
        return Err(Error::Range);
    }
    let boot = crate::BootId::parse(&text.to_ascii_lowercase())?;
    let domain = Domain(crate::DomainValue::Darwin { boot });
    let nanos = nanos(nix::time::ClockId::from_raw(libc::CLOCK_UPTIME_RAW))?;
    Ok(ClockSample { domain, nanos })
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_sample() -> Result<ClockSample> {
    Err(Error::Unavailable(
        "this OS has no qualified local deadline clock".into(),
    ))
}

trait Clock {
    fn instant(&self) -> Instant;
    fn sample(&self) -> Result<ClockSample>;
}
struct SystemClock;
impl Clock for SystemClock {
    fn instant(&self) -> Instant {
        Instant::now()
    }
    fn sample(&self) -> Result<ClockSample> {
        sample()
    }
}

#[derive(Clone, Debug)]
pub struct Deadline {
    envelope: DeadlineEnvelope,
    instant: Instant,
}
impl Deadline {
    /// The caller creates this instant before encoding, admission or clock reads.
    /// Preserve both coordinates when joining controls from the same clock domain.
    pub fn intersect(&self, other: &Self) -> Result<Self> {
        self.intersect_with(other, &SystemClock)
    }
    fn intersect_with(&self, other: &Self, clock: &impl Clock) -> Result<Self> {
        if self.envelope.domain != other.envelope.domain {
            return Err(Error::DomainMismatch);
        }
        let mut merged = self.clone();
        merged.instant = self.instant.min(other.instant);
        merged.envelope.expires_ns.0 = self.envelope.expires_ns.0.min(other.envelope.expires_ns.0);
        merged.check_with(clock)?;
        Ok(merged)
    }

    pub fn capture(original: Instant) -> Result<Self> {
        Self::capture_with(original, &SystemClock)
    }
    pub fn receive(envelope: &DeadlineEnvelope, cap: Duration) -> Result<Self> {
        Self::receive_with(envelope, cap, &SystemClock)
    }
    pub fn shorten(&self, earlier: Instant) -> Result<Self> {
        self.shorten_with(earlier, &SystemClock)
    }
    pub fn check(&self) -> Result<()> {
        self.check_with(&SystemClock)
    }
    pub fn envelope(&self) -> &DeadlineEnvelope {
        &self.envelope
    }
    pub fn instant(&self) -> Instant {
        self.instant
    }

    fn capture_with(original: Instant, clock: &impl Clock) -> Result<Self> {
        if clock.instant() >= original {
            return Err(Error::Expired);
        }
        let sample = clock.sample()?;
        let remaining = remaining(original, clock.instant())?;
        let expires = sample.nanos.0.checked_add(remaining).ok_or(Error::Range)?;
        Ok(Self {
            instant: original,
            envelope: DeadlineEnvelope {
                version: Version,
                domain: sample.domain,
                expires_ns: Decimal(expires),
            },
        })
    }
    fn receive_with(
        envelope: &DeadlineEnvelope,
        cap: Duration,
        clock: &impl Clock,
    ) -> Result<Self> {
        let before = clock.instant();
        let sample = clock.sample()?;
        if sample.domain != envelope.domain {
            return Err(Error::DomainMismatch);
        }
        let rest = envelope
            .expires_ns
            .0
            .checked_sub(sample.nanos.0)
            .filter(|n| *n > 0)
            .ok_or(Error::Expired)?;
        let instant = before
            .checked_add(Duration::from_nanos(rest).min(cap))
            .ok_or(Error::Range)?;
        // Propagation also consumes sampling cost; sample+fullcap would refresh
        // that cost at the next receiver even though our local instant did not.
        let rest_after = remaining(instant, clock.instant())?;
        let expires = sample.nanos.0.checked_add(rest_after).ok_or(Error::Range)?;
        Ok(Self {
            instant,
            envelope: DeadlineEnvelope {
                version: Version,
                domain: sample.domain,
                expires_ns: Decimal(expires.min(envelope.expires_ns.0)),
            },
        })
    }
    fn shorten_with(&self, earlier: Instant, clock: &impl Clock) -> Result<Self> {
        let mut next = Self::capture_with(earlier.min(self.instant), clock)?;
        if next.envelope.domain != self.envelope.domain {
            return Err(Error::DomainMismatch);
        }
        next.envelope.expires_ns.0 = next.envelope.expires_ns.0.min(self.envelope.expires_ns.0);
        next.check_with(clock)?;
        Ok(next)
    }
    fn check_with(&self, clock: &impl Clock) -> Result<()> {
        if clock.instant() >= self.instant {
            return Err(Error::Expired);
        }
        let sample = clock.sample()?;
        if sample.domain != self.envelope.domain {
            return Err(Error::DomainMismatch);
        }
        if sample.nanos.0 >= self.envelope.expires_ns.0 || clock.instant() >= self.instant {
            return Err(Error::Expired);
        }
        Ok(())
    }
}
fn remaining(deadline: Instant, now: Instant) -> Result<u64> {
    let duration = deadline
        .checked_duration_since(now)
        .filter(|n| !n.is_zero())
        .ok_or(Error::Expired)?;
    u64::try_from(duration.as_nanos()).map_err(|_| Error::Range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Fake {
        base: Instant,
        elapsed: Cell<u64>,
        sample_cost: u64,
        os_base: u64,
        domain: Domain,
    }
    impl Fake {
        fn new(cost: u64) -> Self {
            Self {
                base: Instant::now(),
                elapsed: Cell::new(0),
                sample_cost: cost,
                os_base: 100,
                domain: crate::tests::domain(),
            }
        }
    }
    impl Clock for Fake {
        fn instant(&self) -> Instant {
            self.base + Duration::from_nanos(self.elapsed.get())
        }
        fn sample(&self) -> Result<ClockSample> {
            let nanos = Decimal(self.os_base + self.elapsed.get());
            self.elapsed.set(self.elapsed.get() + self.sample_cost);
            Ok(ClockSample {
                domain: self.domain.clone(),
                nanos,
            })
        }
    }
    #[test]
    fn capture_spends_sampling_time_and_rejects_expiry_and_overflow() {
        let clock = Fake::new(4);
        let d = Deadline::capture_with(clock.base + Duration::from_nanos(10), &clock).unwrap();
        assert_eq!(d.envelope.expires_ns(), 106);
        let clock = Fake::new(10);
        assert!(matches!(
            Deadline::capture_with(clock.base + Duration::from_nanos(10), &clock),
            Err(Error::Expired)
        ));
        let mut clock = Fake::new(0);
        clock.os_base = u64::MAX;
        assert!(matches!(
            Deadline::capture_with(clock.base + Duration::from_nanos(1), &clock),
            Err(Error::Range)
        ));
    }
    #[test]
    fn receive_and_shortening_never_refresh_cap_or_propagated_coordinate() {
        let clock = Fake::new(2);
        let envelope = DeadlineEnvelope {
            version: Version,
            domain: clock.domain.clone(),
            expires_ns: Decimal(200),
        };
        let d = Deadline::receive_with(&envelope, Duration::from_nanos(10), &clock).unwrap();
        assert_eq!(d.instant, clock.base + Duration::from_nanos(10));
        assert_eq!(d.envelope.expires_ns(), 108);
        let next = d
            .shorten_with(clock.base + Duration::from_nanos(1000), &clock)
            .unwrap();
        assert_eq!(next.instant, d.instant);
        assert!(next.envelope.expires_ns() <= d.envelope.expires_ns());
        let clock = Fake::new(10);
        assert!(matches!(
            Deadline::receive_with(&envelope, Duration::from_nanos(10), &clock),
            Err(Error::Expired)
        ));
    }
    #[test]
    fn both_clock_bounds_and_domain_are_enforced() {
        let mut clock = Fake::new(0);
        let d = Deadline::capture_with(clock.base + Duration::from_nanos(10), &clock).unwrap();
        clock.os_base = 110;
        assert!(matches!(d.check_with(&clock), Err(Error::Expired)));
        clock.os_base = 100;
        clock.elapsed.set(10);
        assert!(matches!(d.check_with(&clock), Err(Error::Expired)));
        clock.elapsed.set(0);
        clock.domain = Domain(crate::DomainValue::Linux {
            boot: crate::BootId::parse("00000000-0000-0000-0000-000000000001").unwrap(),
            device: Decimal(1),
            inode: Decimal(2),
        });
        assert!(matches!(d.check_with(&clock), Err(Error::DomainMismatch)));
        assert!(matches!(
            Deadline::receive_with(d.envelope(), Duration::from_secs(1), &clock),
            Err(Error::DomainMismatch)
        ));
    }

    #[test]
    fn earlier_shortening_and_checks_spend_sampling_delay() {
        let clock = Fake::new(0);
        let d = Deadline::capture_with(clock.base + Duration::from_nanos(100), &clock).unwrap();
        let next = d
            .shorten_with(clock.base + Duration::from_nanos(10), &clock)
            .unwrap();
        assert_eq!(next.instant, clock.base + Duration::from_nanos(10));
        assert_eq!(next.envelope.expires_ns(), 110);
        assert!(matches!(
            Deadline::receive_with(d.envelope(), Duration::ZERO, &clock),
            Err(Error::Expired)
        ));
        let slow = Fake {
            base: clock.base,
            sample_cost: 10,
            ..Fake::new(0)
        };
        assert!(matches!(next.check_with(&slow), Err(Error::Expired)));
        let mut expired = d.envelope.clone();
        expired.expires_ns = Decimal(100);
        assert!(matches!(
            Deadline::receive_with(&expired, Duration::from_secs(1), &clock),
            Err(Error::Expired)
        ));
    }
    #[test]
    fn intersection_keeps_both_earliest_coordinates() {
        let clock = Fake::new(0);
        let first = Deadline::capture_with(clock.base + Duration::from_nanos(100), &clock).unwrap();
        let mut second = first.clone();
        second.instant = clock.base + Duration::from_nanos(50);
        second.envelope.expires_ns = Decimal(300);
        let merged = first.intersect_with(&second, &clock).unwrap();
        assert_eq!(merged.instant, second.instant);
        assert_eq!(merged.envelope.expires_ns, first.envelope.expires_ns);
        let mut wrong = second;
        wrong.envelope.domain = Domain(crate::DomainValue::Darwin {
            boot: crate::BootId::parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap(),
        });
        assert!(matches!(
            first.intersect_with(&wrong, &clock),
            Err(Error::DomainMismatch)
        ));
    }
}
