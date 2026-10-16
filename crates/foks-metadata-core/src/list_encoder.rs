//! Incremental public List encoding; no authorization or response-lifetime proof.
//! The caller must retain admission through the final physical socket write.
use crate::{Error, Result};
use foks_proto::{KvDirent, KvSmallFileBox};
use foks_snowpack::{encode, reserve_allocation, Value};

pub const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 4096;
const MAX_DIRENT_BYTES: usize = 4096;
const MAX_SMALL_BYTES: usize = 65536;

/// Retains encoded bodies only. Capacity is at most twice the payload ceiling;
/// finalization temporarily adds one exact-size output. Allocation admission
/// charges every new allocation before it coexists with any old allocation.
pub struct ListEncoder {
    entries: Vec<u8>,
    extended: Vec<u8>,
    count: usize,
    extended_count: usize,
    maximum_entries: usize,
    maximum_bytes: usize,
}

impl ListEncoder {
    pub fn new(maximum_entries: usize) -> Result<Self> {
        Self::with_limit(maximum_entries, MAX_LIST_BYTES)
    }

    fn with_limit(maximum_entries: usize, maximum_bytes: usize) -> Result<Self> {
        if maximum_entries > MAX_ENTRIES || !(4..=MAX_LIST_BYTES).contains(&maximum_bytes) {
            return Err(Error::Bounds);
        }
        Ok(Self {
            entries: Vec::new(),
            extended: Vec::new(),
            count: 0,
            extended_count: 0,
            maximum_entries,
            maximum_bytes,
        })
    }

    /// Encode both pieces and reserve both buffers before changing the accepted
    /// prefix. Failure may increase capacity, but cannot append half an entry.
    pub fn append(&mut self, entry: &KvDirent, small: Option<&KvSmallFileBox>) -> Result<()> {
        if self.count >= self.maximum_entries
            || entry.name_box.ciphertext.len() > MAX_DIRENT_BYTES
            || small.is_some_and(|small| small.ciphertext.len() > MAX_SMALL_BYTES)
        {
            return Err(Error::Bounds);
        }
        // The schema Value constructors clone binary fields before encode() can
        // charge its output. Admit those bounded copies and container storage.
        let schema_bytes = entry.name_box.ciphertext.len()
            + small.map_or(0, |small| small.ciphertext.len())
            + 512
            + 40 * std::mem::size_of::<Value>();
        charge(schema_bytes)?;
        let entry = encode(&entry.to_value()).map_err(|_| Error::Bounds)?;
        if entry.len() > MAX_DIRENT_BYTES {
            return Err(Error::Bounds);
        }
        let small = small
            .map(|small| encode(&small.to_value()).map_err(|_| Error::Bounds))
            .transpose()?;
        if small
            .as_ref()
            .is_some_and(|small| small.len() > MAX_SMALL_BYTES)
        {
            return Err(Error::Bounds);
        }
        let position = small
            .as_ref()
            .map(|_| encode(&Value::Unsigned(self.count as u64)).map_err(|_| Error::Bounds))
            .transpose()?;
        let extended_add = match (&small, &position) {
            (Some(small), Some(position)) => 1 + position.len() + small.len(),
            (None, None) => 0,
            _ => unreachable!("position exists exactly when a small body exists"),
        };
        let count = self.count + 1;
        let extended_count = self.extended_count + usize::from(small.is_some());
        let entries_len = self
            .entries
            .len()
            .checked_add(entry.len())
            .ok_or(Error::Bounds)?;
        let extended_len = self
            .extended
            .len()
            .checked_add(extended_add)
            .ok_or(Error::Bounds)?;
        if encoded_length(count, extended_count, entries_len, extended_len)? > self.maximum_bytes {
            return Err(Error::Bounds);
        }
        grow(&mut self.entries, entries_len, self.maximum_bytes)?;
        grow(&mut self.extended, extended_len, self.maximum_bytes)?;
        self.entries.extend_from_slice(&entry);
        if let (Some(small), Some(position)) = (small, position) {
            self.extended.push(0x92);
            self.extended.extend_from_slice(&position);
            self.extended.extend_from_slice(&small);
        }
        self.count = count;
        self.extended_count = extended_count;
        Ok(())
    }

