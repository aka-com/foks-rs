use std::sync::Mutex;

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionFaultPoint {
    BeforeDurableMutation,
    AfterDurableCommitBeforeResponse,
    DuringResponseWrite,
    BetweenLargeFileChunks,
}

#[derive(Clone, Copy)]
struct ArmedFault {
    point: SessionFaultPoint,
    protocol: &'static str,
    method: &'static str,
}

#[derive(Default)]
struct FaultState {
    armed: Option<ArmedFault>,
    hits: u64,
}

/// One-shot session fault injection owned by the isolated server testkit.
/// Production configurations leave this absent.
#[doc(hidden)]
#[derive(Default)]
pub struct SessionFaults {
    state: Mutex<FaultState>,
}

impl SessionFaults {
    #[doc(hidden)]
    pub fn arm(&self, point: SessionFaultPoint, protocol: &'static str, method: &'static str) {
        let mut state = self.state.lock().expect("session fault lock");
        state.armed = Some(ArmedFault {
            point,
            protocol,
            method,
        });
    }

    #[doc(hidden)]
    pub fn hits(&self) -> u64 {
        self.state.lock().expect("session fault lock").hits
    }

    pub(crate) fn disconnect(
        &self,
        point: SessionFaultPoint,
        protocol: &str,
        method: &str,
    ) -> bool {
        let mut state = self.state.lock().expect("session fault lock");
        let matches = state.armed.is_some_and(|armed| {
            armed.point == point && armed.protocol == protocol && armed.method == method
        });
        if matches {
            state.armed = None;
            state.hits += 1;
        }
        matches
    }
}
