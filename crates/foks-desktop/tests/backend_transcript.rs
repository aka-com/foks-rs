#![cfg(unix)]

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use foks_agent_proto::{
    ErrorCode, KvPrecondition, KvReadResult, KvRole, KvStoreRef, KvUploadPayload, Operation,
    Request, Response,
};

const TEAM_ID: &str = "140000000000000000000000000000000000000000000000000000000000000000";

fn backend() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_foks-desktop-backend"))
}

fn private_file(path: &Path, content: &[u8]) {
    std::fs::write(path, content).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn private_socket(directory: &Path) -> (PathBuf, UnixListener) {
    let socket = directory.join("agent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    (socket, listener)
}

fn run_backend(socket: &Path, arguments: &[OsString]) -> Output {
    let mut child = Command::new(backend())
        .arg("--agent-socket")
        .arg(socket)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("backend transcript exceeded its 15 second deadline");
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    Output {
        status,
        stdout,
        stderr,
    }
}

fn read_frame(stream: &mut UnixStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).unwrap();
    let length = u32::from_be_bytes(prefix) as usize;
    let mut frame = vec![0; 4 + length];
    frame[..4].copy_from_slice(&prefix);
    stream.read_exact(&mut frame[4..]).unwrap();
    frame
}

fn accept_bounded(listener: &UnixListener) -> UnixStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "backend did not connect within 15 seconds"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    }
}

#[derive(Clone)]
enum Reply {
    Success(serde_json::Value),
    Conflict,
}

fn response(request: &Request, reply: &Reply) -> Response {
    match reply {
        Reply::Success(value) => Response::success(request.id, value.clone()),
        Reply::Conflict => Response::error(request.id, ErrorCode::Conflict, "stale version"),
    }
}

fn capture_operations(arguments: Vec<OsString>, reply: Reply) -> (Output, Vec<Request>) {
    let directory = tempfile::tempdir().unwrap();
    let (socket, listener) = private_socket(directory.path());
    let child_finished = Arc::new(AtomicBool::new(false));
    let server_finished = Arc::clone(&child_finished);
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        let mut stream = accept_bounded(&listener);
        let request = foks_agent_proto::decode_request(&read_frame(&mut stream)).unwrap();
        stream
            .write_all(&foks_agent_proto::encode(&response(&request, &reply)).unwrap())
            .unwrap();
        requests.push(request);

        // Keep the rendezvous alive briefly after the response so an automatic
        // retry would be accepted and become part of the transcript.
        while !server_finished.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request =
                        foks_agent_proto::decode_request(&read_frame(&mut stream)).unwrap();
                    stream
                        .write_all(&foks_agent_proto::encode(&response(&request, &reply)).unwrap())
                        .unwrap();
                    requests.push(request);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
        let deadline = Instant::now() + Duration::from_millis(20);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request =
                        foks_agent_proto::decode_request(&read_frame(&mut stream)).unwrap();
                    stream
                        .write_all(&foks_agent_proto::encode(&response(&request, &reply)).unwrap())
                        .unwrap();
                    requests.push(request);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
        requests
    });
    let output = run_backend(&socket, &arguments);
    child_finished.store(true, Ordering::Release);
    let requests = server.join().unwrap();
    (output, requests)
}

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn show_issues_exactly_one_version_bound_team_read() {
    let store = KvStoreRef::Team(foks_agent_proto::TeamStoreRef {
        profile: "work".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "household".to_owned(),
        team_id: TEAM_ID.to_owned(),
    });
    let (output, requests) = capture_operations(
        strings(&[
            "kv-read",
            "work",
            "personal",
            "/wifi/password",
            "19",
            "--team-alias",
            "household",
            "--team-id",
            TEAM_ID,
        ]),
        Reply::Success(
            serde_json::to_value(KvReadResult {
                store: store.clone(),
                path: "/wifi/password".to_owned(),
                version: 19,
                node_type: "small-file".to_owned(),
                size: Some(15),
                read_role: KvRole::Member { visibility: 0 },
                write_role: KvRole::Admin,
                content: Some(b"sunny-kettle-42".to_vec()),
                symlink_target: None,
            })
            .unwrap(),
        ),
    );
    assert_success(&output);
    assert_eq!(requests.len(), 1, "Show must issue one agent request");
    assert_eq!(
        requests[0].operation,
        Operation::ReadKv {
            store,
            path: "/wifi/password".to_owned(),
            version: 19,
        }
    );
}

