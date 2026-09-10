//! Synchronous, bounded native client for the local FOKS agent protocol.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use foks_agent_proto::{
    KvUploadFrame, KvUploadHeader, KvUploadPayload, Operation, Request, Response,
    MAXIMUM_MESSAGE_BYTES, PROTOCOL_VERSION,
};
use thiserror::Error;
use zeroize::{Zeroize as _, Zeroizing};

const DEVICE_PAIRING_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAXIMUM_UPLOAD_FRAME_BYTES: usize = 128 * 1024;

#[derive(Debug, Error)]
pub enum Error {
    #[error("local agent IPC is unsupported on this platform")]
    Unsupported,
    #[error("local agent socket has insecure permissions or cannot be authenticated")]
    UnsafeSocket,
    #[error("local agent I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("local agent protocol failed: {0}")]
    Protocol(#[from] foks_agent_proto::Error),
    #[error("local agent response ID does not match request ID")]
    ResponseBinding,
    #[error("local agent mutation outcome is ambiguous: {0}")]
    Ambiguous(String),
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
        self.call_cancellable(operation, &|| false)
    }

    /// Cancellation closes the IPC socket; a started mutation remains ambiguous.
    pub fn call_cancellable(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mutation = operation.is_mutation();
        let timeout = if operation.is_device_pairing_wait() {
            self.timeout.max(DEVICE_PAIRING_TIMEOUT)
        } else {
            self.timeout
        };
        let response = call_platform(
            &self.socket,
            timeout,
            Request::new(id, operation),
            mutation,
            cancelled,
        )?;
        if response.id != Some(id) {
            return Err(if mutation {
                Error::Ambiguous(Error::ResponseBinding.to_string())
            } else {
                Error::ResponseBinding
            });
        }
        Ok(response)
    }

    pub fn put_kv_stream<R: std::io::Read + ?Sized>(
        &self,
        header: KvUploadHeader,
        reader: &mut R,
    ) -> Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = upload_platform(&self.socket, self.timeout, id, header, reader)?;
        if response.id != Some(id) {
            return Err(Error::Ambiguous(Error::ResponseBinding.to_string()));
        }
        Ok(response)
    }
}

#[cfg(unix)]
fn call_platform(
    socket: &Path,
    timeout: Duration,
    mut request: Request,
    mutation: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<Response> {
    use std::io::Write as _;
    let frame = foks_agent_proto::encode(&request);
    request.operation.zeroize_plaintext();
    let frame = frame?;
    if cancelled() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "request cancelled",
        )));
    }
    let mut stream = connect_platform(socket, timeout)?;
    let deadline = std::time::Instant::now() + timeout;
    if let Err(error) = stream.write_all(&frame) {
        return Err(if mutation {
            Error::Ambiguous(error.to_string())
        } else {
            Error::Io(error)
        });
    }
    drop(frame);
    read_response_cancellable(&mut stream, cancelled, deadline).map_err(|error| {
        if mutation {
            Error::Ambiguous(error.to_string())
        } else {
            error
        }
    })
}

#[cfg(unix)]
fn upload_platform<R: std::io::Read + ?Sized>(
    socket: &Path,
    timeout: Duration,
    id: u64,
    header: KvUploadHeader,
    reader: &mut R,
) -> Result<Response> {
    use std::io::Write as _;

    let total = header.total_length;
    let mut stream = connect_platform(socket, timeout)?;
    stream.write_all(&foks_agent_proto::encode(&Request::new(
        id,
        Operation::PutKvStream { header },
    ))?)?;
    let mut buffer = Zeroizing::new(vec![0u8; MAXIMUM_UPLOAD_FRAME_BYTES]);
    let mut offset = 0u64;
    while offset < total {
        let wanted = usize::try_from((total - offset).min(MAXIMUM_UPLOAD_FRAME_BYTES as u64))
            .map_err(|_| Error::Io(std::io::Error::other("upload length overflow")))?;
        read_upload_chunk(reader, &mut buffer[..wanted])?;
        let mut frame = KvUploadFrame {
            version: PROTOCOL_VERSION,
            id,
            payload: KvUploadPayload::Chunk {
                offset,
                content: buffer[..wanted].to_vec(),
            },
        };
        let encoded = foks_agent_proto::encode(&frame);
        if let KvUploadPayload::Chunk { content, .. } = &mut frame.payload {
            content.zeroize();
        }
        let encoded = encoded?;
        if let Err(error) = stream.write_all(&encoded) {
            return match read_response(&mut stream) {
                Ok(response)
                    if response.id == Some(id)
                        && matches!(
                            &response.result,
                            foks_agent_proto::ResponseResult::Error { .. }
                        ) =>
                {
                    Ok(response)
                }
                Err(_) => Err(Error::Io(error)),
                Ok(_) => Err(Error::Io(error)),
            };
        }
        offset = offset
            .checked_add(wanted as u64)
            .ok_or_else(|| Error::Io(std::io::Error::other("upload offset overflow")))?;
    }
    let mut excess = [0u8; 1];
    if reader.read(&mut excess)? != 0 {
        excess.zeroize();
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "upload exceeds its declared length",
        )));
    }
    let commit = foks_agent_proto::encode(&KvUploadFrame {
        version: PROTOCOL_VERSION,
        id,
        payload: KvUploadPayload::Commit,
    })?;
    if let Err(error) = stream.write_all(&commit) {
        return match read_response(&mut stream) {
            Ok(response) => Ok(response),
            Err(_) => Err(Error::Ambiguous(error.to_string())),
        };
    }
    read_response(&mut stream).map_err(|error| Error::Ambiguous(error.to_string()))
}

