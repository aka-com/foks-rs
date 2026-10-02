use super::*;

#[test]
fn maintenance_worker_owns_exclusion_and_restores_after_completion() {
    let dir = tempfile::tempdir().unwrap();
    let process = FakeMaintenanceProcess::healthy();
    let handle = AgentHandle::new_for_maintenance_test(
        dir.path().join(DEFAULT_SOCKET_NAME),
        process.clone(),
        |_, _| SafeRootDisposition::Current,
    );
    let observed = Mutex::new(Vec::new());
    let snapshot = handle
        .run_maintenance(
            MaintenanceKind::Verify,
            &|snapshot| observed.lock().unwrap().push(snapshot.clone()),
            |worker| {
                let blocked = handle.transport().call(Operation::AgentStatus).unwrap_err();
                assert_eq!(
                    blocked,
                    DesktopAgentError::Local(foks_desktop::LocalAgentCondition::Maintenance)
                );
                worker.stop_owned_agent().unwrap();
                assert!(handle
                    .run_maintenance(MaintenanceKind::Export, &|_| {}, |_| {
                        MaintenanceCompletion::Continue
                    })
                    .is_err());
                MaintenanceCompletion::Continue
            },
        )
        .unwrap();
    assert_eq!(process.stop_calls.load(Ordering::Acquire), 1);
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
    assert!(matches!(
        snapshot,
        MaintenanceSnapshot::Complete {
            operation: MaintenanceOperationOutcome::Completed,
            disposition: MaintenanceDisposition::ContinueCurrentRoot,
            ..
        }
    ));
    let phases: Vec<_> = observed
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|snapshot| match snapshot {
            MaintenanceSnapshot::Active { phase, .. } => Some(phase),
            MaintenanceSnapshot::Idle { .. } | MaintenanceSnapshot::Complete { .. } => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            Some(MaintenancePhase::Selecting),
            Some(MaintenancePhase::Quiescing),
            Some(MaintenancePhase::Running),
            Some(MaintenancePhase::Restoring),
            None,
        ]
    );
}

#[test]
fn stop_uncertainty_still_runs_explicit_restoration_and_preserves_both_errors() {
    let dir = tempfile::tempdir().unwrap();
    let stop_error = AgentError::new("agent-stop-failed", "exit timed out", true);
    let restore_error = AgentError::new("agent-start-failed", "spawn failed", true);
    let process = Arc::new(FakeMaintenanceProcess {
        stop_calls: AtomicUsize::new(0),
        restore_calls: AtomicUsize::new(0),
        stop_error: Some(stop_error.clone()),
        restore_error: Some(restore_error.clone()),
    });
    let handle = AgentHandle::new_for_maintenance_test(
        dir.path().join(DEFAULT_SOCKET_NAME),
        process.clone(),
        |_, _| SafeRootDisposition::Current,
    );
    let snapshot = handle
        .run_maintenance(MaintenanceKind::Export, &|_| {}, |worker| {
            let error = worker.stop_owned_agent().unwrap_err();
            MaintenanceCompletion::Failed {
                error,
                affected_roots: Vec::new(),
            }
        })
        .unwrap();
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
    assert!(matches!(
        &snapshot,
        MaintenanceSnapshot::Complete {
            operation: MaintenanceOperationOutcome::Failed { error },
            disposition: MaintenanceDisposition::RestorationFailed { error: restoration, .. },
            ..
        } if error == &stop_error && restoration == &restore_error
    ));
    assert_eq!(
        handle.transport().call(Operation::AgentStatus).unwrap_err(),
        DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RestorationFailed)
    );
    let failed_revision = match &snapshot {
        MaintenanceSnapshot::Complete { revision, .. } => *revision,
        _ => unreachable!(),
    };
    handle.record_restoration_success(&|_| {});
    assert!(matches!(
        handle.maintenance_snapshot(),
        MaintenanceSnapshot::Complete {
            revision,
            disposition: MaintenanceDisposition::ContinueCurrentRoot,
            ..
        } if revision > failed_revision
    ));
}

