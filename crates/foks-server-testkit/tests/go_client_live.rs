use std::path::PathBuf;
use std::process::Command;

use foks_server_db::InviteRegime;
use foks_server_testkit::TestEnvironment;

#[test]
fn official_go_client_completes_the_standalone_flow() {
    let Some(oracle) = std::env::var_os("FOKS_GO_ORACLE_DIR").map(PathBuf::from) else {
        return;
    };
    let environment = TestEnvironment::new().unwrap();
    environment
        .set_invite_regime(InviteRegime::Optional)
        .unwrap();
    let server = environment.start_server().unwrap();
    let addresses = server.addresses();
    let root = environment
        .client_path("go-v019", "probe-root.der")
        .unwrap();
    environment.write_probe_root(&root).unwrap();

    let output = Command::new("go")
        .arg("test")
        .arg("-C")
        .arg(&oracle)
        .arg("-mod=readonly")
        .arg("-run")
        .arg("^TestGoClientAgainstRustServer$")
        .arg("-count=1")
        .arg("-timeout=2m")
        .arg("-v")
        .env("FOKS_GO_RUST_PROBE", addresses.probe.to_string())
        .env("FOKS_GO_RUST_CA_DER", &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "official Go client failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
