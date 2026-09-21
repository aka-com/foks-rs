//! The path-addressed data adapter reads: what they cost, and what they
//! report for a path that does not exist.
//!
//! `data_stat`, `data_entry` and `data_chunk` each address one path, so they
//! walk only the directories on it. These tests pin both halves of that: the
//! request count must not grow with directories the path does not walk, and
//! an absent path must still produce the error a whole-namespace traversal
//! produced, because absence at a named path is decided by the complete
//! listing of the directory that would hold it.
use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, Error,
    KvMutationPrecondition, KvRoleSummary, Profile, ProfileRegistry, ProfileSession,
    ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

/// Unrelated top-level directories present for the first measurement, and
/// again added before the second.
const WIDTH: usize = 8;
/// A payload above the small-file boundary, so the addressed entry is a large
/// file and `data_chunk` applies to it.
const LARGE: usize = 300 * 1024;

#[test]
fn addressed_reads_cost_one_path_and_still_report_an_absent_path_as_a_conflict() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let root = state.join("root.der");
    environment.write_probe_root(&root).unwrap();
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            label: None,
            probe: format!(
                "localhost:{}",
                environment.addresses().unwrap().probe.port()
            ),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    let credentials = ClientCredentials::open(&state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            let master = credentials.master_key()?;
            session.probe_and_pin()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "pathreader",
                "device",
                "owner@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;

            let payload = vec![0x7a; LARGE];
            session.put_kv_file_checked(
                "owner",
                "/alpha/beta/large",
                &mut payload.as_slice(),
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                true,
                &mut vault,
                &master,
            )?;
            // Small files come back in the listing's extended side table, so
            // each unrelated directory costs exactly its own directory read
            // and listing in a complete traversal and nothing in a path walk.
            let widen = |session: &foks_client_app::CheckedProfileSession<'_>,
                         vault: &mut AccountVault<'_>,
                         from: usize|
             -> Result<(), Error> {
                for index in from..from + WIDTH {
                    session.put_kv_file_checked(
                        "owner",
                        &format!("/dir-{index:03}/item"),
                        &mut b"unrelated".as_slice(),
                        KvMutationPrecondition::Create,
                        KvRoleSummary::Owner,
                        KvRoleSummary::Owner,
                        true,
                        vault,
                        &master,
                    )?;
                }
                Ok(())
            };
            widen(session, &mut vault, 0)?;

            let catalog = session.data_catalog("owner", None, &mut vault)?;
            let large = catalog
                .entries
                .iter()
                .find(|entry| entry.metadata.path == "/alpha/beta/large")
                .expect("the addressed file is in the catalog");
            let large_version = large.metadata.version;
            let beta_version = catalog
                .entries
                .iter()
                .find(|entry| entry.metadata.path == "/alpha/beta")
                .expect("the addressed directory is in the catalog")
                .metadata
                .version;

            // Warm the node-ciphertext projection first: the first metadata
            // pass over a large file fetches its node, later passes serve it
            // locally, and that difference is not what this test measures.
            session.data_stat(
                "owner",
                None,
                "/alpha/beta/large",
                Some(large_version),
                &mut vault,
            )?;
            session.data_entry(
                "owner",
                None,
                "/alpha/beta/large",
                large_version,
                &mut vault,
            )?;
            session.data_chunk(
                "owner",
                None,
                "/alpha/beta/large",
                large_version,
                0,
                64 * 1024,
                &mut vault,
            )?;
            session.data_catalog("owner", None, &mut vault)?;

            let measure = |run: &mut dyn FnMut() -> Result<(), Error>| -> Result<u64, Error> {
                let before = server.metrics().requests_started;
                run()?;
                Ok(server.metrics().requests_started - before)
            };

            let narrow_stat = measure(&mut || {
                session.data_stat(
                    "owner",
                    None,
                    "/alpha/beta/large",
                    Some(large_version),
                    &mut vault,
                )?;
                Ok(())
            })?;
            let narrow_entry = measure(&mut || {
                session.data_entry(
                    "owner",
                    None,
                    "/alpha/beta/large",
                    large_version,
                    &mut vault,
                )?;
                Ok(())
            })?;
            let narrow_chunk = measure(&mut || {
                session.data_chunk(
                    "owner",
                    None,
                    "/alpha/beta/large",
                    large_version,
                    0,
                    64 * 1024,
                    &mut vault,
                )?;
                Ok(())
            })?;
            let narrow_catalog = measure(&mut || {
                session.data_catalog("owner", None, &mut vault)?;
                Ok(())
            })?;

            // A directory whose own generation the stat reports: the walk
            // descends into the final component when it names one.
            let narrow_directory_stat = measure(&mut || {
                let report = session.data_stat(
                    "owner",
                    None,
                    "/alpha/beta",
                    Some(beta_version),
                    &mut vault,
                )?;
                assert!(
                    report.directory.is_some(),
                    "a directory stat carries the directory's own generation"
                );
                Ok(())
            })?;

            widen(session, &mut vault, WIDTH)?;
            session.data_catalog("owner", None, &mut vault)?;

            let wide_stat = measure(&mut || {
                session.data_stat(
                    "owner",
                    None,
                    "/alpha/beta/large",
                    Some(large_version),
                    &mut vault,
                )?;
                Ok(())
            })?;
            let wide_entry = measure(&mut || {
                session.data_entry(
                    "owner",
                    None,
                    "/alpha/beta/large",
                    large_version,
                    &mut vault,
                )?;
                Ok(())
            })?;
            let wide_chunk = measure(&mut || {
                session.data_chunk(
                    "owner",
                    None,
                    "/alpha/beta/large",
                    large_version,
                    0,
                    64 * 1024,
                    &mut vault,
                )?;
                Ok(())
            })?;
            let wide_catalog = measure(&mut || {
                session.data_catalog("owner", None, &mut vault)?;
                Ok(())
            })?;
            let wide_directory_stat = measure(&mut || {
                session.data_stat("owner", None, "/alpha/beta", Some(beta_version), &mut vault)?;
                Ok(())
            })?;

            println!(
                "adapter requests with {WIDTH} then {} unrelated directories: \
                 stat={narrow_stat}/{wide_stat} entry={narrow_entry}/{wide_entry} \
                 chunk={narrow_chunk}/{wide_chunk} \
                 directory-stat={narrow_directory_stat}/{wide_directory_stat} \
                 catalog={narrow_catalog}/{wide_catalog}",
                2 * WIDTH
            );
            assert_eq!(narrow_stat, wide_stat, "stat walks only its own path");
            assert_eq!(narrow_entry, wide_entry, "entry walks only its own path");
            assert_eq!(narrow_chunk, wide_chunk, "chunk walks only its own path");
            assert_eq!(
                narrow_directory_stat, wide_directory_stat,
                "a directory stat walks only its own path"
            );
            // The catalog keeps the complete traversal, so it is the baseline
            // the three addressed reads used to pay: one directory read and
            // one listing per unrelated directory.
            assert_eq!(
                wide_catalog - narrow_catalog,
                2 * WIDTH as u64,
                "each unrelated directory adds a directory read and a listing"
            );
            assert!(
                wide_stat < narrow_catalog && wide_entry < narrow_catalog,
                "an addressed read costs less than a complete traversal: \
                 stat={wide_stat} entry={wide_entry} catalog={narrow_catalog}"
            );

            // An absent path reports what it reported when the same helpers
            // ran against a whole-namespace projection. A missing leaf is a
            // conflict; a missing directory component is an invalid path.
            for (path, version) in [("/alpha/beta/missing", 9), ("/missing", 1)] {
                assert!(
                    matches!(
                        session.data_stat("owner", None, path, Some(version), &mut vault),
                        Err(Error::KvConflict)
                    ),
                    "stat {path}"
                );
                assert!(
                    matches!(
                        session.data_entry("owner", None, path, version, &mut vault),
                        Err(Error::KvConflict)
                    ),
                    "entry {path}"
                );
                assert!(
                    matches!(
                        session.data_chunk("owner", None, path, version, 0, 8, &mut vault),
                        Err(Error::KvConflict)
                    ),
                    "chunk {path}"
                );
            }
            for path in ["/absent/leaf", "/alpha/absent/leaf"] {
                assert!(
                    matches!(
                        session.data_stat("owner", None, path, Some(1), &mut vault),
                        Err(Error::InvalidKvPath("directory component does not exist"))
                    ),
                    "stat {path}"
                );
                assert!(
                    matches!(
                        session.data_entry("owner", None, path, 1, &mut vault),
                        Err(Error::InvalidKvPath("directory component does not exist"))
                    ),
                    "entry {path}"
                );
                assert!(
                    matches!(
                        session.data_chunk("owner", None, path, 1, 0, 8, &mut vault),
                        Err(Error::InvalidKvPath("directory component does not exist"))
                    ),
                    "chunk {path}"
                );
            }
            // A component that exists but is not a directory is still refused
            // where it is walked, not where the rest of the store is read.
            assert!(matches!(
                session.data_entry("owner", None, "/alpha/beta/large/leaf", 1, &mut vault),
                Err(Error::InvalidKvPath("path component is not a directory"))
            ));
            assert!(matches!(
                session.data_stat("owner", None, "/alpha/beta/large/leaf", Some(1), &mut vault),
                Err(Error::InvalidKvPath("path component is not a directory"))
            ));
            // An existing path at the wrong version stays a conflict.
            assert!(matches!(
                session.data_entry(
                    "owner",
                    None,
                    "/alpha/beta/large",
                    large_version + 1,
                    &mut vault
                ),
                Err(Error::KvConflict)
            ));
            // The root still resolves with no components to walk.
            assert!(session
                .data_stat("owner", None, "/", None, &mut vault)?
                .directory
                .is_some());
            Ok::<_, Error>(())
        })
        .unwrap();
}

