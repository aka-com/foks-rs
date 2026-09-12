use super::*;
use foks_agent_proto::account::RenameAction;
#[allow(clippy::too_many_arguments)]
pub(super) fn rename(
    state_dir: &Path,
    registry: &ProfileRegistry,
    profile: &str,
    alias: &str,
    action: RenameAction,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !action.validate() {
        return Err(Box::new(AgentRequestError("invalid rename action")));
    }
    let session = ProfileSession::open_with_control(registry, profile, timeout, cancellation)?;
    let credentials = ClientCredentials::open(state_dir)?;
    checked_session(&credentials, &session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        let mut vault = AccountVault::new(&mut store);
        let parent = action
            .pin()
            .map(|pin| {
                let loaded = vault.yubi_account(alias)?;
                let device = HardwareYubiProvider::new()
                    .open(&loaded.locator, Some(&Pin::new(pin.expose())?))?;
                Ok::<_, Box<dyn std::error::Error>>(device)
            })
            .transpose()?;
        let id = |s: &str| -> Result<[u8; 16], Box<dyn std::error::Error>> {
            let mut out = [0; 16];
            for (i, pair) in s.as_bytes().chunks_exact(2).enumerate() {
                out[i] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
            }
            Ok(out)
        };
        let action = match action {
            RenameAction::Prepare { username, .. } => {
                foks_client_app::RenameAction::Prepare(username)
            }
            RenameAction::Attempt { operation_id, .. } => {
                foks_client_app::RenameAction::Attempt(id(&operation_id)?)
            }
            RenameAction::Status { operation_id, .. } => {
                foks_client_app::RenameAction::Status(id(&operation_id)?)
            }
            RenameAction::Cancel { operation_id } => {
                foks_client_app::RenameAction::Cancel(id(&operation_id)?)
            }
        };
        Ok(serde_json::to_value(session.rename_account(
            alias,
            action,
            parent.as_deref().map(|p| p as _),
            &mut vault,
            &master,
        )?)?)
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn web_admin(
    state_dir: &Path,
    registry: &ProfileRegistry,
    profile: &str,
    alias: &str,
    action: foks_agent_proto::admin::AdminAction,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use foks_agent_proto::admin::{AdminAction, AdminNavigation};
    if !action.validate() {
        return Err(Box::new(AgentRequestError("invalid admin handoff")));
    }
    let session = ProfileSession::open_with_control(registry, profile, timeout, cancellation)?;
    with_vault(state_dir, &session, |session, vault| match action {
        AdminAction::Policy => {
            let (host_id, uid, destination) = session.web_admin_policy(alias, vault)?;
            Ok(serde_json::to_value(
                foks_agent_proto::admin::AdminPolicy {
                    profile: profile.into(),
                    account_alias: alias.into(),
                    host_id,
                    uid,
                    destination,
                },
            )?)
        }
        AdminAction::Configure { destination } => {
            session.configure_web_admin(alias, &destination, vault)?;
            Ok(serde_json::json!({"configured":true}))
        }
        AdminAction::Prepare { pin } => {
            let parent = pin
                .map(|pin| {
                    let a = vault.yubi_account(alias)?;
                    Ok::<_, Box<dyn std::error::Error>>(
                        HardwareYubiProvider::new()
                            .open(&a.locator, Some(&Pin::new(pin.expose())?))?,
                    )
                })
                .transpose()?;
            let handoff =
                session.web_admin_handoff(alias, parent.as_deref().map(|p| p as _), vault)?;
            Ok(serde_json::to_value(AdminNavigation {
                profile: profile.into(),
                account_alias: alias.into(),
                host_id: handoff.host_id,
                uid: handoff.uid,
                destination: handoff.destination,
                url: foks_agent_proto::SecretString::new(handoff.navigation.expose()),
            })?)
        }
    })
}