#[test]
fn text_link_and_remove_transcripts_name_create_and_exact_version_guards() {
    let values = tempfile::tempdir().unwrap();
    let value = values.path().join("value");
    let target = values.path().join("target");
    private_file(&value, b"correct horse battery staple");
    private_file(&target, b"/production/current");

    let mut create = strings(&[
        "kv-create-text",
        "work",
        "personal",
        "/password",
        "--value-file",
    ]);
    create.push(value.clone().into_os_string());
    let (output, requests) =
        capture_operations(create, Reply::Success(serde_json::json!({ "version": 1 })));
    assert_success(&output);
    assert!(matches!(
        requests.as_slice(),
        [Request {
            operation: Operation::PutKv {
                store: KvStoreRef::Account(_),
                path,
                content,
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                precondition: KvPrecondition::Create,
            },
            ..
        }] if path == "/password" && content == b"correct horse battery staple"
    ));

    let mut edit = strings(&[
        "kv-edit-text",
        "work",
        "personal",
        "/password",
        "7",
        "--read-role",
        "member:-1",
        "--write-role",
        "admin",
        "--value-file",
    ]);
    edit.push(value.clone().into_os_string());
    let (output, requests) =
        capture_operations(edit, Reply::Success(serde_json::json!({ "version": 8 })));
    assert_success(&output);
    assert!(matches!(
        requests.as_slice(),
        [Request {
            operation: Operation::PutKv {
                read_role: KvRole::Member { visibility: -1 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactVersion { version: 7 },
                ..
            },
            ..
        }]
    ));

    let mut link = strings(&[
        "kv-create-link",
        "work",
        "personal",
        "/current",
        "--target-file",
    ]);
    link.push(target.into_os_string());
    let (output, requests) =
        capture_operations(link, Reply::Success(serde_json::json!({ "version": 1 })));
    assert_success(&output);
    assert!(matches!(
        requests.as_slice(),
        [Request {
            operation: Operation::PutKvSymlink {
                target,
                precondition: KvPrecondition::Create,
                ..
            },
            ..
        }] if target == "/production/current"
    ));

    let (output, requests) = capture_operations(
        strings(&["kv-remove", "work", "personal", "/password", "8"]),
        Reply::Success(serde_json::json!({ "removed": true })),
    );
    assert_success(&output);
    assert!(matches!(
        requests.as_slice(),
        [Request {
            operation: Operation::RemoveKv {
                recursive: false,
                precondition: KvPrecondition::ExactVersion { version: 8 },
                ..
            },
            ..
        }]
    ));
}

#[test]
fn forced_conflict_is_one_attempt_and_never_overwrites() {
    let values = tempfile::tempdir().unwrap();
    let value = values.path().join("value");
    private_file(&value, b"draft");
    let mut arguments = strings(&[
        "kv-edit-text",
        "work",
        "personal",
        "/password",
        "7",
        "--read-role",
        "owner",
        "--write-role",
        "owner",
        "--value-file",
    ]);
    arguments.push(value.into_os_string());
    let (output, requests) = capture_operations(arguments, Reply::Conflict);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Conflict"));
    assert_eq!(requests.len(), 1, "a conflict must not be retried");
    assert!(matches!(
        requests[0].operation,
        Operation::PutKv {
            precondition: KvPrecondition::ExactVersion { version: 7 },
            ..
        }
    ));
}

struct UploadTranscript {
    request: Request,
    total: u64,
    chunks: usize,
    maximum_chunk: usize,
    maximum_frame: usize,
}

fn capture_upload(arguments: Vec<OsString>) -> (Output, UploadTranscript) {
    let directory = tempfile::tempdir().unwrap();
    let (socket, listener) = private_socket(directory.path());
    let server = std::thread::spawn(move || {
        let mut stream = accept_bounded(&listener);
        let request = foks_agent_proto::decode_request(&read_frame(&mut stream)).unwrap();
        let mut total = 0u64;
        let mut chunks = 0usize;
        let mut maximum_chunk = 0usize;
        let mut maximum_frame = 0usize;
        loop {
            let encoded = read_frame(&mut stream);
            maximum_frame = maximum_frame.max(encoded.len());
            let frame = foks_agent_proto::decode_upload_frame(&encoded).unwrap();
            assert_eq!(frame.id, request.id);
            match frame.payload {
                KvUploadPayload::Chunk { offset, content } => {
                    assert_eq!(offset, total);
                    total += content.len() as u64;
                    chunks += 1;
                    maximum_chunk = maximum_chunk.max(content.len());
                }
                KvUploadPayload::Commit => break,
            }
        }
        stream
            .write_all(
                &foks_agent_proto::encode(&Response::success(
                    request.id,
                    serde_json::json!({ "version": 1 }),
                ))
                .unwrap(),
            )
            .unwrap();
        UploadTranscript {
            request,
            total,
            chunks,
            maximum_chunk,
            maximum_frame,
        }
    });
    let output = run_backend(&socket, &arguments);
    let transcript = server.join().unwrap();
    (output, transcript)
}

#[test]
fn eighty_four_megabyte_file_uses_bounded_stream_frames_without_inline_content() {
    let values = tempfile::tempdir().unwrap();
    let source = values.path().join("bundle.tar");
    let file = std::fs::File::create(&source).unwrap();
    file.set_len(84 * 1024 * 1024).unwrap();
    drop(file);
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();

    let mut arguments = strings(&[
        "kv-create-file",
        "work",
        "personal",
        "/bundle.tar",
        "--source-file",
    ]);
    arguments.push(source.into_os_string());
    let (output, transcript) = capture_upload(arguments);
    assert_success(&output);
    assert!(matches!(
        transcript.request.operation,
        Operation::PutKvStream {
            header: foks_agent_proto::KvUploadHeader {
                total_length: 88_080_384,
                precondition: KvPrecondition::Create,
                ..
            }
        }
    ));
    assert_eq!(transcript.total, 84 * 1024 * 1024);
    assert!(transcript.chunks > 1);
    assert!(transcript.maximum_chunk <= 128 * 1024);
    assert!(transcript.maximum_frame <= foks_agent_proto::MAXIMUM_MESSAGE_BYTES);
}

#[test]
fn replacement_file_stream_is_bound_to_the_inspected_version_and_roles() {
    let values = tempfile::tempdir().unwrap();
    let source = values.path().join("replacement");
    private_file(&source, b"replacement file");
    let mut arguments = strings(&[
        "kv-replace-file",
        "work",
        "personal",
        "/bundle.tar",
        "11",
        "--read-role",
        "member:-16384",
        "--write-role",
        "admin",
        "--source-file",
    ]);
    arguments.push(source.into_os_string());
    let (output, transcript) = capture_upload(arguments);
    assert_success(&output);
    assert!(matches!(
        transcript.request.operation,
        Operation::PutKvStream {
            header: foks_agent_proto::KvUploadHeader {
                total_length: 16,
                read_role: KvRole::Member { visibility: -16384 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactVersion { version: 11 },
                ..
            }
        }
    ));
    assert_eq!(transcript.total, 16);
    assert_eq!(transcript.chunks, 1);
}