    pub fn finish(self, final_page: bool) -> Result<Vec<u8>> {
        let length = encoded_length(
            self.count,
            self.extended_count,
            self.entries.len(),
            self.extended.len(),
        )?;
        charge(length)?;
        let mut output = Vec::new();
        output.try_reserve_exact(length).map_err(|_| {
            #[cfg(feature = "list-allocation-qualification")]
            crate::allocation_trace::allocator_failed();
            crate::Error::Bounds
        })?;
        output.push(0x93);
        prefix(&mut output, self.count);
        output.extend_from_slice(&self.entries);
        output.push(if final_page { 0xc3 } else { 0xc2 });
        prefix(&mut output, self.extended_count);
        output.extend_from_slice(&self.extended);
        debug_assert_eq!(output.len(), length);
        Ok(output)
    }
}

fn charge(bytes: usize) -> Result<()> {
    reserve_allocation(bytes).map_err(|_| Error::Bounds)
}

fn grow(buffer: &mut Vec<u8>, needed: usize, maximum: usize) -> Result<()> {
    if needed <= buffer.capacity() {
        return Ok(());
    }
    let capacity = needed.max(buffer.capacity().saturating_mul(2)).min(maximum);
    if needed > capacity {
        return Err(Error::Bounds);
    }
    // Reserve the whole new allocation before reallocating; admission still
    // owns the old one. reserve_exact avoids unaccounted Vec geometric growth.
    #[cfg(feature = "list-allocation-qualification")]
    let _phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Growth);
    charge(capacity)?;
    buffer
        .try_reserve_exact(capacity - buffer.len())
        .map_err(|_| {
            #[cfg(feature = "list-allocation-qualification")]
            crate::allocation_trace::allocator_failed();
            crate::Error::Bounds
        })
}

fn encoded_length(
    count: usize,
    extended_count: usize,
    entries: usize,
    extended: usize,
) -> Result<usize> {
    entries
        .checked_add(extended)
        .and_then(|length| {
            length.checked_add(2 + prefix_length(count) + prefix_length(extended_count))
        })
        .ok_or(Error::Bounds)
}