#[test]
fn restoration_retry_keeps_typed_gate_until_fake_process_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    let process = FakeMaintenanceProcess::healthy();
    let handle = AgentHandle::new_for_maintenance_test(
        dir.path().join(DEFAULT_SOCKET_NAME),
        process.clone(),
        |_, _| SafeRootDisposition::Current,
    );
    *handle.transport.disposition.lock().unwrap() = TransportDisposition::RestorationFailed;
    assert_eq!(
        handle.transport().call(Operation::AgentStatus).unwrap_err(),
        DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RestorationFailed)
    );

    let response = handle.retry_started_blocking().unwrap();
    assert!(matches!(response.result, ResponseResult::Success { .. }));
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        *handle.transport.disposition.lock().unwrap(),
        TransportDisposition::Current
    );
}

#[test]
fn cancellation_before_stop_does_not_touch_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let process = FakeMaintenanceProcess::healthy();
    let handle = AgentHandle::new_for_maintenance_test(
        dir.path().join(DEFAULT_SOCKET_NAME),
        process.clone(),
        |_, _| SafeRootDisposition::Current,
    );
    for kind in [
        MaintenanceKind::Export,
        MaintenanceKind::Import,
        MaintenanceKind::Relocate,
    ] {
        let snapshot = handle
            .run_maintenance(kind, &|_| {}, |_| MaintenanceCompletion::Cancelled)
            .unwrap();
        assert!(matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                operation: MaintenanceOperationOutcome::Cancelled,
                disposition: MaintenanceDisposition::ContinueCurrentRoot,
                ..
            }
        ));
    }
    assert_eq!(process.stop_calls.load(Ordering::Acquire), 0);
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
}

#[test]
fn worker_panic_after_stop_is_failed_operation_and_restores() {
    let dir = tempfile::tempdir().unwrap();
    let process = FakeMaintenanceProcess::healthy();
    let handle = AgentHandle::new_for_maintenance_test(
        dir.path().join(DEFAULT_SOCKET_NAME),
        process.clone(),
        |_, _| SafeRootDisposition::Current,
    );
    let snapshot = handle
        .run_maintenance(MaintenanceKind::Verify, &|_| {}, |worker| {
            worker.stop_owned_agent().unwrap();
            panic!("simulated worker failure");
        })
        .unwrap();
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
    assert!(matches!(
        snapshot,
        MaintenanceSnapshot::Complete {
            operation: MaintenanceOperationOutcome::Failed { ref error },
            disposition: MaintenanceDisposition::ContinueCurrentRoot,
            ..
        } if error.code == "maintenance-worker-interrupted"
    ));
}

