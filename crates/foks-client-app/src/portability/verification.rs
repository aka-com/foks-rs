//! Narrow proof-only import authority. No scheduler or mutation replay is invoked.
use super::readiness::{self, NativeReadiness};
use crate::checkpoint::CheckpointStore as _;
use crate::{AccountVault, ClientCredentials, Error, ProfileSession, Result};
use foks_client_db::{HardStateStore, ImportAccountKind, VerifiedImportAccount};
use std::collections::BTreeMap;

/// Secrets stay borrowed in the native caller and are never serialized.
pub enum VerificationSecret<'a> {
    BotToken(&'a str),
    Yubi {
        pin: &'a foks_yubi::Pin,
        provider: &'a dyn foks_yubi::YubiProvider,
    },
}
#[derive(serde::Serialize)]
pub struct AccountVerificationReport {
    pub alias: String,
    pub status: &'static str,
}
#[derive(serde::Serialize)]
pub struct ProfileVerificationReport {
    pub profile: String,
    pub verified: bool,
    pub accounts: Vec<AccountVerificationReport>,
}

pub fn verify_imported_profile(
    credentials: &ClientCredentials,
    session: &ProfileSession,
    secrets: &BTreeMap<String, VerificationSecret<'_>>,
) -> Result<ProfileVerificationReport> {
    // An already completed attempt still goes through ordinary rollback checks.
    if !HardStateStore::open(&session.paths.hard_database)?.requires_import_verification()? {
        let absent = credentials.with_native_manifest(|m| {
            Ok(!m
                .records
                .contains_key(&readiness::key(&session.profile.name)?))
        })?;
        if absent {
            return credentials.with_checked_session(session, |_| {
                Ok(ProfileVerificationReport {
                    profile: session.profile.name.clone(),
                    verified: true,
                    accounts: Vec::new(),
                })
            });
        }
    }
    credentials.with_import_session(session, |checked| {
        let state = readiness::validate_for_proof(credentials, session)?;
        let master = credentials.master_key()?;
        let mut store = foks_keystore::EncryptedFileSecretStore::open(
            &session.paths.credential_store,
            crate::derive_vault_key(&master),
        )?;
        let mut vault = AccountVault::new(&mut store);
        let host = checked.pinned_host()?;
        let mut reports = Vec::new();
        let mut proofs = Vec::new();
        let mut current_users = std::collections::BTreeSet::new();
        for account in &state.accounts {
            let result = (|| {
                let authenticated = match account.kind {
                    ImportAccountKind::Software => {
                        let loaded = vault.account(&account.alias)?;
                        let auth = checked
                            .client
                            .authenticate_and_pin(&host, &loaded.credential)?;
                        checked.client.discover_local_team_graph(
                            &host,
                            &loaded.credential,
                            &auth.verified,
                            &auth.puks,
                        )?;
                        auth
                    }
                    ImportAccountKind::Bot => {
                        let Some(VerificationSecret::BotToken(token)) = secrets.get(&account.alias)
                        else {
                            return Err(Error::BotTokenLocked);
                        };
                        let selected = vault
                            .bot_selection(&account.alias)?
                            .ok_or(Error::AccountMissing)?;
                        let token =
                            foks_crypto::BotToken::import(token).map_err(|_| Error::BotToken)?;
                        let credential = checked.client.load_bot_token(&host, &token)?;
                        if selected.host_id != host.host_id().as_bytes()
                            || selected.uid != credential.uid.as_bytes()
                            || selected.device_id != credential.public_material()?.id.as_bytes()
                        {
                            return Err(Error::InvalidAccount(
                                "token differs from imported bot selection",
                            ));
                        }
                        let auth = checked.client.authenticate_and_pin(&host, &credential)?;
                        checked.client.discover_local_team_graph(
                            &host,
                            &credential,
                            &auth.verified,
                            &auth.puks,
                        )?;
                        auth
                    }
                    ImportAccountKind::Yubi => {
                        let Some(VerificationSecret::Yubi { pin, provider }) =
                            secrets.get(&account.alias)
                        else {
                            return Err(Error::YubiUnlockRequired(account.alias.clone()));
                        };
                        let loaded = vault.yubi_account(&account.alias)?;
                        let parent = provider.open(&loaded.locator, Some(pin))?;
                        let credential = loaded.credential(parent.as_ref());
                        let auth = checked
                            .client
                            .authenticate_yubi_and_pin(&host, &credential)?;
                        checked.client.discover_local_team_graph_yubi(
                            &host,
                            &credential,
                            &auth.verified,
                            &auth.puks,
                        )?;
                        auth
                    }
                };
                let (_, merkle) = checked.client.advance_merkle_root(&host)?;
                current_users.insert(authenticated.verified.uid().as_bytes().to_vec());
                Ok(VerifiedImportAccount {
                    account: account.clone(),
                    user_sequence: authenticated.verified.chain_seqno(),
                    merkle_epoch: merkle.root().epoch,
                })
            })();
            let status = match result {
                Ok(proof) => {
                    proofs.push(proof);
                    "verified"
                }
                Err(error) => classify(&error),
            };
            reports.push(AccountVerificationReport {
                alias: account.alias.clone(),
                status,
            });
        }
        // A revoked mTLS credential can be rejected before an RPC status exists.
        // Label it revoked only when another account in this pass fetched a current
        // authenticated chain for the same user; a transport failure alone is not proof.
        for (account, report) in state.accounts.iter().zip(&mut reports) {
            if !matches!(report.status, "offline" | "failed") {
                continue;
            }
            let identity = match account.kind {
                ImportAccountKind::Software => vault.account(&account.alias).and_then(|a| {
                    Ok((a.credential.uid.clone(), a.credential.public_material()?.id))
                }),
                ImportAccountKind::Bot => vault.bot_selection(&account.alias).and_then(|s| {
                    let s = s.ok_or(Error::AccountMissing)?;
                    Ok((
                        foks_proto::EntityId::from_bytes(s.uid)?,
                        foks_proto::EntityId::from_bytes(s.device_id)?,
                    ))
                }),
                ImportAccountKind::Yubi => continue,
            };
            if let Ok((uid, device)) = identity {
                if current_users.contains(uid.as_bytes())
                    && checked
                        .client
                        .pinned_user(&host, &uid)?
                        .is_some_and(|u| !u.devices().iter().any(|d| d.id == device))
                {
                    report.status = "revoked";
                }
            }
        }
        let verified = proofs.len() == state.accounts.len();
        if verified {
            let mut db = HardStateStore::open(&session.paths.hard_database)?;
            // Re-run after a crash between DB completion and native publication.
            if !state.required {
                db.install_import_readiness(state.archive_id, state.attempt, &state.accounts)?;
            }
            db.complete_import_verification(state.attempt, &proofs)?;
            drop(db);
            credentials.advance_checkpoint(session)?;
            credentials.with_native_manifest(|native| {
                let key = readiness::key(&session.profile.name)?;
                let record: NativeReadiness = serde_json::from_slice(
                    native
                        .records
                        .get(&key)
                        .ok_or(Error::StateRecoveryRequired)?,
                )?;
                if record != NativeReadiness::from_state(&state)? {
                    return Err(Error::StateRecoveryRequired);
                }
                native.remove(&key)?;
                Ok(())
            })?;
        }
        Ok(ProfileVerificationReport {
            profile: session.profile.name.clone(),
            verified,
            accounts: reports,
        })
    })
}
fn classify(error: &Error) -> &'static str {
    match error {
        Error::BotTokenLocked => "token-required",
        Error::YubiUnlockRequired(_) => "hardware-required",
        Error::Client(foks_client::Error::CredentialBinding(_)) => "revoked",
        Error::Client(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1069,
            ..
        })) => "reauthentication-required",
        Error::Client(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1018,
            ..
        })) => "revoked",
        Error::Client(
            foks_client::Error::Connect(_)
            | foks_client::Error::NoAddress(_)
            | foks_client::Error::DeadlineExceeded,
        )
        | Error::Client(foks_client::Error::Rpc(foks_rpc::Error::Io(_)))
        | Error::Io(_) => "offline",
        _ => "failed",
    }
}

