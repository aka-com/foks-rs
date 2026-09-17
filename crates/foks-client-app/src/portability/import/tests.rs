use super::*;
use crate::portability::prepare_state_export;
fn enabled() -> bool {
    std::env::var_os("FOKS_TEST_NATIVE_PORTABILITY").is_some()
}
fn export(fixture: &crate::test_support::StoppedAccountFixture) -> (PathBuf, StateTransferKey) {
    let path = fixture
        .environment
        .client_path("transfer", "archive")
        .unwrap();
    crate::prepare_private_directory(path.parent().unwrap()).unwrap();
    let key = StateTransferKey::generate().unwrap();
    let preview = prepare_state_export(&fixture.root).unwrap();
    let digest = preview.report().unwrap().digest;
    preview
        .authorize(&digest)
        .unwrap()
        .export(&path, &key)
        .unwrap();
    (path, key)
}
fn cleanup(root: &Path) {
    let state = crate::checkpoint::inspect_state_file(root)
        .unwrap()
        .unwrap();
    let mut native = foks_keystore::NativeCredentialStore::open(&state.state_id).unwrap();
    native
        .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
        .unwrap();
}
#[test]
fn independent_imports_rekey_accounts_preserve_database_identity_and_deny_use() {
    if !enabled() {
        return;
    }
    let fixture = crate::test_support::AccountFixture::start_native();
    fixture.run(|s, v, k| s.create_account("work", "stateimport", "laptop", "", "", None, v, k));
    fixture.run(|s, v, k| s.create_named_team("work", "team", "importteam", v, k));
    fixture.run(|s, v, _| {
        let account = v.account("work")?;
        let host = s.pinned_host()?;
        let mut db = foks_client_db::HardStateStore::open(&s.paths.hard_database)?;
        let sample = foks_client_db::AdapterTimeSample {
            process_id: [9; 16],
            wall_seconds: crate::now_microseconds()? / 1_000_000,
            monotonic_seconds: 0,
        };
        assert!(matches!(
            db.check_adapter_admission(
                host.host_id().as_bytes(),
                account.credential.uid.as_bytes(),
                foks_proto::SubmissionHandle::new(1, [8; 16]),
                sample
            ),
            Err(foks_client_db::Error::AdapterExpired)
        ));
        assert!(
            db.adapter_clock(host.host_id().as_bytes(), account.credential.uid.as_bytes())?
                .unwrap()
                .reject_issued_before
                > 1
        );
        Ok(())
    });
    let fixture = fixture.stop_client();
    let original_rows = security_rows(&fixture.root.join("profiles/local/hard.sqlite3"));
    let original_id =
        foks_client_db::HardStateStore::open(&fixture.root.join("profiles/local/hard.sqlite3"))
            .unwrap()
            .metadata()
            .unwrap()
            .database_id;
    let (archive, key) = export(&fixture);
    let source = crate::portability::inspect_native_state(&fixture.root).unwrap();
    let mut ids = vec![fixture.state_id.clone()];
    for index in 0..2 {
        let dest = archive.parent().unwrap().join(format!("import-{index}"));
        if index == 1 {
            crate::prepare_private_directory(&dest).unwrap();
        }
        let report = import_state(&archive, &key, &dest).unwrap();
        assert!(report.verification_required);
        assert_eq!(
            security_rows(&dest.join("profiles/local/hard.sqlite3")),
            original_rows
        );
        assert_eq!(
            foks_client_db::HardStateStore::open(&dest.join("profiles/local/hard.sqlite3"))
                .unwrap()
                .metadata()
                .unwrap()
                .database_id,
            original_id
        );
        let source_master = crate::ClientCredentials::open(&fixture.root)
            .unwrap()
            .master_key()
            .unwrap();
        let dest_master = crate::ClientCredentials::open(&dest)
            .unwrap()
            .master_key()
            .unwrap();
        assert_ne!(*source_master, *dest_master);
        let credentials = crate::ClientCredentials::open(&dest).unwrap();
        assert!(!ids.contains(&credentials.state_id));
        ids.push(credentials.state_id.clone());
        let registry = crate::ProfileRegistry::open(&dest).unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        let db = foks_client_db::HardStateStore::open(&session.paths.hard_database).unwrap();
        assert!(db.requires_import_verification().unwrap());
        assert_eq!(db.import_readiness().unwrap().unwrap().accounts.len(), 1);
        let called = std::cell::Cell::new(false);
        assert!(matches!(
            credentials.with_checked_session(&session, |_| {
                called.set(true);
                Ok::<_, Error>(())
            }),
            Err(Error::ImportVerificationRequired)
        ));
        assert!(!called.get());
        drop(db);
        drop(session);
        drop(registry);
        drop(credentials);
        let inspected = crate::portability::inspect_native_state(&dest).unwrap();
        assert!(!inspected.exportable);
        assert_eq!(inspected.profiles.len(), source.profiles.len());
        assert!(!import_status(&dest).unwrap().unwrap().recovery_required);
        let registry = crate::ProfileRegistry::open(&dest).unwrap();
        let credentials = crate::ClientCredentials::open(&dest).unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        let report =
            crate::portability::verify_imported_profile(&credentials, &session, &BTreeMap::new())
                .unwrap();
        assert!(
            report.verified,
            "{}",
            serde_json::to_string(&report).unwrap()
        );
        credentials
            .with_checked_session(&session, |s| {
                let master = credentials.master_key()?;
                let mut store = foks_keystore::EncryptedFileSecretStore::open(
                    &s.paths.credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = crate::AccountVault::new(&mut store);
                s.put_kv_file(
                    "work",
                    &format!("/copy-{index}"),
                    &mut &b"independent write"[..],
                    false,
                    true,
                    &mut vault,
                    &master,
                )?;
                assert_eq!(
                    s.read_kv_file("work", &format!("/copy-{index}"), &mut vault)?,
                    b"independent write"
                );
                Ok::<_, Error>(())
            })
            .unwrap();
        drop(session);
        drop(credentials);
        drop(registry);
        // Completion history is not transferred as authority on a new export.
        let preview = prepare_state_export(&dest).unwrap();
        assert!(preview.report().unwrap().exportable);
        let digest = preview.report().unwrap().digest;
        preview
            .authorize(&digest)
            .unwrap()
            .export(
                archive.parent().unwrap().join(format!("reexport-{index}")),
                &key,
            )
            .unwrap();
        cleanup(&dest);
    }
    assert!(crate::ClientCredentials::open(&fixture.root).is_ok());
}
#[test]
fn import_recovery_at_each_durable_boundary() {
    if !enabled() {
        return;
    }
    let fixture = crate::test_support::AccountFixture::start_native().stop_client();
    let (archive, key) = export(&fixture);
    let mut points = Vec::new();
    let first = archive.parent().unwrap().join("baseline");
    run(&archive, &key, &first, false, &mut |point| {
        points.push(point);
        Ok(())
    })
    .unwrap();
    cleanup(&first);
    for (index, point) in points.iter().enumerate() {
        let dest = archive
            .parent()
            .unwrap()
            .join(format!("interrupted-{index}"));
        let mut count = 0;
        let result = run(&archive, &key, &dest, false, &mut |_| {
            let fail = count == index;
            count += 1;
            if fail {
                Err(Error::StateRecoveryRequired)
            } else {
                Ok(())
            }
        });
        assert!(result.is_err(), "{index}: {point}");
        let status = import_status(&dest).unwrap();
        if status
            .as_ref()
            .is_some_and(|s| s.recovery_required && !s.requires_archive)
        {
            recover_import(&dest).unwrap_or_else(|e| panic!("recover {index} {point}: {e:?}"));
        } else if status.as_ref().is_none_or(|s| s.requires_archive) {
            import_state(&archive, &key, &dest)
                .unwrap_or_else(|e| panic!("retry {index} {point}: {e:?}"));
        }
        assert!(
            crate::ClientCredentials::open(&dest).is_ok(),
            "{index}: {point}"
        );
        cleanup(&dest);
    }
}
#[test]
fn corrupt_archive_and_nonempty_destination_never_publish_native_state() {
    if !enabled() {
        return;
    }
    let fixture = crate::test_support::AccountFixture::start_native().stop_client();
    let (archive, key) = export(&fixture);
    let dest = archive.parent().unwrap().join("destination");
    let mut bytes = fs::read(&archive).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&archive, bytes).unwrap();
    assert!(import_state(&archive, &key, &dest).is_err());
    assert!(!dest.exists());
    assert!(import_status(&dest).unwrap().is_none());
    crate::prepare_private_directory(&dest).unwrap();
    fs::write(dest.join("keep"), b"keep").unwrap();
    assert!(import_state(&archive, &key, &dest).is_err());
    assert_eq!(fs::read(dest.join("keep")).unwrap(), b"keep");
}