#[test]
fn recovery_and_selected_root_dispositions_never_reopen_the_old_root() {
    let dir = tempfile::tempdir().unwrap();
    let process = FakeMaintenanceProcess::healthy();
    let recovery_root = dir.path().join("recover");
    let recovery = recovery_root.clone();
    let handle = AgentHandle::new_for_maintenance_test(
        dir.path().join("current").join(DEFAULT_SOCKET_NAME),
        process.clone(),
        move |_, _| SafeRootDisposition::Recovery(recovery.clone()),
    );
    let snapshot = handle
        .run_maintenance(MaintenanceKind::Import, &|_| {}, |worker| {
            worker.stop_owned_agent().unwrap();
            MaintenanceCompletion::Failed {
                error: AgentError::new("import-failed", "durable boundary", false),
                affected_roots: vec![recovery_root.clone()],
            }
        })
        .unwrap();
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
    assert!(matches!(
        snapshot,
        MaintenanceSnapshot::Complete {
            operation: MaintenanceOperationOutcome::Failed { .. },
            disposition: MaintenanceDisposition::RecoveryRequired { .. },
            ..
        }
    ));
    assert_eq!(
        handle.transport().call(Operation::AgentStatus).unwrap_err(),
        DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RecoveryRequired)
    );

    let selected = dir.path().join("selected");
    let expected = selected.clone();
    let process = FakeMaintenanceProcess::healthy();
    let moved = AgentHandle::new_for_maintenance_test(
        dir.path().join("old").join(DEFAULT_SOCKET_NAME),
        process.clone(),
        move |_, _| SafeRootDisposition::Selected(expected.clone()),
    );
    let snapshot = moved
        .run_maintenance(MaintenanceKind::Relocate, &|_| {}, |worker| {
            worker.stop_owned_agent().unwrap();
            MaintenanceCompletion::RestartSelected(selected.clone())
        })
        .unwrap();
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
    assert!(matches!(
        snapshot,
        MaintenanceSnapshot::Complete {
            disposition: MaintenanceDisposition::RestartSelectedRoot { .. },
            ..
        }
    ));
    assert_eq!(
        moved.transport().call(Operation::AgentStatus).unwrap_err(),
        DesktopAgentError::Local(foks_desktop::LocalAgentCondition::RestartRequired)
    );

    let selected = dir.path().join("import-selected-after-failure");
    let expected = selected.clone();
    let process = FakeMaintenanceProcess::healthy();
    let imported = AgentHandle::new_for_maintenance_test(
        dir.path().join("import-old").join(DEFAULT_SOCKET_NAME),
        process.clone(),
        move |_, _| SafeRootDisposition::Selected(expected.clone()),
    );
    let snapshot = imported
        .run_maintenance(MaintenanceKind::Import, &|_| {}, |worker| {
            worker.stop_owned_agent().unwrap();
            MaintenanceCompletion::Failed {
                error: AgentError::new(
                    "import-failed",
                    "selection was durably published before cleanup failed",
                    false,
                ),
                affected_roots: vec![selected.clone()],
            }
        })
        .unwrap();
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
    assert!(matches!(
        snapshot,
        MaintenanceSnapshot::Complete {
            operation: MaintenanceOperationOutcome::Failed { .. },
            disposition: MaintenanceDisposition::RestartSelectedRoot { .. },
            ..
        }
    ));
}

#[test]
fn maintenance_reservation_drains_preemptible_calls_and_releases_the_signal() {
    let dir = tempfile::tempdir().unwrap();
    let handle = AgentHandle::new(dir.path().join(DEFAULT_SOCKET_NAME));
    let transport = Arc::clone(&handle.transport);

    // A call that parks on the agent, as a chat inbox poll does for its
    // full 55 seconds, releases its reservation once maintenance asks.
    let started = Arc::new((Mutex::new(false), Condvar::new()));
    let poll_started = Arc::clone(&started);
    let polling = Arc::clone(&transport);
    let poll = std::thread::spawn(move || {
        let _use = polling.reserve_use().unwrap();
        let (held, ready) = &*poll_started;
        *held.lock().unwrap() = true;
        ready.notify_all();
        while !polling.preempted(true) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!polling.preempted(false), "a mutation is never preempted");
    });
    let (held, ready) = &*started;
    let mut waiting = held.lock().unwrap();
    while !*waiting {
        waiting = ready.wait(waiting).unwrap();
    }
    drop(waiting);
    let reservation = transport
        .reserve_for_maintenance(MAINTENANCE_RESERVATION_WAIT)
        .expect("a preemptible call gives the reservation up");
    poll.join().unwrap();
    // The reservation keeps later calls out on its own, so the drain
    // signal is not left set behind it.
    assert!(!transport.maintenance_pending.load(Ordering::Acquire));
    assert_eq!(
        transport.reserve_use().unwrap_err(),
        DesktopAgentError::Local(foks_desktop::LocalAgentCondition::Maintenance)
    );
    drop(reservation);

    // A call that will not give it up costs the wait, and leaves the
    // transport usable rather than refusing every later call.
    let retained = transport.reserve_use().unwrap();
    assert!(transport
        .reserve_for_maintenance(Duration::from_millis(60))
        .is_none());
    assert!(!transport.maintenance_pending.load(Ordering::Acquire));
    drop(retained);
    assert!(transport.reserve_use().is_ok());
}