fn prefix_length(count: usize) -> usize {
    if count <= 15 {
        1
    } else {
        3
    }
}
fn prefix(output: &mut Vec<u8>, count: usize) {
    match count {
        0 => output.push(0xc0),
        1..=15 => output.push(0x90 | count as u8),
        _ => {
            output.push(0xdc);
            output.extend_from_slice(&(count as u16).to_be_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::{
        KvDirectoryStatus, KvExtendedDirent, KvListResponse, KvNodeId, Role, RoleAndGeneration,
        SecretBox,
    };
    use foks_snowpack::{with_allocation_budget, AllocationBudget};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn entry(n: usize) -> KvDirent {
        let mut id = [0; 16];
        id[8..].copy_from_slice(&(n as u64).to_be_bytes());
        KvDirent::new(
            [0; 16],
            id,
            KvNodeId([3; 17]),
            1,
            1,
            Role::OWNER,
            [2; 32],
            SecretBox {
                nonce: [3; 16],
                ciphertext: vec![4; 32],
            },
            KvDirectoryStatus::Active,
            [5; 32],
            n as u64,
        )
        .unwrap()
    }
    fn small(size: usize) -> KvSmallFileBox {
        KvSmallFileBox {
            key: RoleAndGeneration {
                role: Role::OWNER,
                generation: 1,
            },
            ciphertext: vec![6; size],
        }
    }
    fn oracle(entries: Vec<KvDirent>, bodies: Vec<KvExtendedDirent>, final_page: bool) -> Vec<u8> {
        KvListResponse::new(entries, final_page, bodies)
            .unwrap()
            .encoded()
            .to_vec()
    }

    #[test]
    fn exact_native_bytes_at_all_array_and_position_boundaries() {
        for n in [0, 1, 15, 16, 127, 128, 129, 255, 256, 257, 4096] {
            for sparse in [false, true] {
                for final_page in [false, true] {
                    let mut encoder = ListEncoder::new(n).unwrap();
                    let mut entries = Vec::new();
                    let mut bodies = Vec::new();
                    for i in 0..n {
                        let mut entry = entry(i);
                        // Deliberately change the semantic fields without updating
                        // cached exact bytes: normalization must use the schema.
                        entry.creation_time = u64::MAX - i as u64;
                        let body = (!sparse || i % 3 == 0).then(|| small(17));
                        encoder.append(&entry, body.as_ref()).unwrap();
                        entries.push(entry);
                        if let Some(small_file) = body {
                            bodies.push(KvExtendedDirent {
                                position: i as u64,
                                small_file,
                            });
                        }
                    }
                    assert_eq!(
                        encoder.finish(final_page).unwrap(),
                        oracle(entries, bodies, final_page)
                    );
                }
            }
        }
    }

    struct Refuse {
        call: AtomicUsize,
        at: usize,
    }
    impl AllocationBudget for Refuse {
        fn reserve(&self, _bytes: usize) -> bool {
            self.call.fetch_add(1, Ordering::SeqCst) + 1 != self.at
        }
    }

    #[test]
    fn each_append_allocation_failure_keeps_the_accepted_prefix() {
        let first = entry(0);
        let second = entry(1);
        let body = small(32);
        for at in 1..=6 {
            let mut encoder = ListEncoder::new(2).unwrap();
            encoder.append(&first, None).unwrap();
            let refusal = Arc::new(Refuse {
                call: AtomicUsize::new(0),
                at,
            });
            assert_eq!(
                with_allocation_budget(refusal.clone(), || encoder.append(&second, Some(&body))),
                Err(Error::Bounds)
            );
            assert_eq!(refusal.call.load(Ordering::SeqCst), at);
            assert_eq!(
                encoder.finish(true).unwrap(),
                oracle(vec![first.clone()], vec![], true)
            );
        }
    }

    #[test]
    fn failed_final_allocation_produces_no_response() {
        let encoder = ListEncoder::new(0).unwrap();
        let refusal = Arc::new(Refuse {
            call: AtomicUsize::new(0),
            at: 1,
        });
        assert_eq!(
            with_allocation_budget(refusal, || encoder.finish(true)),
            Err(Error::Bounds)
        );
    }

    #[test]
    fn exact_logical_limit_and_overflow_leave_prefix_valid() {
        let entry = entry(0);
        let body = small(300);
        let expected = oracle(
            vec![entry.clone()],
            vec![KvExtendedDirent {
                position: 0,
                small_file: body.clone(),
            }],
            true,
        );
        for limit in [expected.len() - 1, expected.len(), expected.len() + 1] {
            let mut encoder = ListEncoder::with_limit(2, limit).unwrap();
            if limit < expected.len() {
                assert_eq!(encoder.append(&entry, Some(&body)), Err(Error::Bounds));
                assert_eq!(encoder.finish(true).unwrap(), oracle(vec![], vec![], true));
            } else {
                encoder.append(&entry, Some(&body)).unwrap();
                assert_eq!(encoder.append(&entry, None), Err(Error::Bounds));
                assert_eq!(encoder.finish(true).unwrap(), expected);
            }
        }
    }

    #[test]
    fn count_and_per_node_bounds_are_checked_before_schema_copies() {
        let mut encoder = ListEncoder::new(1).unwrap();
        let mut huge = entry(0);
        huge.name_box.ciphertext.resize(MAX_DIRENT_BYTES + 1, 0);
        let refusal = Arc::new(Refuse {
            call: AtomicUsize::new(0),
            at: 1,
        });
        assert_eq!(
            with_allocation_budget(refusal.clone(), || encoder.append(&huge, None)),
            Err(Error::Bounds)
        );
        assert_eq!(refusal.call.load(Ordering::SeqCst), 0);
        encoder.append(&entry(0), None).unwrap();
        assert_eq!(encoder.append(&entry(1), None), Err(Error::Bounds));
        assert!(ListEncoder::new(MAX_ENTRIES + 1).is_err());
        assert_eq!(
            encoder.finish(false).unwrap(),
            oracle(vec![entry(0)], vec![], false)
        );
    }

    #[test]
    fn maximum_valid_stored_nodes_match_native_normalization() {
        let entry = (MAX_DIRENT_BYTES - 512..MAX_DIRENT_BYTES)
            .find_map(|size| {
                let mut value = entry(0);
                value.name_box.ciphertext.resize(size, 7);
                (value.encode().unwrap().len() == MAX_DIRENT_BYTES).then_some(value)
            })
            .unwrap();
        let body = (MAX_SMALL_BYTES - 128..MAX_SMALL_BYTES)
            .map(small)
            .find(|value| value.encode().unwrap().len() == MAX_SMALL_BYTES)
            .unwrap();
        let mut encoder = ListEncoder::new(1).unwrap();
        encoder.append(&entry, Some(&body)).unwrap();
        assert_eq!(
            encoder.finish(false).unwrap(),
            oracle(
                vec![entry],
                vec![KvExtendedDirent {
                    position: 0,
                    small_file: body
                }],
                false
            )
        );
    }

    // Construct a real List of any length near the public maximum, without
    // filling one invalid oversized small node or imposing a lower payload cap.
    fn near_limit(length: usize) -> Vec<u8> {
        let mut encoder = ListEncoder::new(256).unwrap();
        let body = small(65500);
        for i in 0..255 {
            encoder.append(&entry(i), Some(&body)).unwrap();
        }
        let last = entry(255);
        let base = encoded_length(
            256,
            256,
            encoder.entries.len() + last.encode().unwrap().len(),
            encoder.extended.len() + 1 + 2 + small(300).encode().unwrap().len(),
        )
        .unwrap();
        let tail = small(300 + length - base);
        encoder.append(&last, Some(&tail)).unwrap();
        let output = encoder.finish(false).unwrap();
        assert_eq!(output.len(), length);
        output
    }

    #[test]
    fn actual_rpc_envelope_keeps_its_original_precise_limit() {
        let sequence = 256;
        let empty = oracle(vec![], vec![], true);
        let framed = foks_rpc::encode_success_response_at(&empty, sequence).unwrap();
        let content = foks_rpc::read_frame(&mut framed.as_slice(), MAX_LIST_BYTES).unwrap();
        let overhead = content.len() - empty.len();
        for length in [
            MAX_LIST_BYTES - overhead - 1,
            MAX_LIST_BYTES - overhead,
            MAX_LIST_BYTES - overhead + 1,
            MAX_LIST_BYTES,
        ] {
            let output = near_limit(length);
            assert_eq!(
                foks_rpc::encode_success_response_at(&output, sequence).is_ok(),
                length <= MAX_LIST_BYTES - overhead
            );
        }
    }
}

#[cfg(all(test, feature = "list-allocation-qualification"))]
mod trace_tests {
    use super::*;
    use foks_proto::{KvDirectoryStatus, KvNodeId, Role, RoleAndGeneration, SecretBox};
    use foks_snowpack::{with_allocation_budget, AllocationBudget};
    use std::sync::{Arc, Mutex};
    struct Trace(Mutex<Vec<usize>>);
    impl AllocationBudget for Trace {
        fn reserve(&self, b: usize) -> bool {
            self.0.lock().unwrap().push(b);
            true
        }
    }
    #[test]
    fn growth_events_match_private_capacity_observation_and_finish_exactly() {
        let d = KvDirent::new(
            [0; 16],
            [0; 16],
            KvNodeId([3; 17]),
            1,
            1,
            Role::OWNER,
            [0; 32],
            SecretBox {
                nonce: [0; 16],
                ciphertext: vec![7; 100],
            },
            KvDirectoryStatus::Active,
            [0; 32],
            0,
        )
        .unwrap();
        let small = KvSmallFileBox {
            key: RoleAndGeneration {
                role: Role::OWNER,
                generation: 1,
            },
            ciphertext: vec![8; 1000],
        };
        let mut encoder = ListEncoder::new(40).unwrap();
        for _ in 0..40 {
            let before = (encoder.entries.capacity(), encoder.extended.capacity());
            let r = Arc::new(Trace(Mutex::new(Vec::with_capacity(8))));
            with_allocation_budget(r.clone(), || encoder.append(&d, Some(&small))).unwrap();
            let after = (encoder.entries.capacity(), encoder.extended.capacity());
            let events = r.0.lock().unwrap();
            let mut suffix = Vec::new();
            if after.0 > before.0 {
                suffix.push(after.0);
            }
            if after.1 > before.1 {
                suffix.push(after.1);
            }
            assert!(events.ends_with(&suffix));
            assert_eq!(events.len(), 4 + suffix.len());
        }
        let expected = encoded_length(
            encoder.count,
            encoder.extended_count,
            encoder.entries.len(),
            encoder.extended.len(),
        )
        .unwrap();
        let r = Arc::new(Trace(Mutex::new(Vec::with_capacity(1))));
        let bytes = with_allocation_budget(r.clone(), || encoder.finish(false)).unwrap();
        assert_eq!(bytes.len(), expected);
        assert_eq!(*r.0.lock().unwrap(), [expected]);
    }
}
