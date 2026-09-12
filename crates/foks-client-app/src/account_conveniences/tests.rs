use super::*;
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;
struct Fixture {
    _environment: TestEnvironment,
    _server: foks_server_testkit::InProcessServer,
    credentials: ClientCredentials,
    registry: ProfileRegistry,
}
impl Fixture {
    fn start() -> Self {
        let environment = TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let state = environment.client_path("sso-app", "state").unwrap();
        let root = environment.client_path("sso-app", "root.der").unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                probe: format!(
                    "localhost:{}",
                    environment.addresses().unwrap().probe.port()
                ),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let f = Self {
            _environment: environment,
            _server: server,
            credentials,
            registry,
        };
        f.run(|s, _, _| {
            s.probe_and_pin()?;
            Ok(())
        });
        f
    }
    fn run<T>(
        &self,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> T {
        // Reopen the profile, vault and protected store for every foreground action.
        let session = ProfileSession::open(&self.registry, "local").unwrap();
        self.credentials
            .with_checked_session(&session, |s| {
                let master = self.credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &s.paths.credential_store,
                    derive_vault_key(&master),
                )?;
                op(s, &mut AccountVault::new(&mut store), &master)
            })
            .unwrap()
    }
}
fn id(p: &RenameReport) -> [u8; 16] {
    std::array::from_fn(|i| u8::from_str_radix(&p.operation_id[i * 2..i * 2 + 2], 16).unwrap())
}
#[test]
fn rename_reopens_refreshes_uid_labels_and_preserves_aliases() {
    let f = Fixture::start();
    f.run(|s, v, k| s.create_account("work", "renamealice", "laptop", "", "", None, v, k));
    f.run(|s, v, k| s.create_account("other", "renamebob", "laptop", "", "", None, v, k));
    let uid = f.run(|_, v, _| {
        let a = v.account("work")?;
        v.commit_created("second", &a.username, &a.credential)?;
        Ok(a.credential.uid)
    });
    let p = f.run(|s, v, k| {
        s.rename_account(
            "work",
            RenameAction::Prepare("renamecarol".into()),
            None,
            v,
            k,
        )
    });
    assert_eq!(p.state, "prepared");
    f.run(|s, v, k| {
        assert!(s
            .rename_account("other", RenameAction::Attempt(id(&p)), None, v, k)
            .is_err());
        assert_eq!(v.account("work")?.username, "renamealice");
        assert_eq!(s.account_renames("work", v)?.len(), 1);
        Ok(())
    });
    let done = f.run(|s, v, k| s.rename_account("work", RenameAction::Attempt(id(&p)), None, v, k));
    assert_eq!(done.state, "complete");
    f.run(|s, v, k| {
        for alias in ["work", "second"] {
            let a = v.account(alias)?;
            assert_eq!(a.credential.uid, uid);
            assert_eq!(a.username, "renamecarol");
        }
        assert_eq!(v.account("other")?.username, "renamebob");
        assert_eq!(
            s.rename_account("work", RenameAction::Status(id(&p)), None, v, k)?
                .state,
            "complete"
        );
        Ok(())
    });
    let display = f.run(|s, v, k| {
        s.rename_account(
            "work",
            RenameAction::Prepare("RenameCarol".into()),
            None,
            v,
            k,
        )
    });
    assert_eq!(
        f.run(|s, v, k| s.rename_account("work", RenameAction::Attempt(id(&display)), None, v, k))
            .state,
        "complete"
    );
    f.run(|_, v, _| {
        assert_eq!(v.account("second")?.username, "RenameCarol");
        Ok(())
    });
    let cancel = f.run(|s, v, k| {
        s.rename_account(
            "work",
            RenameAction::Prepare("renamedan".into()),
            None,
            v,
            k,
        )
    });
    for _ in 0..2 {
        assert_eq!(
            f.run(|s, v, k| s.rename_account(
                "work",
                RenameAction::Cancel(id(&cancel)),
                None,
                v,
                k
            ))
            .state,
            "rejected"
        );
    }
}
#[test]
fn hardware_rename_defers_without_pin_and_signs_with_selected_parent() {
    use foks_yubi::{MockYubiProvider, Pin, SlotId, YubiProvider as _};
    let f = Fixture::start();
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("rename", 778, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    f.run(|s, v, k| {
        s.create_yubi_account(
            YubiSignupInput {
                alias: "key".into(),
                username: "renameyubi".into(),
                device_name: "key".into(),
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
    f.run(|s, v, k| {
        assert!(matches!(
            s.rename_account(
                "key",
                RenameAction::Prepare("renamedkey".into()),
                None,
                v,
                k
            ),
            Err(Error::YubiUnlockRequired(_))
        ));
        Ok(())
    });
    let parent = f.run(|_, v, _| Ok(provider.open(&v.yubi_account("key")?.locator, Some(&pin))?));
    let p = f.run(|s, v, k| {
        s.rename_account(
            "key",
            RenameAction::Prepare("renamedkey".into()),
            Some(parent.as_ref()),
            v,
            k,
        )
    });
    let deferred =
        f.run(|s, v, k| s.rename_account("key", RenameAction::Attempt(id(&p)), None, v, k));
    assert!(deferred.hardware_required);
    assert_eq!(deferred.state, "prepared");
    assert_eq!(
        f.run(|s, v, k| s.rename_account(
            "key",
            RenameAction::Attempt(id(&p)),
            Some(parent.as_ref()),
            v,
            k
        ))
        .state,
        "complete"
    );
    f.run(|_, v, _| {
        assert_eq!(v.yubi_account("key")?.username, "renamedkey");
        Ok(())
    });
}