#[test]
fn maintenance_excludes_retained_transports_and_restart_required_startup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("absent-state");
    let handle = AgentHandle::new(root.join(DEFAULT_SOCKET_NAME));
    let transport = handle.transport();
    {
        let _maintenance = handle.transport.maintenance.write().unwrap();
        assert!(handle.call_blocking(Operation::AgentStatus).is_err());
        assert!(transport.call(Operation::AgentStatus).is_err());
        assert!(handle.ensure_started_blocking().is_err());
    }
    *handle.transport.disposition.lock().unwrap() = TransportDisposition::RestartRequired;
    assert!(transport.call(Operation::AgentStatus).is_err());
    assert!(handle.ensure_started_blocking().is_err());
    assert!(!root.exists());
}

#[test]
fn external_agent_endpoint_rejects_maintenance_before_running_worker() {
    let dir = tempfile::tempdir().unwrap();
    let handle = AgentHandle::new(dir.path().join(DEFAULT_SOCKET_NAME));
    let ran = AtomicBool::new(false);
    let error = handle
        .run_maintenance(MaintenanceKind::Export, &|_| {}, |_| {
            ran.store(true, Ordering::Release);
            MaintenanceCompletion::Continue
        })
        .unwrap_err();
    assert_eq!(error.code, "external-agent");
    assert!(!ran.load(Ordering::Acquire));
    assert!(matches!(
        handle.maintenance_snapshot(),
        MaintenanceSnapshot::Idle { .. }
    ));
}

#[test]
fn stale_maintenance_events_cannot_replace_a_newer_generation() {
    let dir = tempfile::tempdir().unwrap();
    let handle = AgentHandle::new(dir.path().join(DEFAULT_SOCKET_NAME));
    let observed = Mutex::new(Vec::new());
    let notify = |snapshot: &MaintenanceSnapshot| {
        observed.lock().unwrap().push(snapshot.generation());
    };
    handle.publish_maintenance(
        MaintenanceSnapshot::Active {
            generation: 7,
            revision: 0,
            kind: MaintenanceKind::Verify,
            phase: MaintenancePhase::Running,
        },
        &notify,
    );
    handle.publish_maintenance(
        MaintenanceSnapshot::Complete {
            generation: 7,
            revision: 0,
            kind: MaintenanceKind::Export,
            operation: MaintenanceOperationOutcome::Completed,
            disposition: MaintenanceDisposition::ContinueCurrentRoot,
        },
        &notify,
    );
    handle.publish_maintenance(
        MaintenanceSnapshot::Active {
            generation: 7,
            revision: 0,
            kind: MaintenanceKind::Verify,
            phase: MaintenancePhase::Restoring,
        },
        &notify,
    );
    assert_eq!(observed.into_inner().unwrap(), vec![7, 7]);
    assert_eq!(handle.maintenance_snapshot().generation(), 7);
    assert!(matches!(
        handle.maintenance_snapshot(),
        MaintenanceSnapshot::Complete { .. }
    ));
}

#[test]
fn auto_recovery_validates_status_and_clears_only_successful_probe_failures() {
    use std::io::Write as _;
    for response_kind in ["ready", "bootstrap", "error", "invalid", "version"] {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_socket_request(&mut stream);
            let response = match response_kind {
                "error" => Response::error(request.id, ErrorCode::Busy, "not ready"),
                "version" => {
                    Response::error(request.id, ErrorCode::VersionMismatch, "incompatible")
                }
                "invalid" => Response::success(request.id, serde_json::json!({"unrelated": true})),
                "bootstrap" => Response::success(
                    request.id,
                    serde_json::to_value(foks_agent_proto::AgentStatus::Bootstrap {
                        step: "choose".into(),
                    })
                    .unwrap(),
                ),
                _ => Response::success(
                    request.id,
                    serde_json::to_value(foks_agent_proto::AgentStatus::ready()).unwrap(),
                ),
            };
            stream
                .write_all(&foks_agent_proto::encode(&response).unwrap())
                .unwrap();
        });
        let handle = AgentHandle::new(socket);
        *handle.connection_failure.lock().unwrap() = Some("older connection loss".into());
        let result = handle.auto_recover_blocking();
        assert_eq!(
            result.is_ok(),
            matches!(response_kind, "ready" | "bootstrap")
        );
        assert_eq!(handle.take_connection_failure().is_none(), result.is_ok());
        if let Err(error) = result {
            assert_eq!(
                error.code,
                match response_kind {
                    "version" => "version-mismatch",
                    "invalid" => "protocol",
                    _ => "busy",
                }
            );
        }
        server.join().unwrap();
    }
}

