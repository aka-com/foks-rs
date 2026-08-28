//! Synchronous, bounded native client for the local FOKS agent protocol.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use foks_agent_proto::{Operation, Request, Response, MAXIMUM_MESSAGE_BYTES};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("local agent IPC is unsupported on this platform")]
    Unsupported,
    #[error("local agent socket is not private and authenticatable")]
    UnsafeSocket,
    #[error("local agent I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("local agent protocol failed: {0}")]
    Protocol(#[from] foks_agent_proto::Error),
    #[error("local agent response ID changed")]
    ResponseBinding,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub struct AgentClient {
    socket: PathBuf,
    timeout: Duration,
    next_id: AtomicU64,
}

impl AgentClient {
    pub fn new(socket: impl AsRef<Path>) -> Self {
        Self {
            socket: socket.as_ref().to_path_buf(),
            timeout: Duration::from_secs(15),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout.is_zero() || timeout > Duration::from_secs(300) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agent timeout is outside supported bounds",
            )));
        }
        self.timeout = timeout;
        Ok(())
    }

    pub fn call(&self, operation: Operation) -> Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = call_platform(&self.socket, self.timeout, Request::new(id, operation))?;
        if response.id != id {
            return Err(Error::ResponseBinding);
        }
        Ok(response)
    }
}

#[cfg(unix)]
fn call_platform(socket: &Path, timeout: Duration, request: Request) -> Result<Response> {
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
    use std::os::unix::net::UnixStream;

    let metadata = std::fs::symlink_metadata(socket)?;
    if !metadata.file_type().is_socket()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(Error::UnsafeSocket);
    }
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(&foks_agent_proto::encode(&request)?)?;
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err(foks_agent_proto::Error::TooLarge.into());
    }
    let mut frame = Vec::with_capacity(4 + length);
    frame.extend_from_slice(&prefix);
    frame.resize(4 + length, 0);
    stream.read_exact(&mut frame[4..])?;
    foks_agent_proto::decode_response(&frame).map_err(Into::into)
}

#[cfg(not(unix))]
fn call_platform(_socket: &Path, _timeout: Duration, _request: Request) -> Result<Response> {
    Err(Error::Unsupported)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use foks_agent_proto::{ResponseResult, PROTOCOL_VERSION};
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener;

    #[test]
    fn binds_response_id_over_a_private_socket() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut prefix = [0u8; 4];
            stream.read_exact(&mut prefix).unwrap();
            let length = u32::from_be_bytes(prefix) as usize;
            let mut frame = vec![0; 4 + length];
            frame[..4].copy_from_slice(&prefix);
            stream.read_exact(&mut frame[4..]).unwrap();
            let request = foks_agent_proto::decode_request(&frame).unwrap();
            let response = Response::success(request.id, serde_json::json!({ "ready": true }));
            stream
                .write_all(&foks_agent_proto::encode(&response).unwrap())
                .unwrap();
        });
        let response = AgentClient::new(&socket).call(Operation::Ping).unwrap();
        assert_eq!(response.version, PROTOCOL_VERSION);
        assert!(matches!(response.result, ResponseResult::Success { .. }));
        server.join().unwrap();
    }
}
