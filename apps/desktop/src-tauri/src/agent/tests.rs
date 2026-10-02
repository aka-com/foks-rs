use super::maintenance::*;
use super::process::*;
use super::transport::*;
use super::*;
use foks_agent_proto::{ErrorCode, Operation, Response, ResponseResult};
use foks_desktop::{AgentError as DesktopAgentError, IpcErrorCode};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

mod maintenance;
mod process;
mod transport;

struct FakeMaintenanceProcess {
    stop_calls: AtomicUsize,
    restore_calls: AtomicUsize,
    stop_error: Option<AgentError>,
    restore_error: Option<AgentError>,
}

impl FakeMaintenanceProcess {
    fn healthy() -> Arc<Self> {
        Arc::new(Self {
            stop_calls: AtomicUsize::new(0),
            restore_calls: AtomicUsize::new(0),
            stop_error: None,
            restore_error: None,
        })
    }
}

impl MaintenanceProcess for FakeMaintenanceProcess {
    fn preflight(&self, _handle: &AgentHandle) -> Result<(), AgentError> {
        Ok(())
    }

    fn stop(&self, _handle: &AgentHandle) -> Result<(), AgentError> {
        self.stop_calls.fetch_add(1, Ordering::AcqRel);
        self.stop_error.clone().map_or(Ok(()), Err)
    }

    fn restore(&self, _handle: &AgentHandle) -> Result<Response, AgentError> {
        self.restore_calls.fetch_add(1, Ordering::AcqRel);
        match &self.restore_error {
            Some(error) => Err(error.clone()),
            None => Ok(Response::success(
                0,
                serde_json::to_value(foks_agent_proto::AgentStatus::ready()).unwrap(),
            )),
        }
    }
}

fn read_socket_request(stream: &mut std::os::unix::net::UnixStream) -> foks_agent_proto::Request {
    use std::io::Read as _;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).unwrap();
    let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
    frame[..4].copy_from_slice(&prefix);
    stream.read_exact(&mut frame[4..]).unwrap();
    foks_agent_proto::decode_request(&frame).unwrap()
}