fn read_upload_chunk<R: std::io::Read + ?Sized>(reader: &mut R, output: &mut [u8]) -> Result<()> {
    reader.read_exact(output).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "upload ended before its declared length",
            ))
        } else {
            Error::Io(error)
        }
    })
}

#[cfg(unix)]
fn connect_platform(socket: &Path, timeout: Duration) -> Result<std::os::unix::net::UnixStream> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
    use std::os::unix::net::UnixStream;

    let metadata = std::fs::symlink_metadata(socket)?;
    if !metadata.file_type().is_socket()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(Error::UnsafeSocket);
    }
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}

#[cfg(unix)]
fn read_response_cancellable(
    stream: &mut std::os::unix::net::UnixStream,
    cancelled: &dyn Fn() -> bool,
    deadline: std::time::Instant,
) -> Result<Response> {
    use std::io::{ErrorKind, Read as _};
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut read = |mut bytes: &mut [u8]| -> std::io::Result<()> {
        while !bytes.is_empty() {
            if cancelled() {
                return Err(std::io::Error::new(
                    ErrorKind::Interrupted,
                    "request cancelled",
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    "request deadline exceeded",
                ));
            }
            match stream.read(bytes) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        ErrorKind::UnexpectedEof,
                        "agent disconnected",
                    ))
                }
                Ok(count) => bytes = &mut bytes[count..],
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    };
    let mut prefix = [0; 4];
    read(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err(foks_agent_proto::Error::TooLarge.into());
    }
    let mut frame = Zeroizing::new(vec![0; 4 + length]);
    frame[..4].copy_from_slice(&prefix);
    read(&mut frame[4..])?;
    if cancelled() {
        return Err(Error::Io(std::io::Error::new(
            ErrorKind::Interrupted,
            "request cancelled",
        )));
    }
    Ok(foks_agent_proto::decode_response(&frame)?)
}

#[cfg(unix)]
fn read_response(stream: &mut std::os::unix::net::UnixStream) -> Result<Response> {
    use std::io::Read as _;

    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err(foks_agent_proto::Error::TooLarge.into());
    }
    let mut frame = Zeroizing::new(Vec::with_capacity(4 + length));
    frame.extend_from_slice(&prefix);
    frame.resize(4 + length, 0);
    stream.read_exact(&mut frame[4..])?;
    foks_agent_proto::decode_response(&frame).map_err(Into::into)
}

#[cfg(not(unix))]
fn call_platform(
    _socket: &Path,
    _timeout: Duration,
    mut request: Request,
    _mutation: bool,
    _cancelled: &dyn Fn() -> bool,
) -> Result<Response> {
    request.operation.zeroize_plaintext();
    Err(Error::Unsupported)
}

