use super::*;

#[test]
fn exit_barrier_blocks_retained_transports_startup_and_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let handle = AgentHandle::new(temporary.path().join("agent.sock"));
    let retained = Arc::clone(&handle.transport);
    handle.begin_exit();
    assert!(retained.reserve_use().is_err());
    assert!(retained.preempted(true));
    assert!(!retained.preempted(false));
    assert!(handle.ensure_started_blocking().is_err());
    assert!(handle
        .launch_unless_exiting(Path::new("/nonexistent-agent"), temporary.path())
        .is_err());
    assert!(handle.retry_started_blocking().is_err());
    assert!(handle.auto_recover_blocking().is_err());
    *retained.disposition.lock().unwrap() = TransportDisposition::Current;
    assert!(retained.reserve_use().is_err());
}

#[cfg(unix)]
#[test]
fn exit_waits_for_unowned_process_and_force_stop_works_without_socket() {
    let temporary = tempfile::tempdir().unwrap();
    let binary = temporary.path().join("foks-agent");
    let source = temporary.path().join("agent.c");
    let ready = temporary.path().join("ready");
    let socket = temporary.path().join("agent.sock");
    std::fs::write(
        &source,
        r#"
#include <fcntl.h>
#include <signal.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>
static int listener;
static char *socket_path;
static void stop_listener(int sig) { close(listener); unlink(socket_path); }
int main(int argc, char **argv) {
    socket_path = argv[1];
    listener = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un address;
    memset(&address, 0, sizeof address);
    address.sun_family = AF_UNIX;
    strncpy(address.sun_path, argv[1], sizeof address.sun_path - 1);
    if (bind(listener, (struct sockaddr *)&address, sizeof address) != 0
        || chmod(argv[1], 0600) != 0 || listen(listener, 8) != 0) return 1;
    signal(SIGTERM, stop_listener);
    close(open(argv[2], O_CREAT | O_WRONLY, 0600));
    for (;;) pause();
}
"#,
    )
    .unwrap();
    assert!(Command::new("cc")
        .arg("-o")
        .arg(&binary)
        .arg(&source)
        .status()
        .unwrap()
        .success());
    let mut child = Command::new(&binary)
        .arg(&socket)
        .arg(&ready)
        .arg("--state-dir")
        .arg(temporary.path())
        .spawn()
        .unwrap();
    let pid = child.id();
    struct Cleanup(u32);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.0 as i32, libc::SIGKILL);
            }
        }
    }
    let cleanup = Cleanup(pid);
    let reaper = std::thread::spawn(move || child.wait().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "test process failed to initialize"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let handle = AgentHandle::new(temporary.path().join("agent.sock"));
    let targets = handle.discover_exit_targets().unwrap();
    let target = targets.iter().find(|target| target.pid == pid).unwrap();
    let mut changed = target.clone();
    changed.started_at += 1;
    assert_eq!(
        stop_exit_process(&changed, true, Duration::ZERO)
            .unwrap_err()
            .code,
        "agent-exit-changed"
    );
    assert!(!process_has_exited(pid));
    assert!(stop_exit_process(target, false, Duration::from_millis(100)).is_err());
    assert!(!process_has_exited(pid));
    assert!(!socket.exists());
    // Even an outstanding mutation cannot prevent an explicit force stop.
    // Neither a live socket nor MANAGED_AGENT_PID is needed to find it.
    let _active_mutation = handle.transport.maintenance.read().unwrap();
    handle.force_stop_for_exit().unwrap();
    assert!(process_has_exited(pid));
    reaper.join().unwrap();
    std::mem::forget(cleanup);
}

#[test]
fn managed_endpoint_uses_rust_specific_names() {
    let state = default_state_directory().unwrap();
    #[cfg(target_os = "macos")]
    assert!(state.ends_with("Library/Application Support/foks-rs"));
    #[cfg(not(target_os = "macos"))]
    assert!(state.ends_with("foks-rs"));
    assert_eq!(default_socket(), Some(state.join(DEFAULT_SOCKET_NAME)));
}

#[test]
fn socket_resolution_precedence_is_stable() {
    assert_eq!(
        resolve_endpoint(
            ["--agent-socket", "/run/argument.sock"],
            Some(PathBuf::from("/run/environment.sock"))
        )
        .map(|endpoint| endpoint.socket),
        Some(PathBuf::from("/run/argument.sock"))
    );
    assert_eq!(
        resolve_endpoint(
            ["--unrelated"],
            Some(PathBuf::from("/run/environment.sock"))
        )
        .map(|endpoint| endpoint.socket),
        Some(PathBuf::from("/run/environment.sock"))
    );
}

