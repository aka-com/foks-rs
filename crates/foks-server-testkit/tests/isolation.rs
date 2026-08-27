use foks_server_testkit::IsolatedTestServer;

#[test]
fn every_path_and_socket_is_confined_to_the_test_root() {
    let server = IsolatedTestServer::start().unwrap();
    let root = server.root().canonicalize().unwrap();
    for path in server.owned_paths() {
        let existing = if path.exists() {
            path
        } else {
            path.parent().unwrap()
        };
        assert!(existing.canonicalize().unwrap().starts_with(&root));
    }
    for address in [
        server.addresses().probe,
        server.addresses().public_services,
        server.addresses().authenticated,
    ] {
        assert!(address.ip().is_loopback());
        assert_ne!(address.port(), 0);
    }
    if let Some(home) = std::env::var_os("HOME") {
        assert!(!root.starts_with(home));
    }
    assert!(!root.to_string_lossy().contains("AKA"));
    server.shutdown().unwrap();
}

#[test]
fn every_testkit_integration_test_uses_the_sealed_constructor() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for path in rust_sources(&tests) {
        let source = std::fs::read_to_string(&path).unwrap();
        let direct_bind = ["TcpListener", "::bind"].concat();
        let direct_database = ["Database", "::open"].concat();
        assert!(
            !source.contains(&direct_bind),
            "{} binds directly",
            path.display()
        );
        assert!(
            !source.contains(&direct_database),
            "{} opens a server database directly",
            path.display()
        );
        if source.contains("start_server()")
            || source.contains("start_probe_override(")
            || source.contains("IsolatedTestServer::start()")
        {
            assert!(
                source.contains("TestEnvironment::new()")
                    || source.contains("TestEnvironment::with_profile(")
                    || source.contains("IsolatedTestServer::start()")
                    || source.contains("Fixture::start("),
                "{} bypasses the sealed isolated environment",
                path.display()
            );
        }
        let forbidden = [
            ["std::env::var(\"", "HOME\")"].concat(),
            ["aka", "-"].concat(),
            ["AKA", "_DATA"].concat(),
        ];
        for forbidden in forbidden {
            assert!(
                !source.contains(&forbidden),
                "{} references a user or AKA data path",
                path.display()
            );
        }
    }
}

fn rust_sources(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources.extend(rust_sources(&path));
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            sources.push(path);
        }
    }
    sources
}
