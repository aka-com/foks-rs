//! Synchronous RESPONSE ownership. A trap skips Drop; the JS host must retire
//! that entire instance. Normal failures drop buffers and preserve sticky failure.
use super::codec::MAX_BYTES;
use crate::{Error, Result};
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Idle,
    Scalar,
    Domain,
    Page,
}
struct State {
    epoch: u64,
    depth: u8,
    phase: Phase,
    attempted: bool,
    buffer: Option<Vec<u8>>,
    failure: Option<Error>,
}
impl State {
    const fn new() -> Self {
        Self {
            epoch: 0,
            depth: 0,
            phase: Phase::Idle,
            attempted: false,
            buffer: None,
            failure: None,
        }
    }
    fn fail(&mut self, error: Error) {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
    }
}
thread_local! { static STATE: RefCell<State> = const { RefCell::new(State::new()) }; }

/// Pure/setup exports cannot mutate shared buffers during an SQL invocation.
pub(crate) fn idle() -> bool {
    STATE.with(|state| {
        let mut s = state.borrow_mut();
        if s.depth == 0 && s.phase == Phase::Idle {
            true
        } else {
            s.fail(Error::Storage);
            false
        }
    })
}
/// Only the established domain callback may prepare/read the nested run buffers.
pub(crate) fn input_output_allowed() -> bool {
    STATE.with(|state| {
        let mut s = state.borrow_mut();
        if (s.depth == 0 && s.phase == Phase::Idle)
            || (s.depth == 1
                && s.phase == Phase::Domain
                && !s.attempted
                && s.buffer.is_none()
                && s.failure.is_none())
        {
            true
        } else {
            s.fail(Error::Storage);
            false
        }
    })
}
pub(crate) fn clear_idle() {
    if !idle() {
        return;
    }
    STATE.with(|state| {
        use zeroize::Zeroize;
        let mut s = state.borrow_mut();
        if let Some(bytes) = &mut s.buffer {
            bytes.zeroize();
        }
        s.buffer = None;
    });
}

pub(crate) fn fail(error: Error) {
    STATE.with(|state| state.borrow_mut().fail(error));
}
pub(crate) fn check() -> Result<()> {
    STATE.with(|state| state.borrow().failure.clone().map_or(Ok(()), Err))
}
pub(crate) fn checked<T>(result: Result<T>) -> Result<T> {
    check()?;
    result
}
pub(crate) struct Invocation {
    epoch: u64,
    depth: u8,
}
impl Invocation {
    pub(crate) fn begin(allow_nested: bool) -> Result<Self> {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.depth == 0 && state.phase == Phase::Idle && state.buffer.is_none() {
                state.epoch = state.epoch.checked_add(1).ok_or(Error::Bounds)?;
                state.failure = None;
            } else if !(allow_nested
                && state.depth == 1
                && state.phase == Phase::Domain
                && !state.attempted
                && state.buffer.is_none()
                && state.failure.is_none())
            {
                state.fail(Error::Storage);
                return Err(Error::Storage);
            }
            state.depth += 1;
            Ok(Self {
                epoch: state.epoch,
                depth: state.depth,
            })
        })
    }
}
impl Drop for Invocation {
    fn drop(&mut self) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.epoch != self.epoch || state.depth != self.depth {
                state.fail(Error::Storage);
                return;
            }
            state.depth -= 1;
            if state.depth == 0 {
                if state.phase != Phase::Idle || state.buffer.is_some() {
                    state.fail(Error::Storage);
                }
                state.buffer = None;
                state.phase = Phase::Idle;
                state.attempted = false;
            }
        });
    }
}