#[test]
fn only_the_default_managed_endpoint_owns_crash_marker_storage() {
    let explicit = resolve_endpoint(
        ["--agent-socket", "/tmp/caller-owned/agent.sock"],
        Some(PathBuf::from("/tmp/environment-owned/agent.sock")),
    )
    .unwrap();
    assert_eq!(
        explicit.socket,
        PathBuf::from("/tmp/caller-owned/agent.sock")
    );
    assert_eq!(explicit.managed_crash_directory, None);

    let managed_socket = default_socket().unwrap();
    let explicit_managed_path = resolve_endpoint(
        [
            std::ffi::OsString::from(SOCKET_ARG),
            managed_socket.as_os_str().to_os_string(),
        ],
        None,
    )
    .unwrap();
    assert_eq!(explicit_managed_path.socket, managed_socket);
    assert_eq!(explicit_managed_path.managed_crash_directory, None);

    let environment = resolve_endpoint(
        ["--unrelated"],
        Some(PathBuf::from("/tmp/environment-owned/agent.sock")),
    )
    .unwrap();
    assert_eq!(
        environment.socket,
        PathBuf::from("/tmp/environment-owned/agent.sock")
    );
    assert_eq!(environment.managed_crash_directory, None);

    let environment_managed_path =
        resolve_endpoint(std::iter::empty::<&str>(), default_socket()).unwrap();
    assert_eq!(environment_managed_path.socket, default_socket().unwrap());
    assert_eq!(environment_managed_path.managed_crash_directory, None);

    let empty_environment =
        resolve_endpoint(std::iter::empty::<&str>(), Some(PathBuf::new())).unwrap();
    assert_eq!(empty_environment.socket, default_socket().unwrap());
    assert!(empty_environment.managed_crash_directory.is_some());

    let managed = resolve_endpoint(std::iter::empty::<&str>(), None).unwrap();
    assert_eq!(managed.socket, default_socket().unwrap());
    assert_eq!(
        managed.managed_crash_directory,
        default_state_directory().map(|state| state.join("crashes"))
    );
}

#[test]
fn managed_state_and_spawn_lock_are_private() {
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state");
    prepare_state_directory(&state).unwrap();
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(acquire_spawn_lock(&state.join("agent.sock"))
        .unwrap()
        .metadata()
        .unwrap()
        .is_file());
}

#[test]
fn managed_crash_storage_prepares_private_owned_state() {
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state");
    let crashes = state.join("crashes");
    prepare_managed_crash_directory(&crashes).unwrap();
    for directory in [state, crashes] {
        let metadata = std::fs::symlink_metadata(directory).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    }
}

#[test]
fn managed_crash_storage_does_not_parse_client_state() {
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state");
    prepare_state_directory(&state).unwrap();
    std::fs::write(
        state.join("client-state.toml"),
        b"version = 2\nstate_id = \"legacy\"\ncredential_backend = \"native\"\n",
    )
    .unwrap();

    let crashes = state.join("crashes");
    prepare_managed_crash_directory(&crashes).unwrap();
    assert!(crashes.is_dir());
}

#[test]
fn managed_launch_passes_state_socket_and_the_desktop_request_timeout() {
    let arguments = managed_agent_arguments(
        Path::new("/private/foks-state"),
        Path::new("/private/foks-state/agent.sock"),
    );
    assert_eq!(
        arguments,
        [
            std::ffi::OsString::from("--state-dir"),
            std::ffi::OsString::from("/private/foks-state"),
            std::ffi::OsString::from("--socket"),
            std::ffi::OsString::from("/private/foks-state/agent.sock"),
            std::ffi::OsString::from("--request-timeout-seconds"),
            std::ffi::OsString::from("60"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn managed_launch_reports_only_bounded_output_from_the_failed_process() {
    use std::os::unix::fs::PermissionsExt as _;

    let temporary = tempfile::tempdir().unwrap();
    let binary = temporary.path().join("foks-agent");
    let socket = temporary.path().join("agent.sock");
    let log = temporary.path().join("agent.log");
    std::fs::write(&log, "stale failure from an earlier launch\n").unwrap();
    let output = format!(
        "foks-agent: bind failed: Address already in use (os error 48)\n{}",
        "x".repeat(5000)
    );
    std::fs::write(
        &binary,
        format!("#!/bin/sh\nprintf '%s' '{output}' >&2\nexit 23\n"),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();

    let mut launch = launch_agent(&binary, temporary.path(), &socket).unwrap();
    let error = (0..100)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(10));
            launch.early_exit_error()
        })
        .expect("failed agent should exit promptly");
    assert_eq!(error.code, "agent-start-failed");
    assert!(error.retryable);
    assert!(!error.fatal);
    assert!(error.message.contains("exit status: 23"));
    assert!(!error.message.contains("stale failure"));
    let diagnostic = error.message.split_once("Agent output:\n").unwrap().1;
    assert!(diagnostic.starts_with("foks-agent: bind failed: Address already in use"));
    assert!(diagnostic.len() <= MAX_AGENT_STARTUP_DIAGNOSTIC_BYTES as usize);

    std::fs::write(&binary, "#!/bin/sh\nexit 7\n").unwrap();
    let mut launch = launch_agent(&binary, temporary.path(), &socket).unwrap();
    let error = (0..100)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(10));
            launch.early_exit_error()
        })
        .expect("silent failed agent should exit promptly");
    assert!(error.message.contains("exit status: 7"));
    assert!(!error.message.contains("Agent output:"));
    assert!(!error.message.contains("bind failed"));
}