pub enum ImportReauthenticationAction {
    Begin,
    Status([u8; 16]),
    Poll([u8; 16]),
    Cancel([u8; 16]),
    Finish([u8; 16]),
}
/// Only current-device reauthentication is admitted while import is gated.
pub fn reauthenticate_imported_account(
    credentials: &ClientCredentials,
    session: &ProfileSession,
    alias: &str,
    action: ImportReauthenticationAction,
    hardware: Option<(&foks_yubi::Pin, &dyn foks_yubi::YubiProvider)>,
    http: &foks_oidc::ProviderHttp,
) -> Result<crate::SsoReport> {
    credentials.with_import_session(session, |checked| {
        let state = readiness::validate_for_proof(credentials, session)?;
        if !state
            .accounts
            .iter()
            .any(|a| a.alias == alias && a.kind != ImportAccountKind::Bot)
        {
            return Err(Error::AccountMissing);
        }
        let master = credentials.master_key()?;
        let mut store = foks_keystore::EncryptedFileSecretStore::open(
            &session.paths.credential_store,
            crate::derive_vault_key(&master),
        )?;
        let mut vault = AccountVault::new(&mut store);
        let parent = if let Some((pin, provider)) = hardware {
            Some(provider.open(&vault.yubi_account(alias)?.locator, Some(pin))?)
        } else {
            None
        };
        let id = match action {
            ImportReauthenticationAction::Begin => None,
            ImportReauthenticationAction::Status(id)
            | ImportReauthenticationAction::Poll(id)
            | ImportReauthenticationAction::Cancel(id)
            | ImportReauthenticationAction::Finish(id) => Some(id),
        };
        if let Some(id) = id {
            if checked.checked_sso_flow(alias, id, &mut vault)?.purpose
                != foks_proto::SsoPurpose::Reauthenticate
            {
                return Err(Error::ImportVerificationRequired);
            }
        }
        match action {
            ImportReauthenticationAction::Begin => match parent {
                Some(parent) => checked.begin_yubi_existing_sso(
                    alias,
                    foks_proto::SsoPurpose::Reauthenticate,
                    parent.as_ref(),
                    &mut vault,
                    &master,
                    http,
                ),
                None => checked.begin_account_sso(
                    alias,
                    foks_proto::SsoPurpose::Reauthenticate,
                    &mut vault,
                    &master,
                    http,
                ),
            },
            ImportReauthenticationAction::Finish(id) if parent.is_some() => checked
                .finish_yubi_sso_login(
                    alias,
                    id,
                    parent
                        .as_ref()
                        .ok_or(Error::ImportVerificationRequired)?
                        .as_ref(),
                    &mut vault,
                    &master,
                    http,
                ),
            other => {
                let (id, action) = match other {
                    ImportReauthenticationAction::Status(id) => (id, crate::SsoAction::Status),
                    ImportReauthenticationAction::Poll(id) => (id, crate::SsoAction::Poll),
                    ImportReauthenticationAction::Cancel(id) => (id, crate::SsoAction::Cancel),
                    ImportReauthenticationAction::Finish(id) => (id, crate::SsoAction::FinishLogin),
                    ImportReauthenticationAction::Begin => {
                        return Err(Error::ImportVerificationRequired)
                    }
                };
                checked.account_sso(alias, id, action, &mut vault, &master, http)
            }
        }
    })
}

/// Local identity labels let an imported account reach its sign-in UI while all
/// service work remains gated. This does not return a checked capability.
pub struct ImportedLocalCatalog {
    pub accounts: Vec<(String, String)>,
    pub yubi: Vec<crate::YubiAccountSummary>,
}
pub fn imported_local_catalog(
    credentials: &ClientCredentials,
    session: &ProfileSession,
) -> Result<ImportedLocalCatalog> {
    credentials.with_import_session(session, |_| {
        let state = readiness::validate_for_proof(credentials, session)?;
        let master = credentials.master_key()?;
        let mut store = foks_keystore::EncryptedFileSecretStore::open(
            &session.paths.credential_store,
            crate::derive_vault_key(&master),
        )?;
        let mut vault = AccountVault::new(&mut store);
        let mut accounts = Vec::new();
        for account in state
            .accounts
            .iter()
            .filter(|a| a.kind != ImportAccountKind::Yubi)
        {
            accounts.push((
                account.alias.clone(),
                vault.account_display_name(&account.alias)?,
            ));
        }
        Ok(ImportedLocalCatalog {
            accounts,
            yubi: vault.yubi_accounts()?,
        })
    })
}
