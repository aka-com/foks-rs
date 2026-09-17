//! Native counterpart of the durable per-profile import gate. Ordinary checkpoint
//! reconciliation cannot remove this record after a forward SQLite modification.
use crate::{checkpoint::NativeManifestStore, Error, Result};
use foks_client_db::ImportReadiness;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
pub(crate) const PREFIX: &str = "import-readiness.";
#[derive(Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeReadiness {
    pub version: u32,
    pub archive_id: [u8; 16],
    pub attempt: [u8; 32],
    pub accounts_digest: [u8; 32],
}
pub(crate) fn key(profile: &str) -> Result<String> {
    crate::validate_name(profile)?;
    Ok(format!("{PREFIX}{profile}"))
}
impl NativeReadiness {
    pub fn from_state(state: &ImportReadiness) -> Result<Self> {
        let mut accounts = state
            .accounts
            .iter()
            .map(|a| (a.alias.as_str(), a.kind as u8))
            .collect::<Vec<_>>();
        accounts.sort();
        let digest = Sha256::digest(serde_json::to_vec(&accounts)?).into();
        Ok(Self {
            version: 1,
            archive_id: state.archive_id,
            attempt: state.attempt,
            accounts_digest: digest,
        })
    }
}
pub(super) fn validate(
    profile: &str,
    state: Option<&ImportReadiness>,
    native: &NativeManifestStore,
) -> Result<Option<String>> {
    let key = key(profile)?;
    match (state, native.records.get(&key)) {
        (Some(state), Some(bytes)) if state.required => {
            let record: NativeReadiness = serde_json::from_slice(bytes)?;
            if record != NativeReadiness::from_state(state)? {
                return Err(Error::StateRecoveryRequired);
            }
            Ok(Some(key))
        }
        (None, None) => Ok(None),
        (Some(state), None) if !state.required => Ok(None),
        _ => Err(Error::StateRecoveryRequired),
    }
}

/// Readiness can be locally false after interruption, but native authority remains.
/// Reverification still requires the exact authenticated attempt and account set.
pub(crate) fn validate_for_proof(
    credentials: &crate::ClientCredentials,
    session: &crate::ProfileSession,
) -> Result<foks_client_db::ImportReadiness> {
    if credentials.backend != crate::CredentialBackend::Native {
        return Err(Error::PortabilityUnsupported);
    }
    let state = foks_client_db::HardStateStore::open(&session.paths.hard_database)?
        .import_readiness()?
        .ok_or(Error::InvalidConfig(
            "profile is not an imported verification attempt",
        ))?;
    credentials.with_native_manifest(|native| {
        let bytes = native
            .records
            .get(&key(&session.profile.name)?)
            .ok_or(Error::InvalidConfig("profile is already verified"))?;
        let record: NativeReadiness = serde_json::from_slice(bytes)?;
        if record != NativeReadiness::from_state(&state)? {
            return Err(Error::StateRecoveryRequired);
        }
        Ok(())
    })?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::CheckpointStore as _;
    #[test]
    fn native_gate_survives_forward_database_flag_changes_and_all_factories_deny_use() {
        if std::env::var_os("FOKS_TEST_NATIVE_PORTABILITY").is_none() {
            return;
        }
        let mut fixture = crate::test_support::AccountFixture::start_native();
        let mut profile = fixture.registry.profile("local").unwrap().clone();
        profile.name = "second".into();
        fixture.registry.add(profile).unwrap();
        let second = crate::ProfileSession::open(&fixture.registry, "second").unwrap();
        fixture
            .credentials
            .with_checked_session(&second, |_| Ok::<_, Error>(()))
            .unwrap();
        fixture.run(|s, _, _| {
            let mut db = foks_client_db::HardStateStore::open(&s.paths.hard_database)?;
            db.install_import_readiness([1; 16], [2; 32], &[])?;
            Ok(())
        });
        let session = crate::ProfileSession::open(&fixture.registry, "local").unwrap();
        {
            let _lock =
                crate::runtime::NativeManifestLock::acquire(&fixture.credentials.state_id).unwrap();
            let mut store =
                foks_keystore::NativeCredentialStore::open(&fixture.credentials.state_id).unwrap();
            let mut manifest = NativeManifestStore::decode(
                &store
                    .get(crate::checkpoint::NATIVE_MANIFEST_RECORD)
                    .unwrap(),
            )
            .unwrap();
            let db = foks_client_db::HardStateStore::open(&session.paths.hard_database).unwrap();
            manifest
                .put(
                    &key("local").unwrap(),
                    &serde_json::to_vec(
                        &NativeReadiness::from_state(&db.import_readiness().unwrap().unwrap())
                            .unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap();
            manifest.generation += 1;
            store
                .put(
                    crate::checkpoint::NATIVE_MANIFEST_RECORD,
                    &manifest.encode().unwrap(),
                )
                .unwrap();
        }
        // An attacker can advance SQLite's revision while clearing its flag;
        // ordinary checkpoint reconciliation must not grant write permission.
        {
            let mut db =
                foks_client_db::HardStateStore::open(&session.paths.hard_database).unwrap();
            db.complete_import_verification([2; 32], &[]).unwrap();
        }
        let called = std::cell::Cell::new(false);
        assert!(matches!(
            fixture.credentials.with_checked_session(&session, |_| {
                called.set(true);
                Ok::<_, Error>(())
            }),
            Err(Error::ImportVerificationRequired)
        ));
        assert!(matches!(
            fixture.credentials.try_with_checked_session(&session, |_| {
                called.set(true);
                Ok::<_, Error>(())
            }),
            Err(Error::ImportVerificationRequired)
        ));
        assert!(matches!(
            fixture
                .credentials
                .with_checked_sessions(&session, &second, |_, _| {
                    called.set(true);
                    Ok::<_, Error>(())
                }),
            Err(Error::ImportVerificationRequired)
        ));
        fixture
            .credentials
            .with_import_session(&session, |_| {
                assert!(matches!(
                    fixture.credentials.with_checked_session(&session, |_| {
                        called.set(true);
                        Ok::<_, Error>(())
                    }),
                    Err(Error::ImportVerificationRequired)
                ));
                assert!(matches!(
                    fixture.credentials.try_with_checked_session(&session, |_| {
                        called.set(true);
                        Ok::<_, Error>(())
                    }),
                    Err(Error::ImportVerificationRequired)
                ));
                Ok(())
            })
            .unwrap();
        assert!(!called.get());
        drop(second);
        // Explicit destructive reset consumes the gated identity and its preview.
        fixture.credentials.reset_hard_state(&session).unwrap();
        fixture
            .credentials
            .with_checked_session(&session, |_| Ok::<_, Error>(()))
            .unwrap();
        drop(session);
        drop(fixture.stop_client());
    }
}
