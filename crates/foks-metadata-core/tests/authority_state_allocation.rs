//! Requested-live allocation qualification for the preserved typed state codec.
//! The tracker is test-only, thread-local, fixed-capacity and excludes input/mock
//! construction. It models realloc old+new coexistence, not allocator RSS.
use foks_metadata_core::{
    authority::{self, Activation, Caller, Fence, Identity, Invocation, Number, Request},
    Error, Limits, Result, Sql, Statement, Value,
};
use foks_snowpack::{with_allocation_budget, AllocationBudget};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::RefCell,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Clone, Copy)]
struct Slot {
    address: usize,
    bytes: usize,
}
const EMPTY: Slot = Slot {
    address: 0,
    bytes: 0,
};
struct Tracking {
    enabled: bool,
    slots: [Slot; 128],
    live: usize,
    peak: usize,
    traffic: usize,
    overflow: bool,
}
impl Tracking {
    const fn new() -> Self {
        Self {
            enabled: false,
            slots: [EMPTY; 128],
            live: 0,
            peak: 0,
            traffic: 0,
            overflow: false,
        }
    }
    fn allocated(&mut self, pointer: *mut u8, bytes: usize) {
        if !self.enabled || pointer.is_null() {
            return;
        }
        if let Some(slot) = self.slots.iter_mut().find(|s| s.address == 0) {
            *slot = Slot {
                address: pointer as usize,
                bytes,
            };
            self.live += bytes;
            self.traffic += bytes;
            self.peak = self.peak.max(self.live);
        } else {
            self.overflow = true;
        }
    }
    fn freed(&mut self, pointer: *mut u8) {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.address == pointer as usize)
        {
            self.live -= slot.bytes;
            *slot = EMPTY;
        }
    }
    fn reallocating(&mut self, pointer: *mut u8, bytes: usize) {
        if self.enabled && self.slots.iter().any(|s| s.address == pointer as usize) {
            // Conservative requested old+new overlap, even if System realloc
            // can grow in place; no claim about physical allocator peak.
            self.peak = self.peak.max(self.live + bytes);
        }
    }
}
thread_local! { static TRACKING: RefCell<Tracking> = const { RefCell::new(Tracking::new()) }; }
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = unsafe { System.alloc(layout) };
        let _ = TRACKING.try_with(|s| s.borrow_mut().allocated(result, layout.size()));
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let result = unsafe { System.alloc_zeroed(layout) };
        let _ = TRACKING.try_with(|s| s.borrow_mut().allocated(result, layout.size()));
        result
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let _ = TRACKING.try_with(|s| s.borrow_mut().freed(pointer));
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        let _ = TRACKING.try_with(|s| s.borrow_mut().reallocating(pointer, bytes));
        let result = unsafe { System.realloc(pointer, layout, bytes) };
        if !result.is_null() {
            let _ = TRACKING.try_with(|s| {
                let mut s = s.borrow_mut();
                s.freed(pointer);
                s.allocated(result, bytes);
            });
        }
        result
    }
}
struct Measurement;
impl Measurement {
    fn begin() -> Self {
        TRACKING.with(|s| {
            let mut s = s.borrow_mut();
            *s = Tracking::new();
            s.enabled = true;
        });
        Self
    }
}
impl Drop for Measurement {
    fn drop(&mut self) {
        TRACKING.with(|s| s.borrow_mut().enabled = false);
    }
}
#[derive(Default)]
struct Budget {
    reserved: AtomicUsize,
    calls: AtomicUsize,
}
impl AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        self.reserved.fetch_add(bytes, Ordering::Relaxed);
        self.calls.fetch_add(1, Ordering::Relaxed);
        true
    }
}
struct Mock {
    row: Option<Vec<Value>>,
    writes: usize,
}
impl Sql for Mock {
    fn query(&mut self, statement: Statement, _: &[Value]) -> Result<Option<Vec<Value>>> {
        assert_eq!(statement, Statement::AuthorityState);
        Ok(self.row.take())
    }
    fn execute(&mut self, statement: Statement, _: &[Value]) -> Result<u64> {
        assert_eq!(statement, Statement::AuthorityClock);
        self.writes += 1;
        Ok(1)
    }
}
fn identity() -> Identity {
    Identity {
        installation: [1; 16],
        tenant: [2; 16],
        host: vec![3; 33],
        cell: [4; 32],
    }
}
fn activation() -> Activation {
    Activation {
        version: Number(1),
        schema: Number(59),
        schema_hash: [5; 32],
        kernel_hash: [6; 32],
        catalog_hash: [7; 32],
        key_generation: Number(1),
        key_digest: [8; 32],
        storage_namespace: [9; 16],
        storage_placement: [10; 32],
        limits: Limits {
            blob_bytes: usize::MAX,
            dirent_bytes: 4096,
            namespace_bytes: u64::MAX,
            namespace_objects: u64::MAX,
            directories: u64::MAX,
            dirents: u64::MAX,
        },
    }
}
fn run_case(id_text: String, active_text: String) {
    assert!(id_text.len() <= 4096 && active_text.len() <= 4096);
    let id = serde_json::from_str::<Identity>(&id_text);
    let active = serde_json::from_str::<Activation>(&active_text);
    let success = id.is_ok() && active.is_ok();
    let expected_calls = if id.is_ok() { 2 } else { 1 };
    let expected_reservation = 67 * id_text.len()
        + 3476
        + if id.is_ok() {
            67 * active_text.len() + 3476
        } else {
            0
        };
    let request = Invocation {
        caller: Caller::Controller {},
        now: Number(1),
        deadline: Number(2),
        request: Request::Inspect {
            fence: Fence {
                identity: id.unwrap_or_else(|_| identity()),
                activation: active.unwrap_or_else(|_| activation()),
                owner: [1; 16],
                epoch: Number(1),
            },
        },
    };
    let mut row = vec![
        Value::Text(id_text),
        Value::Text(active_text),
        Value::Blob(vec![1; 16]),
    ];
    row.extend([1, 1, 0, 0, 0, 256, 1_048_576, 0].map(Value::Integer));
    let mut sql = Mock {
        row: Some(row),
        writes: 0,
    };
    let budget = Arc::new(Budget::default());
    // Only the real authority load and unchanged parser execute under tracking;
    // input/fence/row, recorder Arc and instrumentation are prepared beforehand.
    let measurement = Measurement::begin();
    let result = with_allocation_budget(budget.clone(), || authority::run(&mut sql, request));
    drop(measurement);
    let (peak, traffic, live, overflow) = TRACKING.with(|s| {
        let s = s.borrow();
        (s.peak, s.traffic, s.live, s.overflow)
    });
    assert!(!overflow, "tracker exhausted fixed slots");
    assert_eq!(live, 0, "load/parser owners must drop before result");
    assert_eq!(budget.calls.load(Ordering::Relaxed), expected_calls);
    assert_eq!(
        budget.reserved.load(Ordering::Relaxed),
        expected_reservation
    );
    assert!(peak <= expected_reservation, "requested peak {peak} exceeds held {expected_reservation}; cumulative traffic {traffic} is a separate metric");
    assert_eq!(result.is_ok(), success);
    if !success {
        assert_eq!(result.unwrap_err(), Error::Storage);
    }
    assert_eq!(sql.writes, usize::from(success));
}
#[test]
fn preserved_state_serde_requested_peak_fits_preheld_envelope() {
    let id = serde_json::to_string(&identity()).unwrap();
    let active = serde_json::to_string(&activation()).unwrap();
    run_case(id.clone(), active.clone());
    let mut large = identity();
    large.host = vec![0; 1800];
    let large_id = serde_json::to_string(&large).unwrap();
    run_case(large_id.clone(), active.clone());
    run_case(
        format!("{},\"host\":[]}}", &large_id[..large_id.len() - 1]),
        active.clone(),
    );
    for text in [
        format!("{{\"{}\":0}}", "x".repeat(4000)),
        format!("\"{}\"", "\\u0001".repeat(600)),
        format!("{{\"installation\":\"{}\"}}", "🦀".repeat(900)),
        "[".repeat(4000),
        format!("{{\"host\":[{}]}}", "9".repeat(4000)),
    ] {
        run_case(text, active.clone());
    }
    for text in [
        format!("{{\"{}\":0}}", "x".repeat(4000)),
        active.replace(
            "\"version\":\"1\"",
            &format!("\"version\":\"{}\"", "x".repeat(2000)),
        ),
        active.replace("\"version\":\"1\"", "\"version\":\"-9223372036854775808\""),
        active.replace("\"version\":\"1\"", "\"version\":\"\\u0031\""),
        active.replace("\"dirent_bytes\":4096", "\"dirent_bytes\":1e0"),
        active.replace("\"dirent_bytes\":4096", "\"dirent_bytes\":-0"),
        active.replace(
            "\"dirent_bytes\":4096",
            "\"dirent_bytes\":18446744073709551616",
        ),
        active.replace("\"version\":\"1\"", "\"version\":null"),
    ] {
        run_case(large_id.clone(), text);
    }
}

#[test]
fn pinned_format_growth_and_boxed_string_overlap_fit_message_term() {
    for text in [
        String::new(),
        "x".repeat(4000),
        "\u{1}".repeat(4000),
        "🦀".repeat(1000),
    ] {
        let message_bound = 10 * text.len() + 512;
        for debug in [false, true] {
            let measurement = Measurement::begin();
            let message = if debug {
                format_args!("invalid type: string {:?}, expected a sequence", text).to_string()
            } else {
                format_args!(
                    "unknown field `{}`, expected one of `installation`, `tenant`, `host`, `cell`",
                    text
                )
                .to_string()
            };
            let length = message.len();
            let boxed = message.into_boxed_str();
            drop(boxed);
            drop(measurement);
            let (peak, live, overflow) = TRACKING.with(|s| {
                let s = s.borrow();
                (s.peak, s.live, s.overflow)
            });
            assert!(!overflow);
            assert_eq!(live, 0);
            assert!(length <= message_bound);
            assert!(peak <= 3 * message_bound + 256);
        }
    }
}
