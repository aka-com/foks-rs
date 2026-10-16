//! Synchronous, bounded native client for the local FOKS agent protocol.

#![forbid(unsafe_code)]

pub use foks_local_deadline::Deadline;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use foks_agent_proto::{
    KvUploadFrame, KvUploadHeader, KvUploadPayload, Operation, Request, Response,
    MAXIMUM_MESSAGE_BYTES, PROTOCOL_VERSION,
};
use thiserror::Error;
use zeroize::{Zeroize as _, Zeroizing};

pub mod secret_file;
#[cfg(unix)]
mod transport;

/// One upload frame's plaintext. Byte payloads are base64, so this is the
/// protocol's own payload bound, which is sized against the frame ceiling
/// with the envelope reserve held back.
const MAXIMUM_UPLOAD_FRAME_BYTES: usize = foks_agent_proto::MAXIMUM_KV_PAYLOAD_BYTES;

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
    #[error("{0}")]
    LocalDeadline(#[source] foks_local_deadline::Error),
    #[error("local agent request deadline exceeded")]
    DeadlineExceeded,
    #[error("local agent mutation outcome is ambiguous: {0}")]
    Ambiguous(Box<Error>),
}

impl From<foks_local_deadline::Error> for Error {
    fn from(error: foks_local_deadline::Error) -> Self {
        match error {
            foks_local_deadline::Error::Expired => Self::DeadlineExceeded,
            error => Self::LocalDeadline(error),
        }
    }
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

#[derive(Clone)]
pub struct AgentClient {
    socket: PathBuf,
    timeout: Option<Duration>,
    deadline: Option<Deadline>,
    next_id: Arc<AtomicU64>,
}
impl AgentClient {
    pub fn new(socket: impl AsRef<Path>) -> Self {
        Self {
            socket: socket.as_ref().to_path_buf(),
            timeout: None,
            deadline: None,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }
    /// Explicit ceiling; it never enlarges the operation's classified budget.
    pub fn set_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout.is_zero() || timeout > Duration::from_secs(300) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agent timeout is outside supported bounds",
            )));
        }
        self.timeout = Some(timeout);
        Ok(())
    }
    pub fn with_operation_deadline(&self, original: Instant) -> Result<Self> {
        let deadline = match &self.deadline {
            Some(existing) => existing.shorten(original)?,
            None => Deadline::capture(original)?,
        };
        self.with_local_deadline(deadline)
    }
    pub fn with_local_deadline(&self, deadline: Deadline) -> Result<Self> {
        deadline.check()?;
        let mut client = self.clone();
        client.deadline = Some(match &self.deadline {
            Some(existing) => existing.intersect(&deadline)?,
            None => deadline,
        });
        Ok(client)
    }
    fn deadline_for(&self, class: foks_agent_proto::DeadlineClass) -> Result<Deadline> {
        // Capture BEFORE clock acquisition, encoding, source reads or admission.
        let cap = self.timeout.unwrap_or(class.cap()).min(class.cap());
        let original = Instant::now()
            .checked_add(cap)
            .ok_or(foks_local_deadline::Error::Range)?;
        Ok(match &self.deadline {
            Some(existing) => existing.shorten(original)?,
            None => Deadline::capture(original)?,
        })
    }
    pub fn call(&self, operation: Operation) -> Result<Response> {
        self.call_cancellable(operation, &|| false)
    }
    pub fn call_cancellable(
        &self,
        mut operation: Operation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Response> {
        let deadline = match self.deadline_for(operation.deadline_class()) {
            Ok(d) => d,
            Err(error) => {
                operation.zeroize_plaintext();
                return Err(error);
            }
        };
        self.call_with_context(operation, cancelled, &deadline)
    }
    fn call_with_context(
        &self,
        operation: Operation,
        cancelled: &dyn Fn() -> bool,
        deadline: &Deadline,
    ) -> Result<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mutation = operation.is_mutation();
        let mut response = call_platform(
            &self.socket,
            deadline,
            Request::new(id, operation, deadline.envelope().clone()),
            mutation,
            cancelled,
        )?;
        if response.id != Some(id) {
            response.zeroize_plaintext();
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
    /// A source Read is not preemptible. Check both sides, retain ownership and
    /// never reread the source after an ambiguous operation.
    pub fn put_kv_stream_cancellable<R: std::io::Read + ?Sized>(
        &self,
        header: KvUploadHeader,
        reader: &mut R,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Response> {
        let deadline = self.deadline_for(foks_agent_proto::DeadlineClass::Bulk)?;
        if header.adapter.is_none() {
            if let Some(upload_id) = header.upload_id {
                let phase = deadline
                    .shorten(Instant::now() + foks_agent_proto::DeadlineClass::Ordinary.cap())?;
                let prepared = self.call_with_context(
                    Operation::PrepareKvUpload {
                        store: header.store.clone(),
                        upload_id,
                        intent: (&header).into(),
                    },
                    cancelled,
                    &phase,
                )?;
                if matches!(
                    &prepared.result,
                    foks_agent_proto::ResponseResult::Error { .. }
                ) {
                    return Ok(prepared);
                }
                let expected_upload = upload_id.to_string();
                match &prepared.result {
                    foks_agent_proto::ResponseResult::Success { value }
                        if value.get("state").and_then(|v| v.as_str())
                            == Some("awaiting-source")
                            && value.get("upload_id").and_then(|v| v.as_str())
                                == Some(expected_upload.as_str()) => {}
                    _ => return Err(Error::ResponseBinding),
                }
            }
        }
        check_upload_cancelled(cancelled, &deadline)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut response = upload_platform(&self.socket, &deadline, id, header, reader, cancelled)?;
        if response.id != Some(id) {
            response.zeroize_plaintext();
            return Err(Error::Ambiguous(Box::new(Error::ResponseBinding)));
        }
        Ok(response)
    }
}

#[cfg(unix)]
fn call_platform(
    socket: &Path,
    deadline: &Deadline,
    mut request: Request,
    mutation: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<Response> {
    let frame = foks_agent_proto::encode(&request);
    request.operation.zeroize_plaintext();
    let frame = Zeroizing::new(frame?);
    check_upload_cancelled(cancelled, deadline)?;
    let mut stream = connect_platform(socket, cancelled, deadline)?;
    check_upload_cancelled(cancelled, deadline)?;
    if let Err(error) = transport::write_all(&mut stream, &frame, cancelled, deadline) {
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
    deadline: &Deadline,
    id: u64,
    header: KvUploadHeader,
    reader: &mut R,
    cancelled: &dyn Fn() -> bool,
) -> Result<Response> {
    check_upload_cancelled(cancelled, deadline)?;
    let total = header.total_length;
    let handshake = header.upload_id.is_some() && header.adapter.is_none();
    let mut stream = connect_platform(socket, cancelled, deadline)?;
    check_upload_cancelled(cancelled, deadline)?;
    let result = (|| {
        transport::write_all(
            &mut stream,
            &foks_agent_proto::encode(&Request::new(
                id,
                Operation::PutKvStream { header },
                deadline.envelope().clone(),
            ))?,
            cancelled,
            deadline,
        )?;
        if handshake {
            let ready = read_response_cancellable(&mut stream, cancelled, deadline)?;
            if ready.id != Some(id) {
                return Err(Error::ResponseBinding);
            }
            match &ready.result {
                foks_agent_proto::ResponseResult::Error { .. } => return Ok(ready),
                foks_agent_proto::ResponseResult::Success { value }
                    if value.get("upload_ready").and_then(|v| v.as_bool()) == Some(true) => {}
                _ => return Err(Error::ResponseBinding),
            }
        }
        let mut buffer = Zeroizing::new(vec![0u8; MAXIMUM_UPLOAD_FRAME_BYTES]);
        let mut offset = 0u64;
        while offset < total {
            check_upload_cancelled(cancelled, deadline)?;
            let wanted = usize::try_from((total - offset).min(MAXIMUM_UPLOAD_FRAME_BYTES as u64))
                .map_err(|_| Error::Io(std::io::Error::other("upload length overflow")))?;
            read_upload_chunk_cancellable(reader, &mut buffer[..wanted], cancelled, deadline)?;
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
            let encoded = Zeroizing::new(encoded?);
            if let Err(error) = transport::write_all(&mut stream, &encoded, cancelled, deadline) {
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
                    Ok(_) => Err(error),
                };
            }
            offset = offset
                .checked_add(wanted as u64)
                .ok_or_else(|| Error::Io(std::io::Error::other("upload offset overflow")))?;
        }
        let mut excess = [0u8; 1];
        check_upload_cancelled(cancelled, deadline)?;
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
        if let Err(error) = transport::write_all(&mut stream, &commit, cancelled, deadline) {
            return match read_response_cancellable(&mut stream, cancelled, deadline) {
                Ok(response) => Ok(response),
                Err(cause) => Err(Error::Ambiguous(Box::new(upload_write_failure(
                    error, cause,
                )))),
            };
        }
        read_response_cancellable(&mut stream, cancelled, deadline)
            .map_err(|error| Error::Ambiguous(Box::new(error)))
    })();
    result.map_err(|error| match error {
        Error::Cancelled | Error::DeadlineExceeded | Error::LocalDeadline(_) => {
            Error::Ambiguous(Box::new(error))
        }
        error => error,
    })
}

fn check_upload_cancelled(cancelled: &dyn Fn() -> bool, deadline: &Deadline) -> Result<()> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    deadline.check().map_err(Into::into)
}

fn upload_write_failure(write: Error, response: Error) -> Error {
    if matches!(response, Error::Cancelled | Error::DeadlineExceeded) && write.is_connection_loss()
    {
        write
    } else {
        response
    }
}

fn read_upload_chunk_cancellable<R: std::io::Read + ?Sized>(
    reader: &mut R,
    mut output: &mut [u8],
    cancelled: &dyn Fn() -> bool,
    deadline: &Deadline,
) -> Result<()> {
    while !output.is_empty() {
        check_upload_cancelled(cancelled, deadline)?;
        let count = match reader.read(output) {
            Ok(0) => {
                return Err(Error::UploadSource(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "upload ended before its declared length",
                )))
            }
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::UploadSource(error)),
        };
        check_upload_cancelled(cancelled, deadline)?;
        output = &mut output[count..];
    }
    Ok(())
}