#[test]
fn packaged_agent_discovery_matches_the_platform_bundle_layout() {
    #[cfg(target_os = "macos")]
    assert_eq!(
        packaged_agent_candidate(Path::new(
            "/Applications/FOKS.app/Contents/MacOS/foks-desktop"
        )),
        Some(PathBuf::from(
            "/Applications/FOKS.app/Contents/MacOS/foks-agent"
        ))
    );
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        packaged_agent_candidate(Path::new("/usr/bin/foks-desktop")),
        Some(PathBuf::from("/usr/bin/foks-agent"))
    );
}

#[cfg(unix)]
#[test]
fn only_named_foks_agent_processes_are_replaceable() {
    assert!(is_foks_agent_executable(Path::new(
        "/Applications/FOKS.app/Contents/MacOS/foks-agent"
    )));
    assert!(is_foks_agent_executable(Path::new(
        "/tmp/foks-agent (deleted)"
    )));
    assert!(!is_foks_agent_executable(Path::new(
        "/Applications/FOKS.app/Contents/MacOS/foks-desktop"
    )));
    assert!(!is_replaceable_agent_process(0));
    assert!(!is_replaceable_agent_process(1));
    assert!(!is_replaceable_agent_process(std::process::id()));
}

#[cfg(unix)]
#[test]
fn agent_state_directory_requires_an_explicit_argument() {
    assert_eq!(
        agent_state_dir(&[
            "foks-agent".into(),
            "--state-dir".into(),
            "/private/foks".into(),
        ]),
        Some(PathBuf::from("/private/foks"))
    );
    assert_eq!(
        agent_state_dir(&["foks-agent".into(), "--state-dir=/private/other".into(),]),
        Some(PathBuf::from("/private/other"))
    );
    assert_eq!(agent_state_dir(&["foks-agent".into()]), None);
    assert_eq!(
        agent_state_dir(&["foks-agent".into(), "--state-dir".into()]),
        None
    );
}

#[cfg(unix)]
#[test]
fn process_inspection_identifies_the_current_invocation() {
    let pid = std::process::id();
    assert_eq!(process_owner_uid(pid).unwrap(), unsafe { libc::geteuid() });
    assert!(!process_arguments(pid).unwrap().is_empty());
    assert!(process_ids().unwrap().contains(&pid));
    assert!(process_start_time(pid).is_ok());
}

#[cfg(unix)]
#[test]
fn unix_peer_pid_identifies_the_same_process_listener() {
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::{UnixListener, UnixStream};

    let temporary = tempfile::tempdir().unwrap();
    let socket = temporary.path().join("agent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(200));
    });
    let stream = UnixStream::connect(&socket).unwrap();
    assert_eq!(unix_peer_pid(&stream).unwrap(), std::process::id());
    drop(stream);
    server.join().unwrap();
}

#[cfg(unix)]
#[test]
fn parent_pid_and_orphan_detection_read_the_process_table() {
    let mut child = std::process::Command::new("sleep")
        .arg("5")
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    assert_eq!(process_parent_pid(pid).unwrap(), std::process::id());
    // A live child is supervised by this process, not reparented to init.
    assert!(!process_is_orphaned(pid));
    assert!(!process_is_orphaned(std::process::id()));
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(process_parent_pid(pid).is_err() || process_has_exited(pid));
}

#[cfg(unix)]
#[test]
fn same_file_compares_by_identity_not_path() {
    let temporary = tempfile::tempdir().unwrap();
    let file = temporary.path().join("foks-agent");
    std::fs::write(&file, b"").unwrap();
    let link = temporary.path().join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(same_file(&file, &link));
    let other = temporary.path().join("other");
    std::fs::write(&other, b"").unwrap();
    assert!(!same_file(&file, &other));
    assert!(!same_file(&file, &temporary.path().join("missing")));
}