#[test]
fn imported_hardware_and_bot_accounts_require_the_original_credentials() {
    if !enabled() {
        return;
    }
    use foks_yubi::{MockYubiProvider, Pin, SlotId, YubiProvider as _};
    let fixture = crate::test_support::AccountFixture::start_native();
    fixture.run(|s, v, k| s.create_account("work", "importbot", "laptop", "", "", None, v, k));
    let prepared =
        fixture.run(|s, v, k| s.prepare_bot_account("work", foks_proto::Role::OWNER, None, v, k));
    let id = std::array::from_fn(|i| {
        u8::from_str_radix(&prepared.operation_id[i * 2..i * 2 + 2], 16).unwrap()
    });
    fixture.run(|s, v, k| {
        s.bot_account_enrollment("work", id, crate::BotEnrollmentAction::Attempt, None, v, k)
    });
    let token = fixture
        .run(|s, v, k| {
            s.bot_account_enrollment("work", id, crate::BotEnrollmentAction::Export, None, v, k)
        })
        .exported
        .unwrap();
    fixture.run(|s, v, _| {
        s.load_bot_account("bot", &token, v)?;
        Ok(())
    });
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("import", 780, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    fixture.run(|s, v, k| {
        s.create_yubi_account(
            crate::YubiSignupInput {
                alias: "hardware".into(),
                username: "importyubi".into(),
                device_name: "owner key".into(),
                email: String::new(),
                invite: String::new(),
                passphrase: None,
                card,
                signing_slot: SlotId::new(0x82)?,
                pq_slot: SlotId::new(0x83)?,
                retry_configuration: None,
            },
            Pin::new("654321")?,
            &provider,
            v,
            k,
        )
    });
    let fixture = fixture.stop_client();
    let (archive, key) = export(&fixture);
    let dest = archive.parent().unwrap().join("hardware-copy");
    import_state(&archive, &key, &dest).unwrap();
    let registry = crate::ProfileRegistry::open(&dest).unwrap();
    let credentials = crate::ClientCredentials::open(&dest).unwrap();
    let session = crate::ProfileSession::open(&registry, "local").unwrap();
    let first =
        crate::portability::verify_imported_profile(&credentials, &session, &BTreeMap::new())
            .unwrap();
    assert!(!first.verified);
    assert!(first
        .accounts
        .iter()
        .any(|r| r.alias == "bot" && r.status == "token-required"));
    assert!(first
        .accounts
        .iter()
        .any(|r| r.alias == "hardware" && r.status == "hardware-required"));
    let secrets = BTreeMap::from([
        (
            "bot".into(),
            crate::portability::VerificationSecret::BotToken(&token),
        ),
        (
            "hardware".into(),
            crate::portability::VerificationSecret::Yubi {
                pin: &pin,
                provider: &provider,
            },
        ),
    ]);
    let report =
        crate::portability::verify_imported_profile(&credentials, &session, &secrets).unwrap();
    assert!(
        report.verified,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    drop(session);
    drop(credentials);
    drop(registry);
    cleanup(&dest);
}

fn security_rows(path: &Path) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let db = rusqlite::Connection::open(path).unwrap();
    let tables=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT GLOB 'sqlite_*' AND name NOT IN ('hard_state_metadata','import_accounts','import_readiness') ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let mut result = BTreeMap::new();
    for table in tables {
        assert!(table
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        let mut query = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns = query.column_count();
        let rows = query
            .query_map([], |r| {
                (0..columns)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        result.insert(table, rows);
    }
    result
}

#[test]
fn imported_oidc_requires_reauthentication_and_never_allows_signup_replay() {
    if !enabled() {
        return;
    }
    use crate::portability::{
        reauthenticate_imported_account, verify_imported_profile,
        ImportReauthenticationAction as Action,
    };
    let idp = foks_server_testkit::oidc::TestOidcProvider::start();
    let fixture = crate::test_support::AccountFixture::start_native_oidc(&idp);
    let http = foks_oidc::ProviderHttp::new(foks_oidc::NetworkPolicy::loopback_test()).unwrap();
    let started = fixture
        .run(|s, v, k| s.begin_account_sso("work", foks_proto::SsoPurpose::Signup, v, k, &http));
    let flow = |r: &crate::SsoReport| {
        std::array::from_fn(|i| {
            u8::from_str_radix(&r.operation_id.as_ref().unwrap()[i * 2..i * 2 + 2], 16).unwrap()
        })
    };
    let original = flow(&started);
    // The server fixture clock is frozen, while provider JWTs use wall time.
    fixture
        .environment
        .set_clock(crate::now_microseconds().unwrap());
    idp.complete(started.browser_url.as_ref().unwrap());
    fixture.run(|s, v, k| s.account_sso("work", original, crate::SsoAction::Poll, v, k, &http));
    fixture.run(|s, v, k| {
        s.finish_account_sso_signup(
            "work",
            original,
            crate::SsoSignupInput {
                device_name: "imported laptop".into(),
                invite: String::new(),
                passphrase: None,
            },
            v,
            k,
            &http,
        )
    });
    let (uid, device, sequence) = fixture.run(|s, v, _| {
        let a = v.account("work")?;
        let auth = s
            .client
            .authenticate_and_pin(&s.pinned_host()?, &a.credential)?;
        Ok((
            a.credential.uid.as_bytes().to_vec(),
            a.credential.public_material()?.id.as_bytes().to_vec(),
            auth.verified.chain_seqno(),
        ))
    });
    let fixture = fixture.stop_client();
    let (archive, key) = export(&fixture);
    let dest = archive.parent().unwrap().join("oidc-copy");
    import_state(&archive, &key, &dest).unwrap();
    fixture
        ._server
        .writer_handle()
        .call(move |db| {
            let old = db.sso_access(&uid)?.unwrap();
            let mut next = old.clone();
            next.state = foks_server_db::SsoAccessState::ReauthenticationRequired;
            next.revision += 1;
            db.sso_transition_access(
                &old,
                &next,
                &device,
                sequence,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            )?;
            Ok(())
        })
        .unwrap();
    let registry = crate::ProfileRegistry::open(&dest).unwrap();
    let credentials = crate::ClientCredentials::open(&dest).unwrap();
    let session = crate::ProfileSession::open(&registry, "local").unwrap();
    let first = verify_imported_profile(&credentials, &session, &BTreeMap::new()).unwrap();
    assert!(!first.verified);
    assert_eq!(first.accounts[0].status, "reauthentication-required");
    assert!(reauthenticate_imported_account(
        &credentials,
        &session,
        "work",
        Action::Finish(original),
        None,
        &http
    )
    .is_err());
    let started =
        reauthenticate_imported_account(&credentials, &session, "work", Action::Begin, None, &http)
            .unwrap();
    let id = flow(&started);
    // The server fixture clock is frozen, while provider JWTs use wall time.
    fixture
        .environment
        .set_clock(crate::now_microseconds().unwrap());
    idp.complete(started.browser_url.as_ref().unwrap());
    reauthenticate_imported_account(
        &credentials,
        &session,
        "work",
        Action::Poll(id),
        None,
        &http,
    )
    .unwrap();
    let done = reauthenticate_imported_account(
        &credentials,
        &session,
        "work",
        Action::Finish(id),
        None,
        &http,
    )
    .unwrap();
    assert!(done.service_access);
    assert!(credentials.requires_import_verification(&session).unwrap());
    assert!(
        verify_imported_profile(&credentials, &session, &BTreeMap::new())
            .unwrap()
            .verified
    );
    drop(session);
    drop(credentials);
    drop(registry);
    cleanup(&dest);
}

#[test]
fn later_source_revocation_keeps_an_old_import_gated() {
    if !enabled() {
        return;
    }
    let fixture = crate::test_support::AccountFixture::start_native();
    fixture.run(|s, v, k| s.create_account("work", "importrevoked", "first", "", "", None, v, k));
    fixture.run(|s, v, k| s.provision_owner_device("work", "survivor", "second", 1, v, k));
    let revoked = fixture.run(|_, v, _| {
        Ok(crate::hex(
            v.account("work")?
                .credential
                .public_material()?
                .id
                .as_bytes(),
        ))
    });
    let fixture = fixture.stop_client();
    let (archive, key) = export(&fixture);
    {
        let registry = crate::ProfileRegistry::open(&fixture.root).unwrap();
        let credentials = crate::ClientCredentials::open(&fixture.root).unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        credentials
            .with_checked_session(&session, |s| {
                let master = credentials.master_key()?;
                let mut store = foks_keystore::EncryptedFileSecretStore::open(
                    &s.paths.credential_store,
                    crate::derive_vault_key(&master),
                )?;
                s.remove_software_device(
                    "survivor",
                    &revoked,
                    &mut crate::AccountVault::new(&mut store),
                    &master,
                )?;
                Ok::<_, Error>(())
            })
            .unwrap();
    }
    let dest = archive.parent().unwrap().join("revoked-copy");
    import_state(&archive, &key, &dest).unwrap();
    let registry = crate::ProfileRegistry::open(&dest).unwrap();
    let credentials = crate::ClientCredentials::open(&dest).unwrap();
    let session = crate::ProfileSession::open(&registry, "local").unwrap();
    let report =
        crate::portability::verify_imported_profile(&credentials, &session, &BTreeMap::new())
            .unwrap();
    assert!(!report.verified);
    assert!(
        report
            .accounts
            .iter()
            .any(|a| a.alias == "work" && a.status == "revoked"),
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert!(credentials.requires_import_verification(&session).unwrap());
    assert!(matches!(
        credentials.try_with_checked_session(&session, |_| Ok::<_, Error>(())),
        Err(Error::ImportVerificationRequired)
    ));
    drop(session);
    drop(credentials);
    drop(registry);
    cleanup(&dest);
}

#[test]
fn native_cli_transcript_keeps_transfer_key_out_of_output() {
    if !enabled() {
        return;
    }
    let fixture = crate::test_support::AccountFixture::start_native();
    fixture.run(|s, v, k| s.create_account("work", "clitransfer", "laptop", "", "", None, v, k));
    let fixture = fixture.stop_client();
    let parent = fixture.environment.client_path("cli", "transfer").unwrap();
    crate::prepare_private_directory(&parent).unwrap();
    let archive = parent.join("archive");
    let key = parent.join("key");
    let dest = parent.join("imported");
    let binary = std::env::var_os("FOKS_PORTABILITY_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/foks-rs")
        });
    assert!(
        binary.is_file(),
        "build foks-cli before native CLI acceptance, or set FOKS_PORTABILITY_CLI"
    );
    let run = |root: &Path, args: &[&std::ffi::OsStr]| {
        let output = std::process::Command::new(&binary)
            .arg("--state-dir")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let exported = run(
        &fixture.root,
        &[
            "state".as_ref(),
            "export".as_ref(),
            "--yes".as_ref(),
            "--output".as_ref(),
            archive.as_os_str(),
            "--key-output".as_ref(),
            key.as_os_str(),
        ],
    );
    let secret = crate::portability::read_private_secret_file(&key, 128).unwrap();
    let imported = run(
        &dest,
        &[
            "state".as_ref(),
            "import".as_ref(),
            "--input".as_ref(),
            archive.as_os_str(),
            "--key-file".as_ref(),
            key.as_os_str(),
        ],
    );
    let status = run(&dest, &["state".as_ref(), "status".as_ref()]);
    let verified = run(&dest, &["state".as_ref(), "verify-online".as_ref()]);
    for output in [exported, imported, status, verified] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(secret.as_str()));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(secret.as_str()));
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    }
    assert!(archive.is_file() && key.is_file());
    cleanup(&dest);
}

#[test]
fn account_rekey_interruptions_and_forged_retry_metadata_preserve_authority() {
    if !enabled() {
        return;
    }
    let fixture = crate::test_support::AccountFixture::start_native();
    fixture.run(|s, v, k| s.create_account("work", "rekeyrecovery", "laptop", "", "", None, v, k));
    let fixture = fixture.stop_client();
    let (archive, key) = export(&fixture);
    for (index, boundary) in [
        "before-vault-rekey",
        "after-vault-rekey",
        "before-rekey-remove",
        "after-rekey-remove",
        "after-rekey-rename",
        "after-readiness-write",
        "after-import-checkpoint",
    ]
    .into_iter()
    .enumerate()
    {
        let dest = archive.parent().unwrap().join(format!("rekey-{index}"));
        let mut reached = false;
        assert!(run(&archive, &key, &dest, false, &mut |point| {
            if point == boundary {
                reached = true;
                Err(Error::StateRecoveryRequired)
            } else {
                Ok(())
            }
        })
        .is_err());
        assert!(reached, "missing boundary {boundary}");
        assert!(import_status(&dest).unwrap().unwrap().requires_archive);
        let guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&dest)).unwrap();
        let locator = locator_path(&guard.base, &dest);
        let original = fs::read(&locator).unwrap();
        let mut forged: Locator = serde_json::from_slice(&original).unwrap();
        assert!(forged.marker.identity.staging.exists());
        forged.marker.identity_mac[0] ^= 1;
        fs::write(&locator, serde_json::to_vec(&forged).unwrap()).unwrap();
        drop(guard);
        // Neither keyless recovery nor possession of the archive authorizes a
        // locator with a different authentication tag, even before native publish.
        assert!(recover_import(&dest).is_err());
        assert!(import_state(&archive, &key, &dest).is_err());
        assert!(forged.marker.identity.staging.exists());
        assert!(!dest.exists());
        fs::write(&locator, &original).unwrap();
        assert!(import_state(&archive, &StateTransferKey::generate().unwrap(), &dest).is_err());
        assert_eq!(fs::read(&locator).unwrap(), original);
        import_state(&archive, &key, &dest).unwrap();
        let credentials = crate::ClientCredentials::open(&dest).unwrap();
        let registry = crate::ProfileRegistry::open(&dest).unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        assert!(credentials.requires_import_verification(&session).unwrap());
        drop(session);
        drop(registry);
        drop(credentials);
        cleanup(&dest);
    }
}
