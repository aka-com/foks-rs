//! Synchronous command stages. Missing context never creates a fresh budget.
use foks_client_app::{CancellationToken, ProfileOperationScope};
use foks_local_deadline::Deadline;
use std::{
    cell::RefCell,
    marker::PhantomData,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
struct Stage {
    deadline: Deadline,
    _scope: ProfileOperationScope,
}
#[derive(Default)]
struct State {
    command: bool,
    stage: Option<Stage>,
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }
pub(super) struct CommandScope {
    _thread: PhantomData<Rc<()>>,
}
impl CommandScope {
    pub fn enter(budget: Duration) -> Result<Self> {
        let original = Instant::now()
            .checked_add(budget)
            .ok_or("invalid command budget")?;
        Self::with_deadline(Deadline::capture(original)?)
    }
    fn with_deadline(deadline: Deadline) -> Result<Self> {
        STATE.with(|s| -> Result<()> {
            let mut state = s.borrow_mut();
            if state.command {
                return Err("command deadline context already exists".into());
            }
            let scope = ProfileOperationScope::new(deadline.clone(), CancellationToken::new())?;
            state.command = true;
            state.stage = Some(Stage {
                deadline,
                _scope: scope,
            });
            Ok(())
        })?;
        Ok(Self {
            _thread: PhantomData,
        })
    }
}
impl Drop for CommandScope {
    fn drop(&mut self) {
        STATE.with(|s| *s.borrow_mut() = State::default());
    }
}
pub(super) fn current() -> Result<Deadline> {
    let deadline = STATE
        .with(|s| s.borrow().stage.as_ref().map(|s| s.deadline.clone()))
        .ok_or("command deadline context is required")?;
    deadline.check()?;
    Ok(deadline)
}
pub(super) fn check() -> Result<()> {
    current().map(|_| ())
}
pub(super) fn agent_client(state: &Path) -> Result<foks_agent_client::AgentClient> {
    Ok(
        foks_agent_client::AgentClient::new(state.join("foks-rs.sock"))
            .with_local_deadline(current()?)?,
    )
}
/// End a completed preparation/startup phase before human wait or MCP service.
pub(super) fn pause() -> Result<()> {
    check()?;
    STATE.with(|s| {
        s.borrow_mut().stage.take();
    });
    Ok(())
}
/// Only an explicit post-confirmation stage may receive a new operation budget.
pub(super) fn confirmed_stage(budget: Duration) -> Result<()> {
    let original = Instant::now()
        .checked_add(budget)
        .ok_or("invalid command budget")?;
    let deadline = Deadline::capture(original)?;
    STATE.with(|s| -> Result<()> {
        let mut state = s.borrow_mut();
        if !state.command || state.stage.is_some() {
            return Err("invalid confirmed command stage".into());
        }
        let scope = ProfileOperationScope::new(deadline.clone(), CancellationToken::new())?;
        state.stage = Some(Stage {
            deadline,
            _scope: scope,
        });
        Ok(())
    })
}
pub(super) fn print_json(value: &impl serde::Serialize) -> Result<()> {
    let text = zeroize::Zeroizing::new(serde_json::to_string_pretty(value)?);
    check()?;
    println!("{}", text.as_str());
    Ok(())
}
/// Agent JSON can contain bearer URLs or invitation material. Keep the decoded
/// owner guarded through formatting, deadline checks, stdout errors and unwinds.
pub(super) struct AgentValue(foks_agent_proto::Response);
impl AgentValue {
    pub fn new(value: serde_json::Value) -> Self {
        Self(foks_agent_proto::Response::success(0, value))
    }
    fn erase(&mut self) {
        self.0.zeroize_plaintext();
    }
    pub fn print(self) -> Result<()> {
        print_json(&*self)
    }
}
impl std::ops::Deref for AgentValue {
    type Target = serde_json::Value;
    fn deref(&self) -> &Self::Target {
        match &self.0.result {
            foks_agent_proto::ResponseResult::Success { value } => value,
            _ => unreachable!(),
        }
    }
}
impl std::ops::DerefMut for AgentValue {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut self.0.result {
            foks_agent_proto::ResponseResult::Success { value } => value,
            _ => unreachable!(),
        }
    }
}
impl Drop for AgentValue {
    fn drop(&mut self) {
        self.erase();
    }
}
pub(super) fn print_agent_value(value: serde_json::Value) -> Result<()> {
    AgentValue::new(value).print()
}

/// Cooperative source boundary: a blocking read may overrun, but bytes read
/// after expiration are erased and cannot enter upload preparation.
pub(super) struct Reader<R> {
    inner: R,
    deadline: Deadline,
}
impl<R> Reader<R> {
    pub fn new(inner: R) -> Result<Self> {
        Ok(Self {
            inner,
            deadline: current()?,
        })
    }
}
impl<R: std::io::Read> std::io::Read for Reader<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        self.deadline
            .check()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::TimedOut, e))?;
        let count = match self.inner.read(output) {
            Ok(count) => count,
            Err(error) => {
                zeroize::Zeroize::zeroize(output);
                return Err(error);
            }
        };
        if let Err(error) = self.deadline.check() {
            zeroize::Zeroize::zeroize(&mut output[..count]);
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, error));
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_scope_fails_closed_and_unwind_clears_worker_context() {
        assert!(current().is_err());
        let _ = std::panic::catch_unwind(|| {
            let _scope = CommandScope::enter(Duration::from_secs(2)).unwrap();
            assert!(current().is_ok());
            panic!("test unwind");
        });
        assert!(current().is_err());
    }
    #[test]
    fn only_explicit_confirmed_stage_changes_original_deadline() {
        let _scope = CommandScope::enter(Duration::from_secs(2)).unwrap();
        let original = current().unwrap();
        assert_eq!(current().unwrap().envelope(), original.envelope());
        assert!(confirmed_stage(Duration::from_secs(5)).is_err());
        pause().unwrap();
        assert!(current().is_err());
        confirmed_stage(Duration::from_secs(5)).unwrap();
        assert_ne!(current().unwrap().envelope(), original.envelope());
    }
    #[test]
    fn source_bytes_are_erased_when_blocking_read_crosses_original_deadline() {
        use std::io::Read as _;
        let _scope = CommandScope::enter(Duration::from_millis(100)).unwrap();
        struct Slow(Instant);
        impl std::io::Read for Slow {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                std::thread::sleep(self.0.saturating_duration_since(Instant::now()));
                out[..6].copy_from_slice(b"secret");
                Ok(6)
            }
        }
        let mut source = Reader::new(Slow(current().unwrap().instant())).unwrap();
        let mut output = [0xff; 8];
        assert!(source.read(&mut output).is_err());
        assert_eq!(&output[..6], &[0; 6]);
    }
    #[test]
    fn decoded_agent_values_are_erased_by_the_drop_guard() {
        let mut value =
            AgentValue::new(serde_json::json!({"bearer":"private","nested":["secret"]}));
        value.erase();
        assert_eq!(&*value, &serde_json::json!({}));
        assert!(
            print_agent_value(serde_json::json!({"bearer":"private"})).is_err(),
            "missing context must refuse publication"
        );
    }
}