#[cfg(unix)]
fn connect_platform(
    socket: &Path,
    cancelled: &dyn Fn() -> bool,
    deadline: &Deadline,
) -> Result<std::os::unix::net::UnixStream> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};

    check_upload_cancelled(cancelled, deadline)?;
    let metadata = std::fs::symlink_metadata(socket)?;
    if !metadata.file_type().is_socket()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(Error::UnsafeSocket);
    }
    transport::connect(socket, cancelled, deadline)
}

#[cfg(unix)]
fn read_response_cancellable(
    stream: &mut std::os::unix::net::UnixStream,
    cancelled: &dyn Fn() -> bool,
    deadline: &Deadline,
) -> Result<Response> {
    use std::io::{ErrorKind, Read as _};
    stream.set_nonblocking(true)?;
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
                    ) =>
                {
                    transport::wait(
                        Some(stream),
                        rustix::event::PollFlags::IN,
                        cancelled,
                        deadline,
                    )?
                }
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
    let mut response = foks_agent_proto::decode_response(&frame)?;
    if let Err(error) = check_upload_cancelled(cancelled, deadline) {
        response.zeroize_plaintext();
        return Err(error);
    }
    Ok(response)
}

#[cfg(not(unix))]
fn call_platform(
    _socket: &Path,
    _deadline: &Deadline,
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
    _deadline: &Deadline,
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
        listener.set_nonblocking(true).unwrap();
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("unexpected accept error: {error}"),
        }
    }

    #[test]
    fn local_deadlines_are_typed_and_distinct_from_eof() {
        let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
        let deadline = Deadline::capture(Instant::now() + Duration::from_millis(1)).unwrap();
        while Instant::now() < deadline.instant() {
            std::thread::yield_now();
        }
        let error = read_response_cancellable(&mut client, &|| false, &deadline).unwrap_err();
        assert!(matches!(error, Error::DeadlineExceeded));
        assert!(!error.is_connection_loss());
        drop(server);
        let error = read_response_cancellable(
            &mut client,
            &|| false,
            &Deadline::capture(std::time::Instant::now() + Duration::from_secs(1)).unwrap(),
        )
        .unwrap_err();
        assert!(error.is_connection_loss(), "{error:?}");
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
                &Deadline::capture(std::time::Instant::now() + Duration::from_secs(1)).unwrap(),
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
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_cancelled = cancelled.clone();
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_frame(&mut stream);
            let frame = foks_agent_proto::decode_upload_frame(&read_frame(&mut stream)).unwrap();
            assert!(matches!(frame.payload, KvUploadPayload::Commit));
            server_cancelled.store(true, Ordering::Release);
            assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
        });
        let header = KvUploadHeader {
            upload_id: None,
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
        let error = AgentClient::new(&socket)
            .put_kv_stream_cancellable(header, &mut std::io::empty(), &|| {
                cancelled.load(Ordering::Acquire)
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
        read_upload_chunk_cancellable(
            &mut reader,
            &mut output,
            &|| false,
            &Deadline::capture(std::time::Instant::now() + Duration::from_secs(5)).unwrap(),
        )
        .unwrap();
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
            upload_id: None,
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
            assert_eq!(
                frames.len(),
                1,
                "cancellation during source read must send neither chunk nor commit"
            );
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
            upload_id: None,
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
            Err(Error::Ambiguous(cause)) if matches!(*cause, Error::Cancelled)
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
            upload_id: None,
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
    #[test]
    fn durable_upload_waits_for_registration_and_consumption_before_source_reads() {
        use foks_agent_proto::{AccountStoreRef, KvPrecondition, KvRole, KvStoreRef};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_ready = ready.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = foks_agent_proto::decode_request(&read_frame(&mut stream)).unwrap();
            assert!(
                matches!(request.operation,Operation::PrepareKvUpload{upload_id,..} if upload_id==foks_agent_proto::UploadHandle::new([1;16],1,[4;16]))
            );
            stream
                .write_all(
                    &foks_agent_proto::encode(&Response::success(
                        request.id,
                        serde_json::json!({"state":"awaiting-source","upload_id":foks_agent_proto::UploadHandle::new([1;16],1,[4;16]).to_string()}),
                    ))
                    .unwrap(),
                )
                .unwrap();
            drop(stream);
            let (mut stream, _) = listener.accept().unwrap();
            let request = foks_agent_proto::decode_request(&read_frame(&mut stream)).unwrap();
            assert!(matches!(request.operation, Operation::PutKvStream { .. }));
            server_ready.store(true, Ordering::Release);
            stream
                .write_all(
                    &foks_agent_proto::encode(&Response::success(
                        request.id,
                        serde_json::json!({"upload_ready":true}),
                    ))
                    .unwrap(),
                )
                .unwrap();
            let chunk = foks_agent_proto::decode_upload_frame(&read_frame(&mut stream)).unwrap();
            assert!(matches!(
                chunk.payload,
                KvUploadPayload::Chunk { offset: 0, .. }
            ));
            let commit = foks_agent_proto::decode_upload_frame(&read_frame(&mut stream)).unwrap();
            assert!(matches!(commit.payload, KvUploadPayload::Commit));
            stream
                .write_all(
                    &foks_agent_proto::encode(&Response::success(
                        request.id,
                        serde_json::json!({"version":1}),
                    ))
                    .unwrap(),
                )
                .unwrap();
        });
        struct Source {
            ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
            sent: bool,
        }
        impl std::io::Read for Source {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                assert!(
                    self.ready.load(Ordering::Acquire),
                    "source read before durable readiness"
                );
                if self.sent {
                    return Ok(0);
                }
                out[0] = 3;
                self.sent = true;
                Ok(1)
            }
        }
        let header = KvUploadHeader {
            upload_id: Some(foks_agent_proto::UploadHandle::new([1; 16], 1, [4; 16])),
            adapter: None,
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".into(),
                account_alias: "me".into(),
            }),
            path: "/file".into(),
            total_length: 1,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            precondition: KvPrecondition::Create,
            mkdir_p: false,
        };
        AgentClient::new(&socket)
            .put_kv_stream(header, &mut Source { ready, sent: false })
            .unwrap();
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
                match stream.read(&mut [0; 1]) {
                    Ok(0) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                    result => panic!("client did not close the partial reply: {result:?}"),
                }
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

