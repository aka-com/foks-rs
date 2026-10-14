//! Controller-side evidence for recovery experiments. This is deliberately
//! independent of the server database and does not implement replication.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest as _, Sha256};

/// Append-only, fsynced observations owned by the test controller. Operation
/// identifiers must be non-secret; payloads are represented only by SHA-256.
/// Use synthetic payloads: hashes can still reveal low-entropy real secrets.
pub struct RecoveryLedger {
    file: File,
    started: Instant,
    fault: Option<Instant>,
    budget: Duration,
    acknowledged: BTreeMap<String, [u8; 32]>,
    observed_after_fault: BTreeSet<String>,
    wrote_after_fault: bool,
}

impl RecoveryLedger {
    pub fn create(path: impl AsRef<Path>, budget: Duration) -> io::Result<Self> {
        if budget.is_zero() {
            return Err(io::Error::other("recovery budget must be positive"));
        }
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut ledger = Self {
            file,
            started: Instant::now(),
            fault: None,
            budget,
            acknowledged: BTreeMap::new(),
            observed_after_fault: BTreeSet::new(),
            wrote_after_fault: false,
        };
        ledger.append(json!({"event": "start", "budget_ms": budget.as_millis()}))?;
        Ok(ledger)
    }

    /// Call only after the real protocol client reports success. The immutable
    /// operation ID identifies an expected file, not a mutable pathname shared
    /// between multiple operations.
    pub fn acknowledged(&mut self, operation: &str, payload: &[u8]) -> io::Result<()> {
        if self.acknowledged.contains_key(operation) {
            return Err(io::Error::other("duplicate recovery operation ID"));
        }
        let digest: [u8; 32] = Sha256::digest(payload).into();
        self.append(json!({"event": "acknowledged", "operation": operation, "sha256": digest}))?;
        self.acknowledged.insert(operation.to_owned(), digest);
        if self.fault.is_some() {
            self.wrote_after_fault = true;
        }
        Ok(())
    }

    /// Check a real read against controller-side evidence, never against a
    /// restored database or a client cache used as the source of expectations.
    pub fn observed(&mut self, operation: &str, payload: &[u8]) -> io::Result<()> {
        let digest: [u8; 32] = Sha256::digest(payload).into();
        if self.acknowledged.get(operation) != Some(&digest) {
            return Err(io::Error::other(
                "recovered payload does not match acknowledgment",
            ));
        }
        self.append(json!({"event": "observed", "operation": operation, "sha256": digest}))?;
        if self.fault.is_some() {
            self.observed_after_fault.insert(operation.to_owned());
        }
        Ok(())
    }

    /// Start the stopwatch before issuing the fault. Logging, fault injection,
    /// restart, reconnection, and authenticated operations all consume budget.
    pub fn fault(&mut self, kind: &str) -> io::Result<()> {
        if self.fault.is_some() {
            return Err(io::Error::other("recovery fault already active"));
        }
        self.fault = Some(Instant::now());
        self.observed_after_fault.clear();
        self.wrote_after_fault = false;
        self.append(json!({"event": "fault", "kind": kind}))
    }

    /// Call after authenticated reads verified all prior acknowledgments and a
    /// new write was acknowledged and read back. This is a measured assertion,
    /// not cancellation: a slow blocking RPC can finish after the budget.
    pub fn recovered(&mut self) -> io::Result<Duration> {
        let elapsed = self
            .fault
            .ok_or_else(|| io::Error::other("no recovery fault active"))?
            .elapsed();
        if !self.wrote_after_fault || self.observed_after_fault.len() != self.acknowledged.len() {
            return Err(io::Error::other(
                "recovery needs all acknowledged reads and a new write",
            ));
        }
        self.append(json!({
            "event": "recovered", "elapsed_ms": elapsed.as_millis(),
            "within_budget": elapsed <= self.budget,
        }))?;
        self.fault = None;
        if elapsed > self.budget {
            return Err(io::Error::other(format!(
                "recovery took {elapsed:?}, exceeding {:?}",
                self.budget
            )));
        }
        Ok(elapsed)
    }

    fn append(&mut self, mut event: serde_json::Value) -> io::Result<()> {
        event["controller_elapsed_ms"] = json!(self.started.elapsed().as_millis());
        serde_json::to_writer(&mut self.file, &event)?;
        self.file.write_all(b"\n")?;
        self.file.sync_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_requires_every_acknowledged_read_and_a_new_write() {
        let directory = tempfile::tempdir().unwrap();
        let mut ledger = RecoveryLedger::create(
            directory.path().join("evidence.jsonl"),
            Duration::from_secs(15),
        )
        .unwrap();
        ledger.acknowledged("before", b"old value").unwrap();
        assert!(ledger.acknowledged("before", b"other value").is_err());
        ledger.observed("before", b"old value").unwrap();
        assert!(ledger.recovered().is_err());
        ledger.fault("test fault").unwrap();
        assert!(ledger.fault("nested fault").is_err());
        ledger.acknowledged("after", b"new value").unwrap();
        ledger.observed("after", b"new value").unwrap();
        assert!(
            ledger.recovered().is_err(),
            "a pre-fault read cannot satisfy recovery"
        );
        assert!(ledger.observed("before", b"corrupted value").is_err());
        assert!(ledger.observed("unknown", b"old value").is_err());
        ledger.observed("before", b"old value").unwrap();
        ledger.recovered().unwrap();

        ledger.fault("second fault").unwrap();
        ledger.observed("before", b"old value").unwrap();
        ledger.observed("after", b"new value").unwrap();
        assert!(
            ledger.recovered().is_err(),
            "prior-cycle write cannot satisfy recovery"
        );
    }

    #[test]
    fn recovery_outside_budget_records_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("evidence.jsonl");
        assert!(RecoveryLedger::create(&path, Duration::ZERO).is_err());
        let mut ledger = RecoveryLedger::create(&path, Duration::from_nanos(1)).unwrap();
        assert!(RecoveryLedger::create(&path, Duration::from_secs(15)).is_err());
        ledger.fault("test fault").unwrap();
        ledger.acknowledged("after", b"value").unwrap();
        ledger.observed("after", b"value").unwrap();
        assert!(ledger.recovered().is_err());
        let records = std::fs::read_to_string(&path).unwrap();
        let last: serde_json::Value =
            serde_json::from_str(records.lines().last().unwrap()).unwrap();
        assert_eq!(last["event"], "recovered");
        assert_eq!(last["within_budget"], false);
    }
}