#[test]
fn recovery_credentials_are_distinct_and_explicit_retry_rechecks_them() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = root.join("client-state.toml");
    std::fs::write(&config, b"fixture").unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    let checks = Arc::new(AtomicU64::new(0));
    let observed = checks.clone();
    let process = FakeMaintenanceProcess::healthy();
    let handle = AgentHandle::new_for_maintenance_test(
        root.join("agent.sock"),
        process.clone(),
        move |root, _| {
            if observed.fetch_add(1, Ordering::SeqCst) < 2 {
                SafeRootDisposition::CredentialsRequired(root.to_owned())
            } else {
                SafeRootDisposition::Current
            }
        },
    );
    assert_eq!(
        handle.check_recovery_credentials(false).unwrap_err().code,
        "agent-credentials-required"
    );
    assert_eq!(
        handle.retry_started_blocking().unwrap_err().code,
        "agent-credentials-required"
    );
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
    handle.retry_started_blocking().unwrap();
    assert_eq!(checks.load(Ordering::SeqCst), 3);
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 1);
    assert!(!handle.recovery_credentials_required.load(Ordering::Acquire));
}

#[test]
fn automatic_recovery_checks_live_listener_before_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("agent.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let handle =
        AgentHandle::new_for_maintenance_test(socket, FakeMaintenanceProcess::healthy(), |_, _| {
            panic!("must not access credentials")
        });
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_socket_request(&mut stream);
        drop(stream);
        // Keep the listener live through the second, socket-only check.
        let _ = listener.accept().unwrap();
    });
    assert_eq!(
        handle.auto_recover_blocking().unwrap_err().code,
        "agent-busy"
    );
    server.join().unwrap();
}

#[test]
fn auto_recovery_endpoint_checks_allow_only_missing_or_private_stale_sockets() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("agent.sock");
    let handle = AgentHandle::new(socket.clone());
    assert!(handle.require_missing_agent_endpoint().is_ok());
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        handle.require_missing_agent_endpoint().unwrap_err().code,
        "agent-busy"
    );
    drop(listener);
    assert!(handle.require_missing_agent_endpoint().is_ok());
    assert!(socket.exists());
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        handle.require_missing_agent_endpoint().unwrap_err().code,
        "unsafe-socket"
    );
    std::fs::remove_file(&socket).unwrap();
    std::os::unix::fs::symlink(directory.path().join("missing"), &socket).unwrap();
    assert_eq!(
        handle.require_missing_agent_endpoint().unwrap_err().code,
        "unsafe-socket"
    );
}