#[cfg(all(test, unix))]
mod deadline_context_tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::os::unix::{fs::PermissionsExt as _, net::UnixListener};
    #[test]
    fn successive_calls_never_extend_the_original_wire_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let mut envelopes = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut prefix = [0; 4];
                stream.read_exact(&mut prefix).unwrap();
                let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
                frame[..4].copy_from_slice(&prefix);
                stream.read_exact(&mut frame[4..]).unwrap();
                let request = foks_agent_proto::decode_request(&frame).unwrap();
                envelopes.push(request.deadline);
                stream
                    .write_all(
                        &foks_agent_proto::encode(&Response::success(
                            request.id,
                            serde_json::json!({}),
                        ))
                        .unwrap(),
                    )
                    .unwrap();
            }
            envelopes
        });
        let original = Deadline::capture(Instant::now() + Duration::from_secs(2)).unwrap();
        let client = AgentClient::new(&socket)
            .with_local_deadline(original.clone())
            .unwrap();
        client.call(Operation::Ping).unwrap();
        client
            .with_operation_deadline(Instant::now() + Duration::from_secs(60))
            .unwrap()
            .call(Operation::Ping)
            .unwrap();
        let envelopes = server.join().unwrap();
        assert_eq!(envelopes.len(), 2);
        for envelope in envelopes {
            assert_eq!(envelope.domain(), original.envelope().domain());
            // Conservative OS/Instant resampling may shorten by a few ns.
            assert!(envelope.expires_ns() <= original.envelope().expires_ns());
        }
    }
    #[test]
    fn expired_bound_client_fails_before_connect_and_keeps_base_unbound() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let base = AgentClient::new(&socket);
        let deadline = Deadline::capture(Instant::now() + Duration::from_millis(5)).unwrap();
        let client = base.with_local_deadline(deadline.clone()).unwrap();
        while Instant::now() < deadline.instant() {
            std::thread::yield_now();
        }
        assert!(matches!(
            client.call(Operation::Ping),
            Err(Error::DeadlineExceeded)
        ));
        assert!(
            !Error::LocalDeadline(foks_local_deadline::Error::DomainMismatch).is_connection_loss()
        );
        assert!(base.deadline.is_none());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
