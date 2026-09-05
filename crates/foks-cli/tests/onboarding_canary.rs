use std::process::Command;

#[test]
fn onboarding_canary_exercises_real_cli_lifecycle() {
    let environment = foks_server_testkit::TestEnvironment::new().unwrap();
    let _server = environment.start_server().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let ca = temporary.path().join("ca.der");
    environment.write_probe_root(&ca).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/foks-client/onboarding-canary.py");
    let result = Command::new("python3")
        .arg(script)
        .args(["--client", env!("CARGO_BIN_EXE_foks-rs"), "--target"])
        .arg(format!(
            "localhost:{}",
            environment.addresses().unwrap().probe.port()
        ))
        .arg("--ca-der")
        .arg(ca)
        .env("FOKS_CANARY_ALLOW_DISPOSABLE_SIGNUP", "1")
        .env("FOKS_CANARY_SIGNUP_EMAIL", "canary@example.test")
        .env_remove("FOKS_CANARY_INVITE_FILE")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "signup\ndevice-administration\nrecovery\npassphrases\n"
    );
}
