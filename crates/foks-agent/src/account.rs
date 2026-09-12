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
