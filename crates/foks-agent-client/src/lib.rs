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

pub mod secret_file;

const DEVICE_PAIRING_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CHAT_POLL_TIMEOUT: Duration = Duration::from_secs(60);
const MAXIMUM_UPLOAD_FRAME_BYTES: usize = 128 * 1024;

#[derive(Debug, Error)]
pub enum Error {
    #[error("local agent IPC is unsupported on this platform")]
    Unsupported,
    #[error("local agent socket has insecure permissions or cannot be authenticated")]
    UnsafeSocket,
    #[error("local agent I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("upload source failed: {0}")]
    UploadSource(std::io::Error),
    #[error("local agent protocol failed: {0}")]
    Protocol(#[from] foks_agent_proto::Error),
    #[error("local agent response ID does not match request ID")]
    ResponseBinding,
    #[error("local agent request was cancelled")]
    Cancelled,
    #[error("local agent request deadline exceeded")]
    DeadlineExceeded,
    #[error("local agent mutation outcome is ambiguous: {0}")]
    Ambiguous(Box<Error>),
}

impl Error {
    pub fn is_connection_loss(&self) -> bool {
        match self {
            Self::Io(error) => matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::UnexpectedEof
            ),
            Self::Ambiguous(cause) => cause.is_connection_loss(),
            _ => false,
        }
    }
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
        } else if operation.is_chat_poll_wait() {
            self.timeout.max(CHAT_POLL_TIMEOUT)
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
                Error::Ambiguous(Box::new(Error::ResponseBinding))
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
        self.put_kv_stream_cancellable(header, reader, &|| false)
    }

    pub fn put_kv_stream_cancellable<R: std::io::Read + ?Sized>(
        &self,
        header: KvUploadHeader,
        reader: &mut R,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = upload_platform(&self.socket, self.timeout, id, header, reader, cancelled)?;
        if response.id != Some(id) {
            return Err(Error::Ambiguous(Box::new(Error::ResponseBinding)));
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
    let deadline = std::time::Instant::now() + timeout;
    check_upload_cancelled(cancelled, deadline)?;
    let mut stream = connect_platform(socket, timeout)?;
    check_upload_cancelled(cancelled, deadline)?;
    if let Err(error) = stream.write_all(&frame) {
        let error = socket_write_error(error);
        return Err(if mutation {
            Error::Ambiguous(Box::new(error))
        } else {
            error
        });
    }
    drop(frame);
    read_response_cancellable(&mut stream, cancelled, deadline).map_err(|error| {
        if mutation {
            Error::Ambiguous(Box::new(error))
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
    cancelled: &dyn Fn() -> bool,
) -> Result<Response> {
    use std::io::Write as _;

    let deadline = std::time::Instant::now() + timeout;
    check_upload_cancelled(cancelled, deadline)?;
    let total = header.total_length;
    let mut stream = connect_platform(socket, timeout)?;
    check_upload_cancelled(cancelled, deadline)?;
    stream
        .write_all(&foks_agent_proto::encode(&Request::new(
            id,
            Operation::PutKvStream { header },
        ))?)
        .map_err(socket_write_error)?;
    let mut buffer = Zeroizing::new(vec![0u8; MAXIMUM_UPLOAD_FRAME_BYTES]);
    let mut offset = 0u64;
    while offset < total {
        check_upload_cancelled(cancelled, deadline)?;
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
            return match read_response_cancellable(&mut stream, cancelled, deadline) {
                Ok(response) if response.id != Some(id) => Err(Error::ResponseBinding),
                Ok(response)
                    if matches!(
                        &response.result,
                        foks_agent_proto::ResponseResult::Error { .. }
                    ) =>
                {
                    Ok(response)
                }
                Err(cause) => Err(upload_write_failure(error, cause)),
                Ok(_) => Err(socket_write_error(error)),
            };
        }
        offset = offset
            .checked_add(wanted as u64)
            .ok_or_else(|| Error::Io(std::io::Error::other("upload offset overflow")))?;
    }
    let mut excess = [0u8; 1];
    if reader.read(&mut excess).map_err(Error::UploadSource)? != 0 {
        excess.zeroize();
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "upload exceeds its declared length",
        )));
    }
    check_upload_cancelled(cancelled, deadline)?;
    let commit = foks_agent_proto::encode(&KvUploadFrame {
        version: PROTOCOL_VERSION,
        id,
        payload: KvUploadPayload::Commit,
    })?;
    if let Err(error) = stream.write_all(&commit) {
        return match read_response_cancellable(&mut stream, cancelled, deadline) {
            Ok(response) => Ok(response),
            Err(cause) => Err(Error::Ambiguous(Box::new(upload_write_failure(
                error, cause,
            )))),
        };
    }
    read_response_cancellable(&mut stream, cancelled, deadline)
        .map_err(|error| Error::Ambiguous(Box::new(error)))
}