pub(crate) struct Scope {
    epoch: u64,
    depth: u8,
    phase: Phase,
    previous: Phase,
}
impl Scope {
    pub(crate) fn begin(phase: Phase) -> Result<Self> {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let valid = phase != Phase::Idle
                && state.depth > 0
                && state.buffer.is_none()
                && !state.attempted
                && state.failure.is_none()
                && if phase == Phase::Page {
                    (state.depth == 1 && state.phase == Phase::Idle)
                        || (state.depth == 2 && state.phase == Phase::Domain)
                } else {
                    state.phase == Phase::Idle
                        || (state.phase == Phase::Domain
                            && state.depth == 2
                            && phase == Phase::Scalar)
                };
            if !valid {
                state.fail(Error::Storage);
                return Err(Error::Storage);
            }
            let previous = state.phase;
            state.phase = phase;
            Ok(Self {
                epoch: state.epoch,
                depth: state.depth,
                phase,
                previous,
            })
        })
    }
    pub(crate) fn finish(self, failed: bool) -> Result<Vec<u8>> {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if failed {
                state.fail(Error::Storage);
            }
            if state.epoch != self.epoch
                || state.depth != self.depth
                || state.phase != self.phase
                || !state.attempted
                || state.buffer.is_none()
            {
                state.fail(Error::Storage);
            }
            if let Some(error) = &state.failure {
                return Err(error.clone());
            }
            state.buffer.take().ok_or(Error::Storage)
        })
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.epoch != self.epoch || state.depth != self.depth || state.phase != self.phase {
                state.fail(Error::Storage);
                return;
            }
            state.buffer = None;
            state.phase = self.previous;
            state.attempted = false;
            // Never restore an enclosing healthy failure state after nested work.
        });
    }
}

/// Returns a live initialized range for this import only. No borrow survives the
/// admission callback; the epoch/phase are checked again before installing it.
pub(crate) fn allocate(length: usize) -> Option<*mut u8> {
    allocate_kind(DeliveryKind::Scalar, length)
}
pub(crate) fn allocate_page(length: usize) -> Option<*mut u8> {
    allocate_kind(DeliveryKind::Page, length)
}
#[derive(Clone, Copy)]
enum DeliveryKind {
    Scalar,
    Page,
}
impl DeliveryKind {
    fn accepts_phase(self, phase: Phase) -> bool {
        match self {
            Self::Scalar => matches!(phase, Phase::Scalar | Phase::Domain),
            Self::Page => phase == Phase::Page,
        }
    }
    fn accepts_length(self, length: usize) -> bool {
        match self {
            Self::Scalar => length <= MAX_BYTES,
            Self::Page => {
                (crate::page::HEADER_BYTES..=crate::page::MAX_PAGE_BYTES).contains(&length)
            }
        }
    }
}
fn allocate_kind(kind: DeliveryKind, length: usize) -> Option<*mut u8> {
    allocate_kind_with(
        kind,
        length,
        |bytes, length| bytes.try_reserve_exact(length).map_err(|_| ()),
        || {
            #[cfg(feature = "list-allocation-qualification")]
            crate::allocation_trace::allocator_failed();
        },
    )
}
fn allocate_kind_with(
    kind: DeliveryKind,
    length: usize,
    reserve: impl FnOnce(&mut Vec<u8>, usize) -> std::result::Result<(), ()>,
    allocation_failed: impl FnOnce(),
) -> Option<*mut u8> {
    let coordinate = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.depth == 0
            || !kind.accepts_phase(state.phase)
            || state.attempted
            || state.buffer.is_some()
            || state.failure.is_some()
        {
            state.fail(Error::Storage);
            return None;
        }
        state.attempted = true;
        if !kind.accepts_length(length) {
            state.fail(Error::Bounds);
            return None;
        }
        Some((state.epoch, state.depth, state.phase))
    })?;
    if length != 0 && crate::page::charge(length).is_err() {
        fail(Error::Bounds);
        return None;
    }
    let mut bytes = Vec::new();
    if reserve(&mut bytes, length).is_err() {
        allocation_failed();
        fail(Error::Bounds);
        return None;
    }
    bytes.resize(length, 0);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if coordinate != (state.epoch, state.depth, state.phase)
            || state.failure.is_some()
            || state.buffer.is_some()
            || !state.attempted
        {
            state.fail(Error::Storage);
            return None;
        }
        let pointer = bytes.as_mut_ptr();
        state.buffer = Some(bytes);
        Some(pointer)
    })
}

#[cfg(test)]
mod tests;
