use super::*;

#[test]
fn the_startup_gate_holds_an_ordinary_command_and_releases_it() {
    use std::sync::mpsc;

    let gate = Arc::new(StartupGate::new_open());
    gate.hold();

    let (sender, receiver) = mpsc::channel();
    let waiter = Arc::clone(&gate);
    let thread = std::thread::spawn(move || {
        waiter.wait();
        sender.send(()).expect("the receiver outlives this send");
    });

    // The command is held: nothing arrives while the gate is closed.
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    gate.release();
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("releasing the gate wakes the waiting command");
    thread.join().expect("the waiting thread finishes");

    // Once open it stays open, so a later command does not wait at all.
    gate.wait();
}

#[test]
fn ipc_mapping_uses_code_policy_without_confusing_transience_and_health() {
    use IpcErrorCode::*;
    for (code, connection_lost, ambiguous, expected, retryable, fatal) in [
        (
            DeadlineExceeded,
            false,
            false,
            "deadline-exceeded",
            true,
            false,
        ),
        (Cancelled, false, false, "cancelled", true, false),
        (Io, false, false, "io", false, false),
        (Io, true, false, "agent-lost", true, true),
        (Io, true, true, "agent-lost", false, true),
        (DeadlineExceeded, true, false, "agent-lost", true, true),
        (DeadlineExceeded, false, true, "ambiguous", false, false),
        (Protocol, true, false, "protocol", false, true),
        (
            ResponseBinding,
            false,
            true,
            "response-binding",
            false,
            true,
        ),
    ] {
        let mapped = AgentError::from_desktop(DesktopAgentError::Ipc {
            code,
            message: "original cause".into(),
            ambiguous,
            connection_lost,
        });
        assert_eq!(mapped.code, expected);
        assert_eq!(mapped.message, "original cause");
        assert_eq!(
            (mapped.retryable, mapped.fatal, mapped.ambiguous),
            (retryable, fatal, ambiguous),
            "{code:?}"
        );
    }
}

#[test]
fn upload_admission_errors_preserve_actionable_codes_without_automatic_retry() {
    for code in [
        ErrorCode::UploadExpired,
        ErrorCode::UploadIdentityConflict,
        ErrorCode::UploadGenerationMismatch,
        ErrorCode::UploadLegacyReadOnly,
        ErrorCode::UploadPendingFull,
        ErrorCode::UploadRetentionFull,
        ErrorCode::UploadSpoolFull,
        ErrorCode::UploadClockUntrusted,
        ErrorCode::UploadFuture,
    ] {
        let mapped = AgentError::from_desktop(DesktopAgentError::Protocol {
            code,
            message: "recover the existing upload before retrying".into(),
            fields: Box::default(),
        });
        assert_eq!(serde_json::to_value(code).unwrap(), mapped.code);
        assert_eq!(
            mapped.message,
            "recover the existing upload before retrying"
        );
        assert!(!mapped.retryable && !mapped.ambiguous && !mapped.fatal);
    }
}

#[test]
fn client_and_desktop_mappings_preserve_health_separately_from_ambiguity() {
    use foks_agent_client::Error;
    let errors = vec![
        Error::Unsupported,
        Error::Io(std::io::ErrorKind::InvalidInput.into()),
        Error::Io(std::io::ErrorKind::PermissionDenied.into()),
        Error::Cancelled,
        Error::DeadlineExceeded,
        Error::UnsafeSocket,
        Error::ResponseBinding,
        Error::Protocol(foks_agent_proto::Error::Version),
        Error::Protocol(foks_agent_proto::Error::TooLarge),
        Error::Io(std::io::ErrorKind::ConnectionRefused.into()),
        Error::Io(std::io::ErrorKind::UnexpectedEof.into()),
        Error::UploadSource(std::io::ErrorKind::UnexpectedEof.into()),
    ];
    for error in errors {
        let lost = error.is_connection_loss();
        let direct = AgentError::from_client(&error);
        let desktop = client_to_desktop(error);
        assert_eq!(desktop.connection_lost(), lost);
        let mapped = AgentError::from_desktop(desktop);
        assert_eq!(mapped, direct);
    }
    for cause in [
        Error::Cancelled,
        Error::DeadlineExceeded,
        Error::Io(std::io::ErrorKind::UnexpectedEof.into()),
        Error::ResponseBinding,
        Error::Protocol(foks_agent_proto::Error::Version),
    ] {
        let error = Error::Ambiguous(Box::new(cause));
        let lost = error.is_connection_loss();
        let direct = AgentError::from_client(&error);
        let desktop = client_to_desktop(error);
        assert!(desktop.ambiguous());
        assert_eq!(desktop.connection_lost(), lost);
        let mapped = AgentError::from_desktop(desktop);
        assert!(mapped.ambiguous && !mapped.retryable);
        assert_eq!(mapped, direct);
    }
}