#[cfg(not(unix))]
fn upload_platform<R: std::io::Read + ?Sized>(
    _socket: &Path,
    _timeout: Duration,
    _id: u64,
    _header: KvUploadHeader,
    _reader: &mut R,
) -> Result<Response> {
    Err(Error::Unsupported)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use foks_agent_proto::{
        AccountStoreRef, ErrorCode, KvPrecondition, KvRole, KvStoreRef, KvUploadPayload,
        ResponseResult, PROTOCOL_VERSION,
    };
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener;

    struct OneByteReader(usize);

    impl std::io::Read for OneByteReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.0 == 0 || output.is_empty() {
                return Ok(0);
            }
            output[0] = 7;
            self.0 -= 1;
            Ok(1)
        }
    }

    #[test]
    fn upload_chunks_coalesce_valid_short_reads() {
        let mut reader = OneByteReader(9_000);
        let mut output = vec![0; 9_000];
        read_upload_chunk(&mut reader, &mut output).unwrap();
        assert_eq!(reader.0, 0);
        assert!(output.iter().all(|byte| *byte == 7));
    }

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

    #[test]
    fn mutation_close_after_request_is_ambiguous() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut prefix = [0u8; 4];
            stream.read_exact(&mut prefix).unwrap();
            let length = u32::from_be_bytes(prefix) as usize;
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
        });
        let error = AgentClient::new(&socket)
            .call(Operation::RemoveProfile {
                name: "local".to_owned(),
            })
            .unwrap_err();
        assert!(matches!(error, Error::Ambiguous(_)));
        server.join().unwrap();
    }

    #[test]
    fn streamed_mutation_response_binding_failure_is_ambiguous_after_commit() {
        fn read_frame(stream: &mut std::os::unix::net::UnixStream) -> Vec<u8> {
            let mut prefix = [0u8; 4];
            stream.read_exact(&mut prefix).unwrap();
            let length = u32::from_be_bytes(prefix) as usize;
            let mut frame = vec![0; 4 + length];
            frame[..4].copy_from_slice(&prefix);
            stream.read_exact(&mut frame[4..]).unwrap();
            frame
        }

        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = foks_agent_proto::decode_request(&read_frame(&mut stream)).unwrap();
            let commit = foks_agent_proto::decode_upload_frame(&read_frame(&mut stream)).unwrap();
            assert!(matches!(commit.payload, KvUploadPayload::Commit));
            let response = Response::success(request.id + 1, serde_json::json!({ "version": 1 }));
            stream
                .write_all(&foks_agent_proto::encode(&response).unwrap())
                .unwrap();
        });
        let header = KvUploadHeader {
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            path: "/empty".to_owned(),
            total_length: 0,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            precondition: KvPrecondition::Create,
            mkdir_p: false,
        };
        let error = AgentClient::new(&socket)
            .put_kv_stream(header, &mut std::io::empty())
            .unwrap_err();
        assert!(matches!(error, Error::Ambiguous(_)));
        server.join().unwrap();
    }

    #[test]
    fn streamed_mutation_recovers_an_early_rejection_while_sending_content() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut prefix = [0u8; 4];
            stream.read_exact(&mut prefix).unwrap();
            let length = u32::from_be_bytes(prefix) as usize;
            let mut frame = vec![0; 4 + length];
            frame[..4].copy_from_slice(&prefix);
            stream.read_exact(&mut frame[4..]).unwrap();
            let request = foks_agent_proto::decode_request(&frame).unwrap();
            let response = Response::error(request.id, ErrorCode::Busy, "mutation gate is busy");
            stream
                .write_all(&foks_agent_proto::encode(&response).unwrap())
                .unwrap();
            stream.shutdown(std::net::Shutdown::Read).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).ok();
        });
        let length = 2 * 1024 * 1024;
        let header = KvUploadHeader {
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            path: "/large".to_owned(),
            total_length: length,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            precondition: KvPrecondition::Create,
            mkdir_p: false,
        };
        let response = AgentClient::new(&socket)
            .put_kv_stream(header, &mut std::io::repeat(7).take(length))
            .unwrap();
        assert!(matches!(
            response.result,
            ResponseResult::Error {
                code: ErrorCode::Busy,
                ..
            }
        ));
        release.send(()).ok();
        server.join().unwrap();
    }
}

#[cfg(all(test, unix))]
mod chat_cancellation_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::{fs::PermissionsExt, net::UnixListener},
        sync::{atomic::AtomicBool, Arc},
        time::Instant,
    };
    #[test]
    fn cancellation_closes_partial_replies_and_preserves_mutation_uncertainty() {
        for mutation in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("agent.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
            let (ready, received) = std::sync::mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut prefix = [0; 4];
                stream.read_exact(&mut prefix).unwrap();
                let mut body = vec![0; u32::from_be_bytes(prefix) as usize];
                stream.read_exact(&mut body).unwrap();
                stream.write_all(&[0, 0]).unwrap();
                ready.send(()).unwrap();
                assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
            });
            let cancel = Arc::new(AtomicBool::new(false));
            let cancelled = cancel.clone();
            let client = std::thread::spawn(move || {
                let op = if mutation {
                    Operation::SyncAccount {
                        profile: "test".into(),
                        alias: "me".into(),
                    }
                } else {
                    Operation::Ping
                };
                AgentClient::new(socket).call_cancellable(op, &|| cancelled.load(Ordering::Acquire))
            });
            received.recv_timeout(Duration::from_secs(3)).unwrap();
            let start = Instant::now();
            cancel.store(true, Ordering::Release);
            let error = client.join().unwrap().unwrap_err();
            assert_eq!(matches!(error, Error::Ambiguous(_)), mutation);
            assert!(start.elapsed() < Duration::from_secs(2));
            server.join().unwrap();
        }
    }
}