fn check_upload_cancelled(
    cancelled: &dyn Fn() -> bool,
    deadline: std::time::Instant,
) -> Result<()> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    if std::time::Instant::now() >= deadline {
        return Err(Error::DeadlineExceeded);
    }
    Ok(())
}

fn socket_write_error(error: std::io::Error) -> Error {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Error::DeadlineExceeded,
        _ => Error::Io(error),
    }
}

fn upload_write_failure(write: std::io::Error, response: Error) -> Error {
    let write = socket_write_error(write);
    if matches!(response, Error::Cancelled | Error::DeadlineExceeded) && write.is_connection_loss()
    {
        write
    } else {
        response
    }
}

fn read_upload_chunk<R: std::io::Read + ?Sized>(reader: &mut R, output: &mut [u8]) -> Result<()> {
    reader.read_exact(output).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::UploadSource(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "upload ended before its declared length",
            ))
        } else {
            Error::UploadSource(error)
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
    let mut read = |mut bytes: &mut [u8]| -> Result<()> {
        while !bytes.is_empty() {
            check_upload_cancelled(cancelled, deadline)?;
            match stream.read(bytes) {
                Ok(0) => {
                    return Err(Error::Io(std::io::Error::new(
                        ErrorKind::UnexpectedEof,
                        "agent disconnected",
                    )))
                }
                Ok(count) => bytes = &mut bytes[count..],
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(Error::Io(error)),
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
    check_upload_cancelled(cancelled, deadline)?;
    Ok(foks_agent_proto::decode_response(&frame)?)
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
    _cancelled: &dyn Fn() -> bool,
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

    fn read_frame(stream: &mut std::os::unix::net::UnixStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut prefix = [0; 4];
        stream.read_exact(&mut prefix).unwrap();
        let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
        frame[..4].copy_from_slice(&prefix);
        stream.read_exact(&mut frame[4..]).unwrap();
        frame
    }

    #[test]
    fn cancellation_before_connect_or_dispatch_never_makes_a_mutation_ambiguous() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let operation = || Operation::RemoveProfile {
            name: "local".into(),
        };
        assert!(matches!(
            AgentClient::new(&socket).call_cancellable(operation(), &|| true),
            Err(Error::Cancelled)
        ));
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let calls = std::cell::Cell::new(0);
        assert!(matches!(
            AgentClient::new(&socket).call_cancellable(operation(), &|| {
                calls.set(calls.get() + 1);
                calls.get() == 2
            }),
            Err(Error::Cancelled)
        ));
        let (mut stream, _) = listener.accept().unwrap();
        assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
    }

    #[test]
    fn local_deadlines_are_typed_and_distinct_from_eof() {
        let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
        let error = read_response_cancellable(&mut client, &|| false, std::time::Instant::now())
            .unwrap_err();
        assert!(matches!(error, Error::DeadlineExceeded));
        assert!(!error.is_connection_loss());
        drop(server);
        let error = read_response_cancellable(
            &mut client,
            &|| false,
            std::time::Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.is_connection_loss());
        assert!(
            matches!(error, Error::Io(cause) if cause.kind() == std::io::ErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn cancellation_during_prefix_body_and_completed_reply_stays_typed() {
        for cancel_at in 1..=3 {
            let (mut client, mut server) = std::os::unix::net::UnixStream::pair().unwrap();
            let response = Response::success(1, serde_json::json!({"ready": true}));
            server
                .write_all(&foks_agent_proto::encode(&response).unwrap())
                .unwrap();
            let checks = std::cell::Cell::new(0);
            let error = read_response_cancellable(
                &mut client,
                &|| {
                    checks.set(checks.get() + 1);
                    checks.get() == cancel_at
                },
                std::time::Instant::now() + Duration::from_secs(1),
            )
            .unwrap_err();
            assert!(matches!(error, Error::Cancelled));
            assert!(!error.is_connection_loss());
        }
    }

    #[test]
    fn deadlines_after_dispatch_preserve_mutation_uncertainty() {
        for mutation in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("agent.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                read_frame(&mut stream);
                assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
            });
            let mut client = AgentClient::new(&socket);
            client.set_timeout(Duration::from_millis(50)).unwrap();
            let operation = if mutation {
                Operation::RemoveProfile {
                    name: "local".into(),
                }
            } else {
                Operation::Ping
            };
            let error = client.call(operation).unwrap_err();
            assert!(!error.is_connection_loss());
            if mutation {
                assert!(
                    matches!(error, Error::Ambiguous(cause) if matches!(*cause, Error::DeadlineExceeded))
                );
            } else {
                assert!(matches!(error, Error::DeadlineExceeded));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn upload_cancellation_after_commit_preserves_its_cause() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_frame(&mut stream);
            let frame = foks_agent_proto::decode_upload_frame(&read_frame(&mut stream)).unwrap();
            assert!(matches!(frame.payload, KvUploadPayload::Commit));
            assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
        });
        let header = KvUploadHeader {
            adapter: None,
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".into(),
                account_alias: "owner".into(),
            }),
            path: "/empty".into(),
            total_length: 0,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            precondition: KvPrecondition::Create,
            mkdir_p: false,
        };
        let checks = std::cell::Cell::new(0);
        let error = AgentClient::new(&socket)
            .put_kv_stream_cancellable(header, &mut std::io::empty(), &|| {
                checks.set(checks.get() + 1);
                checks.get() >= 4
            })
            .unwrap_err();
        assert!(!error.is_connection_loss());
        assert!(matches!(error, Error::Ambiguous(cause) if matches!(*cause, Error::Cancelled)));
        server.join().unwrap();
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
            adapter: None,
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
    fn cancelled_upload_closes_without_sending_commit() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut frames = Vec::new();
            loop {
                let mut prefix = [0; 4];
                match stream.read_exact(&mut prefix) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(error) => panic!("{error}"),
                }
                let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
                frame[..4].copy_from_slice(&prefix);
                stream.read_exact(&mut frame[4..]).unwrap();
                frames.push(frame);
            }
            assert_eq!(frames.len(), 2, "only header and one chunk may be sent");
            assert!(matches!(
                foks_agent_proto::decode_upload_frame(&frames[1])
                    .unwrap()
                    .payload,
                KvUploadPayload::Chunk { .. }
            ));
        });
        let cancelled = std::cell::Cell::new(false);
        struct Reader<'a>(&'a std::cell::Cell<bool>, bool);
        impl std::io::Read for Reader<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if self.1 {
                    return Ok(0);
                }
                output[0] = 42;
                self.1 = true;
                self.0.set(true);
                Ok(1)
            }
        }
        let header = KvUploadHeader {
            adapter: None,
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".into(),
                account_alias: "owner".into(),
            }),
            path: "/file".into(),
            total_length: 1,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            precondition: KvPrecondition::Create,
            mkdir_p: false,
        };
        assert!(matches!(
            AgentClient::new(&socket).put_kv_stream_cancellable(
                header,
                &mut Reader(&cancelled, false),
                &|| cancelled.get()
            ),
            Err(Error::Cancelled)
        ));
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
            adapter: None,
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
            assert!(!error.is_connection_loss());
            if mutation {
                assert!(
                    matches!(error, Error::Ambiguous(cause) if matches!(*cause, Error::Cancelled))
                );
            } else {
                assert!(matches!(error, Error::Cancelled));
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            server.join().unwrap();
        }
    }
}