#[test]
fn auto_recovery_never_initializes_or_overrides_maintenance_or_unsafe_endpoints() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("agent.sock");
    let process = FakeMaintenanceProcess::healthy();
    let handle = AgentHandle::new_for_maintenance_test(socket.clone(), process.clone(), |_, _| {
        SafeRootDisposition::Current
    });
    assert_eq!(
        handle.auto_recover_blocking().unwrap_err().code,
        "agent-state"
    );
    assert!(!directory.path().join("client-state.toml").exists());
    assert_eq!(handle.take_connection_failure(), None);
    for (disposition, code) in [
        (
            TransportDisposition::RestorationFailed,
            "state-restoration-failed",
        ),
        (
            TransportDisposition::RecoveryRequired,
            "state-recovery-required",
        ),
        (
            TransportDisposition::RestartRequired,
            "state-restart-required",
        ),
    ] {
        *handle.transport.disposition.lock().unwrap() = disposition;
        assert_eq!(handle.auto_recover_blocking().unwrap_err().code, code);
    }
    assert_eq!(process.restore_calls.load(Ordering::Acquire), 0);
    *handle.transport.disposition.lock().unwrap() = TransportDisposition::Current;
    handle.maintenance_in_flight.store(true, Ordering::Release);
    assert_eq!(
        handle.auto_recover_blocking().unwrap_err().code,
        "state-maintenance-active"
    );
    handle.maintenance_in_flight.store(false, Ordering::Release);
    {
        let _request = handle.transport.reserve_use().unwrap();
        assert_eq!(
            handle
                .auto_recover_with_reservation_timeout(Duration::ZERO)
                .unwrap_err()
                .code,
            "agent-busy"
        );
    }
    std::fs::write(&socket, b"not a socket").unwrap();
    assert_eq!(
        handle.auto_recover_blocking().unwrap_err().code,
        "unsafe-socket"
    );
    assert_eq!(
        handle.call_blocking(Operation::Ping).unwrap_err().code,
        "unsafe-socket"
    );
    assert!(!handle
        .transport()
        .call(Operation::Ping)
        .unwrap_err()
        .connection_lost());
    assert_eq!(handle.take_connection_failure(), None);
    assert_eq!(std::fs::read(&socket).unwrap(), b"not a socket");
    std::fs::remove_file(&socket).unwrap();
    let external = AgentHandle::new(socket);
    assert_eq!(
        external.auto_recover_blocking().unwrap_err().code,
        "external-agent"
    );
    assert_eq!(external.take_connection_failure(), None);
    assert_eq!(
        external.call_blocking(Operation::Ping).unwrap_err().code,
        "agent-lost"
    );
    assert!(external.take_connection_failure().is_some());
}

