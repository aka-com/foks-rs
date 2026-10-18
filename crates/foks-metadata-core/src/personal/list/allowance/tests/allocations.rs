//! Test-only requested-live tracker; no global allocator override in non-test builds.
use super::*;
use std::{
    alloc::{GlobalAlloc, Layout as AllocLayout, System},
    cell::RefCell,
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
    slots: [Slot; 256],
    live: usize,
    peak: usize,
    traffic: usize,
    overflow: bool,
}
impl Tracking {
    const fn new() -> Self {
        Self {
            enabled: false,
            slots: [EMPTY; 256],
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
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        let result = unsafe { System.alloc(layout) };
        let _ = TRACKING.try_with(|s| s.borrow_mut().allocated(result, layout.size()));
        result
    }
    unsafe fn alloc_zeroed(&self, layout: AllocLayout) -> *mut u8 {
        let result = unsafe { System.alloc_zeroed(layout) };
        let _ = TRACKING.try_with(|s| s.borrow_mut().allocated(result, layout.size()));
        result
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: AllocLayout) {
        let _ = TRACKING.try_with(|s| s.borrow_mut().freed(pointer));
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: AllocLayout, bytes: usize) -> *mut u8 {
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

fn measure(input: &[u8], small: bool) {
    let parent = Arc::new(Parent::default());
    let budget = Budget::new(parent.clone()).unwrap();
    let measurement = Measurement::begin();
    let result = budget.run(|| {
        let result = if small {
            foks_proto::KvSmallFileBox::decode(input).map(drop)
        } else {
            foks_proto::KvDirent::decode(input).map(drop)
        };
        result.map_err(|error| PageFault::from(super::super::super::decode_error(error)))?;
        Ok(progress())
    });
    drop(measurement);
    let (peak, live, traffic, overflow) = TRACKING.with(|s| {
        let s = s.borrow();
        (s.peak, s.live, s.traffic, s.overflow)
    });
    assert!(peak <= traffic);
    assert!(!overflow);
    assert_eq!(live, 0);
    assert!(
        peak <= budget.terms.workspace_bytes,
        "peak {peak} exceeds page allowance"
    );
    assert!(result.is_err());
    if matches!(result, Err(PageFault::Corrupt)) {
        assert!(!parent.failed());
    } else {
        assert!(parent.failed());
    }
    assert!(budget.state.failed.load(Ordering::SeqCst));
    assert!(!budget.state.occupied.load(Ordering::SeqCst));
    assert_eq!(
        parent.held.load(Ordering::SeqCst),
        budget.terms.workspace_bytes + budget.terms.control_bytes
    );
    assert_eq!(parent.reservations.load(Ordering::SeqCst), 1);
}
#[test]
fn malformed_decoder_requested_peak_and_error_cleanup_fit_held_workspace() {
    // Inputs and owner/control allocation are outside measured page work.
    let mut deep = Vec::new();
    for _ in 0..65 {
        deep.extend([0x81, 0xbf]);
        deep.extend([b'x'; 31]);
    }
    deep.push(0xca);
    measure(&deep, true);
    measure(&[0xc6, 0xff, 0xff, 0xff, 0xff], true);
    let mut scalars = vec![0xdc];
    scalars.extend(60000u16.to_be_bytes());
    scalars.extend(std::iter::repeat_n(0xc0, 60000));
    measure(&scalars, true);
    use foks_snowpack::Value as V;
    let wrong_tag = V::Array(vec![
        V::Array(vec![
            V::Array(vec![
                V::Unsigned(1),
                V::Variant(Some((vec![b'x'; 31], Box::new(V::Unsigned(1))))),
            ]),
            V::Unsigned(1),
        ]),
        V::Binary(vec![]),
    ]);
    let wrong_tag = foks_snowpack::encode(&wrong_tag).unwrap();
    measure(&wrong_tag, true);
    measure(&wrong_tag, false);
}
#[test]
fn unhooked_path_headroom_covers_requested_reallocation_and_error_clone_peak() {
    // A dedicated path-only measurement isolates unhooked live storage from AST
    // credits. This is Rust1.95 Vec/clone behavior, not allocator RSS evidence.
    let tag = [b'x'; 31];
    let measurement = Measurement::begin();
    let mut path = Vec::new();
    for _ in 0..65 {
        path.push(foks_snowpack::PathSegment::Variant(tag.to_vec()));
    }
    let cloned = path.clone();
    drop(cloned);
    drop(path);
    drop(measurement);
    let (peak, live, traffic, overflow) = TRACKING.with(|s| {
        let s = s.borrow();
        (s.peak, s.live, s.traffic, s.overflow)
    });
    assert!(peak <= traffic);
    assert!(!overflow);
    assert_eq!(live, 0);
    assert!(peak <= target_terms().unwrap().auxiliary_live);
}