/// The team branch of the same three reads. A team namespace is reached
/// through `resolve_team_kv_path` rather than `resolve_user_kv_path`, so it
/// is measured separately; its root has to be created by the write adapter,
/// which is the only path that initializes a team KV root.
#[test]
fn addressed_team_reads_cost_one_path_and_still_report_an_absent_path_as_a_conflict() {
    use foks_client_app::{DataWriteKind as Kind, DataWriteSpec, DataWriteStatus as Status};

    const TEAM_WIDTH: usize = 4;

    let issued_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let handle = |byte| foks_client_app::SubmissionHandle::new(issued_at, [byte; 16]);

    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let root = state.join("root.der");
    environment.write_probe_root(&root).unwrap();
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            label: None,
            probe: format!(
                "localhost:{}",
                environment.addresses().unwrap().probe.port()
            ),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    let credentials = ClientCredentials::open(&state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            let master = credentials.master_key()?;
            session.probe_and_pin()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "teampathreader",
                "device",
                "owner@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            session.create_named_team("owner", "group", "datateam", &mut vault, &master)?;
            let team_id = session.resolve_data_team("owner", "datateam", &mut vault)?;

            let spec = |kind, path: &str, body: &[u8], mkdir_p| DataWriteSpec {
                kind,
                path: path.into(),
                destination: None,
                team_selector: Some("datateam".into()),
                overwrite: false,
                mkdir_p,
                recursive: false,
                body_length: body.len() as u64,
                body_hash: foks_crypto::kv_adapter_body_hash(body),
            };
            let write = |session: &foks_client_app::CheckedProfileSession<'_>,
                         vault: &mut AccountVault<'_>,
                         byte: u8,
                         spec: DataWriteSpec,
                         body: &[u8]|
             -> Result<(), Error> {
                session.prepare_data_write("owner", handle(byte), spec, vault, &master)?;
                let outcome = session.execute_data_write(
                    "owner",
                    handle(byte),
                    &mut { body },
                    vault,
                    &master,
                )?;
                assert_eq!(outcome.status, Status::Committed);
                Ok(())
            };

            let payload = vec![0x5c; LARGE];
            write(
                session,
                &mut vault,
                1,
                spec(Kind::Put, "/alpha/beta/large", &payload, true),
                &payload,
            )?;
            let widen = |session: &foks_client_app::CheckedProfileSession<'_>,
                         vault: &mut AccountVault<'_>,
                         from: usize|
             -> Result<(), Error> {
                for index in from..from + TEAM_WIDTH {
                    write(
                        session,
                        vault,
                        0x10 + index as u8,
                        spec(Kind::Mkdir, &format!("/dir-{index:03}"), b"", false),
                        b"",
                    )?;
                }
                Ok(())
            };
            widen(session, &mut vault, 0)?;

            let team = Some(team_id.as_str());
            let catalog = session.data_catalog("owner", team, &mut vault)?;
            let version = catalog
                .entries
                .iter()
                .find(|entry| entry.metadata.path == "/alpha/beta/large")
                .expect("the addressed team file is in the catalog")
                .metadata
                .version;

            // Warm the node-ciphertext projection, as in the personal case.
            session.data_stat(
                "owner",
                team,
                "/alpha/beta/large",
                Some(version),
                &mut vault,
            )?;
            session.data_entry("owner", team, "/alpha/beta/large", version, &mut vault)?;
            session.data_chunk(
                "owner",
                team,
                "/alpha/beta/large",
                version,
                0,
                64 * 1024,
                &mut vault,
            )?;

            let measure = |run: &mut dyn FnMut() -> Result<(), Error>| -> Result<u64, Error> {
                let before = server.metrics().requests_started;
                run()?;
                Ok(server.metrics().requests_started - before)
            };
            let counts = |session: &foks_client_app::CheckedProfileSession<'_>,
                          vault: &mut AccountVault<'_>|
             -> Result<(u64, u64, u64), Error> {
                let stat = measure(&mut || {
                    session.data_stat("owner", team, "/alpha/beta/large", Some(version), vault)?;
                    Ok(())
                })?;
                let entry = measure(&mut || {
                    session.data_entry("owner", team, "/alpha/beta/large", version, vault)?;
                    Ok(())
                })?;
                let chunk = measure(&mut || {
                    session.data_chunk(
                        "owner",
                        team,
                        "/alpha/beta/large",
                        version,
                        0,
                        64 * 1024,
                        vault,
                    )?;
                    Ok(())
                })?;
                Ok((stat, entry, chunk))
            };
            let narrow = counts(session, &mut vault)?;
            widen(session, &mut vault, TEAM_WIDTH)?;
            session.data_catalog("owner", team, &mut vault)?;
            let wide = counts(session, &mut vault)?;

            println!(
                "team adapter requests with {TEAM_WIDTH} then {} unrelated directories: \
                 stat={}/{} entry={}/{} chunk={}/{}",
                2 * TEAM_WIDTH,
                narrow.0,
                wide.0,
                narrow.1,
                wide.1,
                narrow.2,
                wide.2
            );
            assert_eq!(narrow, wide, "a team read walks only its own path");

            assert!(matches!(
                session.data_stat("owner", team, "/alpha/beta/missing", Some(1), &mut vault),
                Err(Error::KvConflict)
            ));
            assert!(matches!(
                session.data_entry("owner", team, "/alpha/beta/missing", 1, &mut vault),
                Err(Error::KvConflict)
            ));
            assert!(matches!(
                session.data_chunk("owner", team, "/alpha/beta/missing", 1, 0, 8, &mut vault),
                Err(Error::KvConflict)
            ));
            assert!(matches!(
                session.data_entry("owner", team, "/absent/leaf", 1, &mut vault),
                Err(Error::InvalidKvPath("directory component does not exist"))
            ));
            Ok::<_, Error>(())
        })
        .unwrap();
}