#[cfg(unix)]
#[test]
fn takeover_confirmation_revalidates_owners_and_continues_startup() {
    use std::cell::RefCell;
    use std::os::unix::process::ExitStatusExt as _;

    struct Fixture {
        root: tempfile::TempDir,
        binary: PathBuf,
        children: RefCell<Vec<std::process::Child>>,
    }
    impl Fixture {
        fn spawn(&self, socket: &Path, version: u32) -> u32 {
            let marker = self
                .root
                .path()
                .join(format!("ready-{}", self.children.borrow().len()));
            let child = Command::new(&self.binary)
                .arg(socket)
                .arg(&marker)
                .arg(version.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id();
            self.children.borrow_mut().push(child);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !marker.exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "dummy agent did not bind"
                );
                assert!(self
                    .children
                    .borrow_mut()
                    .last_mut()
                    .unwrap()
                    .try_wait()
                    .unwrap()
                    .is_none());
                std::thread::sleep(Duration::from_millis(10));
            }
            pid
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only clean up processes launched from this temporary test binary.
            let pid = *MANAGED_AGENT_PID.lock().unwrap();
            if let Some(pid) =
                pid.filter(|pid| process_executable_path(*pid).ok().as_ref() == Some(&self.binary))
            {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                for _ in 0..100 {
                    if process_has_exited(pid) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            for child in self.children.get_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let binary = root.path().join("foks-agent");
    let source = root.path().join("agent.c");
    std::fs::write(
        &source,
        include_str!("../../../tests/fixtures/takeover-agent.c"),
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg(format!(
            "-DPROTOCOL_VERSION={}",
            foks_agent_proto::PROTOCOL_VERSION
        ))
        .arg("-o")
        .arg(&binary)
        .arg(&source)
        .status()
        .unwrap()
        .success());
    let fixture = Fixture {
        root,
        binary: binary.canonicalize().unwrap(),
        children: RefCell::new(Vec::new()),
    };
    let socket = fixture.root.path().join("agent.sock");
    let handle = AgentHandle::new(socket.clone());
    let old = fixture.spawn(&socket, 1);
    assert_eq!(
        handle
            .call_blocking(Operation::AgentStatus)
            .unwrap_err()
            .code,
        "version-mismatch"
    );

    let error = handle
        .start_with_binary(&fixture.root.path().join("missing-agent"), &|_| {
            panic!("must validate replacement before requesting termination")
        })
        .unwrap_err();
    assert_eq!(error.code, "agent-binary");
    let error = handle
        .start_with_binary(&fixture.binary, &|_| Ok(false))
        .unwrap_err();
    assert_eq!(error.code, "agent-takeover-declined");
    assert_eq!(inspect_takeover_target(&socket).unwrap().unwrap().pid, old);

    // Replacement while the confirmation is open requires a new approval.
    let prompts = RefCell::new(Vec::new());
    let error = handle
        .start_with_binary(&fixture.binary, &|target| {
            prompts.borrow_mut().push(target.pid);
            if target.pid == old {
                fixture.spawn(&socket, 1); // deletes and rebinds the original path
                Ok(true)
            } else {
                Ok(false)
            }
        })
        .unwrap_err();
    assert_eq!(error.code, "agent-takeover-declined");
    assert_eq!(prompts.borrow().len(), 2);
    assert_ne!(prompts.borrow()[0], prompts.borrow()[1]);
    assert!(fixture
        .children
        .borrow_mut()
        .iter_mut()
        .all(|child| child.try_wait().unwrap().is_none()));

    // A new explicit approval actually stops that owner and launches our agent.
    let response = handle
        .start_with_binary(&fixture.binary, &|target| {
            assert_eq!(target.pid, prompts.borrow()[1]);
            Ok(true)
        })
        .unwrap();
    assert_eq!(success_value(response).unwrap()["state"], "ready");
    let status = fixture.children.borrow_mut()[1].wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
    assert!(handle.call_blocking(Operation::Ping).is_ok());

    // An already-compatible successor is reused, never terminated or prompted.
    let compatible_socket = fixture.root.path().join("compatible.sock");
    fixture.spawn(&compatible_socket, 1);
    let compatible = AgentHandle::new(compatible_socket.clone());
    let calls = RefCell::new(0);
    compatible
        .start_with_binary(&fixture.binary, &|_| {
            *calls.borrow_mut() += 1;
            fixture.spawn(&compatible_socket, foks_agent_proto::PROTOCOL_VERSION);
            Ok(true)
        })
        .unwrap();
    assert_eq!(*calls.borrow(), 1);
    assert!(fixture
        .children
        .borrow_mut()
        .last_mut()
        .unwrap()
        .try_wait()
        .unwrap()
        .is_none());

    // A respawning supervisor cannot trap startup in an unbounded kill loop.
    let contested_socket = fixture.root.path().join("contested.sock");
    fixture.spawn(&contested_socket, 1);
    let contested = AgentHandle::new(contested_socket.clone());
    let calls = RefCell::new(0);
    let error = contested
        .start_with_binary(&fixture.binary, &|_| {
            *calls.borrow_mut() += 1;
            fixture.spawn(&contested_socket, 1);
            Ok(true)
        })
        .unwrap_err();
    assert_eq!(error.code, "agent-takeover-limit");
    assert_eq!(*calls.borrow(), MAX_STARTUP_TAKEOVERS);
}

#[cfg(unix)]
#[test]
fn takeover_refuses_unsafe_sockets_and_unrelated_processes() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir_in("/tmp").unwrap();
    let socket = root.path().join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        inspect_takeover_target(&socket).unwrap_err().code,
        "unsafe-socket"
    );
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        inspect_takeover_target(&socket).unwrap_err().code,
        "version-mismatch"
    );
    assert!(socket.exists());
}