#[test]
fn observed_cancellation_and_deadline_leave_the_next_call_healthy() {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixListener;
    for deadline in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        // The agent writes half of a length prefix and nothing more.
        let written = Arc::new(AtomicBool::new(false));
        let signal = written.clone();
        let server = std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let request = read_socket_request(&mut first);
            assert!(matches!(request.operation, Operation::Ping));
            first.write_all(&[0, 0]).unwrap();
            signal.store(true, Ordering::Release);
            // The client closed the connection with nothing left unread.
            assert_eq!(first.read(&mut [0; 1]).unwrap(), 0);
            let (mut second, _) = listener.accept().unwrap();
            let request = read_socket_request(&mut second);
            second
                .write_all(
                    &foks_agent_proto::encode(&Response::success(
                        request.id,
                        serde_json::json!({"healthy": true}),
                    ))
                    .unwrap(),
                )
                .unwrap();
        });
        let mut handle = AgentHandle::new(socket);
        Arc::get_mut(&mut handle.transport)
            .unwrap()
            .client
            .set_timeout(Duration::from_millis(500))
            .unwrap();
        // The client checks for cancellation before each read of the
        // response and before each wait for it to become readable, so of
        // three checks made after the two bytes were written, the first or
        // the second is followed by a read that takes them. Cancelling at the
        // third check observes the cancellation in the middle of the prefix.
        // Cancelling as soon as the bytes were written could close the
        // socket with them unread, which the agent reads as a reset instead
        // of an end of file.
        let checks = AtomicUsize::new(0);
        let cancelled = || {
            !deadline
                && written.load(Ordering::Acquire)
                && checks.fetch_add(1, Ordering::AcqRel) >= 2
        };
        let result = handle
            .transport()
            .call_cancellable(Operation::Ping, &cancelled);
        if deadline {
            assert!(matches!(result, Err(DesktopAgentError::DeadlineExceeded)));
        } else {
            assert!(matches!(result, Err(DesktopAgentError::Cancelled)));
        }
        assert_eq!(handle.take_connection_failure(), None);
        assert!(handle.transport().call(Operation::Ping).is_ok());
        assert_eq!(handle.take_connection_failure(), None);
        server.join().unwrap();
    }
}

#[test]
fn observed_eof_records_loss_even_when_the_mutation_is_ambiguous() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("agent.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_socket_request(&mut stream);
    });
    let handle = AgentHandle::new(socket);
    let error = handle
        .transport()
        .call(Operation::RemoveProfile {
            name: "local".into(),
        })
        .unwrap_err();
    assert!(error.ambiguous() && error.connection_lost());
    assert!(handle.take_connection_failure().is_some());
    server.join().unwrap();
}

#[test]
fn connection_loss_notifies_once_per_pending_failure() {
    let directory = tempfile::tempdir().unwrap();
    let handle = AgentHandle::new(directory.path().join("agent.sock"));
    let notifications = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&notifications);
    let flag = Arc::clone(&handle.connection_failure);
    handle.set_connection_loss_notifier(move || {
        // The app layer reads the flag back from this callback, so it must
        // not run under the flag's own lock.
        assert!(flag.try_lock().is_ok());
        counter.fetch_add(1, Ordering::Release);
    });

    let ambiguous: Result<(), DesktopAgentError> = Err(DesktopAgentError::Ambiguous(
        "the write may have applied".into(),
    ));
    handle.transport.record(&ambiguous);
    assert_eq!(notifications.load(Ordering::Acquire), 0);
    assert_eq!(handle.take_connection_failure(), None);

    let lost: Result<(), DesktopAgentError> = Err(DesktopAgentError::Transport(
        "the agent socket closed".into(),
    ));
    handle.transport.record(&lost);
    assert_eq!(notifications.load(Ordering::Acquire), 1);

    // Subsequent errors while a failure is already pending should not trigger
    // extra notifications.
    let again: Result<(), DesktopAgentError> = Err(DesktopAgentError::Transport(
        "the agent socket closed again".into(),
    ));
    handle.transport.record(&again);
    assert_eq!(notifications.load(Ordering::Acquire), 1);
    assert_eq!(
        handle.take_connection_failure().as_deref(),
        Some("the agent socket closed again")
    );

    // Once cleared, a subsequent loss represents a new state transition and
    // notifies again.
    handle.transport.record(&lost);
    assert_eq!(notifications.load(Ordering::Acquire), 2);
    assert!(handle.take_connection_failure().is_some());
}

