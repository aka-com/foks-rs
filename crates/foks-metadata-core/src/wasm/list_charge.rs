//! Feature-only fixture exports. Never part of the guarded artifact allowlist.
use super::{Host, OUTPUT};
use crate::allocation_trace::{self, MAX_INPUT};
use std::cell::{Cell, RefCell};
thread_local! {
 static BUSY: Cell<bool> = const{Cell::new(false)};
 static VIOLATION: Cell<bool> = const{Cell::new(false)};
 static INPUT: RefCell<Option<Vec<u8>>> = const{RefCell::new(None)};
}
struct Guard;
impl Guard {
    fn enter() -> Option<Self> {
        if BUSY.with(|v| v.replace(true)) {
            VIOLATION.with(|v| v.set(true));
            None
        } else {
            VIOLATION.with(|v| v.set(false));
            Some(Self)
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        BUSY.with(|v| v.set(false));
    }
}
fn clear() {
    INPUT.with(|i| *i.borrow_mut() = None);
    OUTPUT.with(|o| *o.borrow_mut() = Vec::new());
}
fn fail(message: &'static [u8]) -> u32 {
    OUTPUT.with(|o| *o.borrow_mut() = message.to_vec());
    1
}
#[no_mangle]
pub extern "C" fn list_charge_input_alloc(length: u32) -> u32 {
    let Some(_guard) = Guard::enter() else {
        return 0;
    };
    clear();
    if length == 0 || length as usize > MAX_INPUT {
        return 0;
    }
    let mut input = Vec::new();
    if input.try_reserve_exact(length as usize).is_err() {
        return 0;
    }
    input.resize(length as usize, 0);
    let pointer = input.as_mut_ptr() as u32;
    INPUT.with(|i| *i.borrow_mut() = Some(input));
    pointer
}
#[no_mangle]
pub extern "C" fn list_charge_layout() -> u32 {
    let Some(_guard) = Guard::enter() else {
        return 1;
    };
    clear();
    match allocation_trace::layout_report().and_then(|r| allocation_trace::encode_report(&r)) {
        Ok(bytes) => {
            OUTPUT.with(|o| *o.borrow_mut() = bytes);
            0
        }
        Err(_) => fail(b"trace layout"),
    }
}
#[no_mangle]
pub extern "C" fn run_list_charge() -> u32 {
    let Some(_guard) = Guard::enter() else {
        return 1;
    };
    OUTPUT.with(|o| *o.borrow_mut() = Vec::new());
    let Some(input) = INPUT.with(|i| i.borrow_mut().take()) else {
        return fail(b"trace input missing");
    };
    let parsed = allocation_trace::Invocation::decode(&input);
    drop(input);
    let Ok(parsed) = parsed else {
        return fail(b"trace input invalid");
    };
    let report = allocation_trace::run_parsed(&mut Host { calls: 0 }, parsed);
    if VIOLATION.with(Cell::get) {
        return fail(b"trace phase violation");
    }
    match report.and_then(|r| allocation_trace::encode_report(&r)) {
        Ok(bytes) => {
            OUTPUT.with(|o| *o.borrow_mut() = bytes);
            0
        }
        Err(_) => fail(b"trace failed"),
    }
}