#[cfg(unix)]
#[test]
fn stop_incompatible_agent_terminates_a_named_listener() {
    use std::os::unix::process::ExitStatusExt as _;
    use std::process::{Command, Stdio};

    struct ChildGuard(std::process::Child);
    impl std::ops::Deref for ChildGuard {
        type Target = std::process::Child;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl std::ops::DerefMut for ChildGuard {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let temporary = tempfile::tempdir().unwrap();
    let socket = temporary.path().join("agent.sock");
    let ready = temporary.path().join("ready");
    let source = temporary.path().join("agent.c");
    let binary = temporary.path().join("foks-agent");
    std::fs::write(
        &source,
        r#"
#include <fcntl.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>
int main(int argc, char **argv) {
    int server = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un address;
    memset(&address, 0, sizeof address);
    address.sun_family = AF_UNIX;
    strncpy(address.sun_path, argv[1], sizeof address.sun_path - 1);
    unlink(argv[1]);
    if (server < 0 || bind(server, (struct sockaddr *)&address, sizeof address) != 0
        || chmod(argv[1], 0600) != 0 || listen(server, 8) != 0) {
        return 1;
    }
    int marker = open(argv[2], O_CREAT | O_WRONLY, 0600);
    if (marker >= 0) {
        close(marker);
    }
    for (;;) {
        int client = accept(server, 0, 0);
        if (client >= 0) {
            char byte;
            read(client, &byte, 1);
            close(client);
        }
    }
}
"#,
    )
    .unwrap();
    let compiled = Command::new("cc")
        .args(["-o"])
        .arg(&binary)
        .arg(&source)
        .status()
        .expect("cc is required to build the dummy agent");
    assert!(compiled.success(), "cc failed to build the dummy agent");
    let mut child = ChildGuard(
        Command::new(&binary)
            .args([&socket, &ready])
            .arg("--state-dir")
            .arg(temporary.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let started = std::time::Instant::now();
    while !ready.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("dummy foks-agent exited early: {status}");
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            panic!("dummy foks-agent did not bind");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let handle = AgentHandle::new(socket.clone());
    let candidates = handle.stale_agent_processes();
    let active = candidates
        .iter()
        .find(|target| target.pid == child.id())
        .unwrap();
    assert!(handle.additional_agent_processes().is_empty());
    assert_eq!(
        handle.terminate_additional_agent(active).unwrap_err().code,
        "agent-cleanup-changed"
    );
    let missing = AgentHandle::new(temporary.path().join("missing.sock"));
    assert!(missing.additional_agent_processes().is_empty());
    assert_eq!(
        missing.terminate_additional_agent(active).unwrap_err().code,
        "agent-cleanup-changed"
    );

    let extra_socket = temporary.path().join("extra.sock");
    let extra_ready = temporary.path().join("extra-ready");
    let mut extra = ChildGuard(
        Command::new(&binary)
            .args([&extra_socket, &extra_ready])
            .arg("--state-dir")
            .arg(temporary.path())
            .spawn()
            .unwrap(),
    );
    let started = std::time::Instant::now();
    while !extra_ready.exists() {
        if started.elapsed() > Duration::from_secs(5) {
            let _ = extra.kill();
            let _ = extra.wait();
            let _ = child.kill();
            let _ = child.wait();
            panic!("additional agent did not bind");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let candidates = handle.additional_agent_processes();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].pid, extra.id());
    let now_active = AgentHandle::new(extra_socket.clone());
    assert_eq!(
        now_active
            .terminate_additional_agent(&candidates[0])
            .unwrap_err()
            .code,
        "agent-cleanup-changed"
    );
    // An unreachable listener is still discoverable and can be cleaned up.
    std::fs::remove_file(&extra_socket).unwrap();
    assert_eq!(handle.additional_agent_processes(), candidates);
    let mut changed = candidates[0].clone();
    changed.started_at += 1;
    assert_eq!(
        handle
            .terminate_additional_agent(&changed)
            .unwrap_err()
            .code,
        "agent-cleanup-changed"
    );
    handle.terminate_additional_agent(&candidates[0]).unwrap();
    assert_eq!(extra.wait().unwrap().signal(), Some(libc::SIGTERM));
    assert!(child.try_wait().unwrap().is_none());

    let mut approvals = 0;
    stop_incompatible_agent(
        &socket,
        &|target| {
            assert_eq!(target.pid, child.id());
            assert_eq!(target.executable, binary.canonicalize().unwrap());
            Ok(true)
        },
        &mut approvals,
    )
    .unwrap();
    assert_eq!(approvals, 1);
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
}