#[test]
fn classifications_preserve_ambiguity_and_fatality() {
    let credentials = AgentError::from_agent(ErrorCode::CredentialsRequired, "locked".to_owned());
    assert_eq!(credentials.code, "credentials-required");
    assert!(!credentials.retryable && !credentials.ambiguous && !credentials.fatal);
    assert_ne!(
        credentials.code,
        AgentError::from_agent(ErrorCode::ReauthenticationRequired, "sign in".to_owned()).code
    );
    let timeout = AgentError::from_agent(ErrorCode::DeadlineExceeded, "slow".to_owned());
    assert!(timeout.retryable && timeout.ambiguous);
    assert!(AgentError::from_agent(ErrorCode::VersionMismatch, "old".to_owned()).fatal);
    let rate_limited = AgentError::from_agent(ErrorCode::RateLimited, "slow down".to_owned());
    assert_eq!(rate_limited.code, "rate-limited");
    assert!(rate_limited.retryable);
    let quota = AgentError::from_agent(ErrorCode::QuotaExceeded, "full".to_owned());
    assert_eq!(quota.code, "quota-exceeded");
    assert!(!quota.retryable);
    let ambiguous = AgentError::from_client(&foks_agent_client::Error::Ambiguous(Box::new(
        foks_agent_client::Error::Cancelled,
    )));
    assert!(ambiguous.ambiguous && !ambiguous.retryable);
    let protocol_version = AgentError::from_client(&foks_agent_client::Error::Protocol(
        foks_agent_proto::Error::Version,
    ));
    assert_eq!(protocol_version.code, "version-mismatch");
    assert!(protocol_version.fatal && !protocol_version.retryable);
}

/// `agent_status` reports incremental history from the Hello capabilities
/// that `ensure_hello` holds. A timed-out Hello keeps them; a refused one
/// clears them, and the next `ensure_hello` fetches them again.
#[test]
fn ensure_hello_fills_the_capabilities_and_a_timed_out_hello_keeps_them() {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixListener;
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("agent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let server = std::thread::spawn(move || {
        answer_socket_hello(&listener);
        // A Hello left unanswered until the client gives up.
        let (mut stream, _) = listener.accept().unwrap();
        foks_agent_proto::decode_hello(&read_socket_frame(&mut stream))
            .unwrap()
            .expect("a hello");
        assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
        // An agent that predates the Hello refuses it.
        let (mut stream, _) = listener.accept().unwrap();
        let frame = read_socket_frame(&mut stream);
        let refusal = serde_json::json!({
            "version": 34,
            "id": foks_agent_proto::request_id(&frame),
            "status": "error",
            "code": "version-mismatch",
            "message": "unsupported protocol version",
        });
        stream
            .write_all(&foks_agent_proto::encode(&refusal).unwrap())
            .unwrap();
        answer_socket_hello(&listener);
    });
    let mut handle = AgentHandle::new(socket);
    Arc::get_mut(&mut handle.transport)
        .unwrap()
        .client
        .set_timeout(Duration::from_millis(300))
        .unwrap();
    let handle = Arc::new(handle);
    let history_after = foks_agent_proto::hello::capability::HISTORY_AFTER;
    assert!(!handle.advertises(history_after));
    tauri::async_runtime::block_on(handle.ensure_hello()).unwrap();
    assert!(handle.advertises(history_after));
    // Held capabilities answer without another Hello.
    tauri::async_runtime::block_on(handle.ensure_hello()).unwrap();
    let timed_out = handle.probe_status().unwrap_err();
    assert_eq!(timed_out.code, "deadline-exceeded");
    assert!(handle.advertises(history_after));
    let refused = handle.probe_status().unwrap_err();
    assert_eq!(refused.code, "version-mismatch");
    assert!(!handle.advertises(history_after));
    tauri::async_runtime::block_on(handle.ensure_hello()).unwrap();
    assert!(handle.advertises(history_after));
    server.join().unwrap();
}
