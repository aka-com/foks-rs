//! The resident process owns loaded bot material; durable records contain selection only.
use super::*;
use foks_agent_proto::{bot::BotAction, SecretString};
use foks_client_app::{BotEnrollmentAction, LoadedAccount};
fn sessions() -> &'static Mutex<BTreeMap<(PathBuf, String), LoadedAccount>> {
    static SESSIONS: OnceLock<Mutex<BTreeMap<(PathBuf, String), LoadedAccount>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
fn copy(a: &LoadedAccount) -> LoadedAccount {
    LoadedAccount {
        alias: a.alias.clone(),
        username: a.username.clone(),
        credential: foks_client::DeviceCredential {
            key_kind: a.credential.key_kind,
            uid: a.credential.uid.clone(),
            seed: foks_proto::SecretSeed::new(*a.credential.seed.as_bytes()),
            certificate_chain: a.credential.certificate_chain.clone(),
        },
    }
}
pub(super) fn attach(
    session: &CheckedProfileSession<'_>,
    vault: &mut AccountVault<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let key = &session.paths().credential_store;
    let mut guard = sessions()
        .lock()
        .map_err(|_| AgentRequestError("bot session store unavailable"))?;
    let keys = guard
        .keys()
        .filter(|(path, _)| path == key)
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        let account = &guard[&key];
        let valid = vault.bot_selection(&account.alias)?.is_some_and(|p| {
            p.host_id
                == session
                    .pinned_host()
                    .map(|h| h.host_id().as_bytes().to_vec())
                    .unwrap_or_default()
                && p.uid == account.credential.uid.as_bytes()
                && p.device_id
                    == account
                        .credential
                        .public_material()
                        .map(|p| p.id.as_bytes().to_vec())
                        .unwrap_or_default()
        });
        if valid {
            vault.attach_loaded_bot(copy(account))?;
        } else {
            guard.remove(&key);
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(super) fn handle(
    state_dir: &Path,
    registry: &ProfileRegistry,
    profile: &str,
    alias: &str,
    action: BotAction,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !action.validate() {
        return Err(Box::new(AgentRequestError("invalid bot action")));
    }
    let session = ProfileSession::open_with_control(registry, profile, timeout, cancellation)?;
    with_vault_and_master(state_dir, &session, |session, vault, master| {
        let key = (session.paths().credential_store.clone(), alias.to_owned());
        match &action {
            BotAction::Load { token } => {
                let account = session.load_bot_account(alias, token.expose(), vault)?;
                let mut store = sessions()
                    .lock()
                    .map_err(|_| AgentRequestError("bot session store unavailable"))?;
                if store.len() >= 64 && !store.contains_key(&key) {
                    return Err(Box::new(AgentRequestError("loaded bot capacity")));
                }
                store.insert(key, account);
                return Ok(serde_json::json!({"account_alias":alias,"loaded":true}));
            }
            BotAction::Unload => {
                sessions()
                    .lock()
                    .map_err(|_| AgentRequestError("bot session store unavailable"))?
                    .remove(&key);
                return Ok(serde_json::json!({"account_alias":alias,"loaded":false}));
            }
            BotAction::List => {
                return Ok(serde_json::to_value(
                    session.bot_account_enrollments(alias, vault)?,
                )?)
            }
            _ => {}
        }
        let parent = action
            .pin()
            .map(|pin| {
                let record = vault.yubi_account(alias)?;
                Ok::<_, Box<dyn std::error::Error>>(
                    HardwareYubiProvider::new()
                        .open(&record.locator, Some(&Pin::new(pin.expose())?))?,
                )
            })
            .transpose()?;
        let parent = parent.as_deref().map(|p| p as _);
        if let BotAction::Revoke { device_id, .. } = action {
            let report =
                session.revoke_bot_account_credential(alias, &device_id, parent, vault, master)?;
            if !report.currently_active {
                sessions()
                    .lock()
                    .map_err(|_| AgentRequestError("bot session store unavailable"))?
                    .retain(|(path, _), a| {
                        path != &session.paths().credential_store
                            || a.credential.public_material().is_ok_and(|p| {
                                p.id.as_bytes()
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect::<String>()
                                    != device_id
                            })
                    });
            }
            return Ok(serde_json::to_value(report)?);
        }
        if let BotAction::Prepare { role, .. } = action {
            let role = match role {
                KvRole::Owner => foks_proto::Role::OWNER,
                KvRole::Admin => foks_proto::Role::ADMIN,
                KvRole::Member { visibility } => foks_proto::Role::member(visibility),
            };
            return Ok(serde_json::to_value(
                session.prepare_bot_account(alias, role, parent, vault, master)?,
            )?);
        }
        let (id, step) = match action {
            BotAction::Attempt { operation_id, .. } => (operation_id, BotEnrollmentAction::Attempt),
            BotAction::Status { operation_id, .. } => (operation_id, BotEnrollmentAction::Status),
            BotAction::Cancel { operation_id } => (operation_id, BotEnrollmentAction::Cancel),
            BotAction::Export { operation_id } => (operation_id, BotEnrollmentAction::Export),
            _ => unreachable!(),
        };
        let id = std::array::from_fn(|i| {
            u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).expect("validated handle")
        });
        let outcome = session.bot_account_enrollment(alias, id, step, parent, vault, master)?;
        if let Some(secret) = outcome.exported {
            return Ok(
                serde_json::json!({"report":outcome.report,"token":SecretString::new(secret.as_str())}),
            );
        }
        Ok(serde_json::to_value(outcome.report)?)
    })
}
