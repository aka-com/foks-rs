use super::*;
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    state_dir: &Path,
    registry: &ProfileRegistry,
    profile: &str,
    alias: &str,
    action: foks_agent_proto::invitations::InvitationAction,
    pin: Option<foks_agent_proto::SecretString>,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !action.validate()
        || pin
            .as_ref()
            .is_some_and(|p| p.expose().is_empty() || p.expose().len() > 32)
    {
        return Err(Box::new(AgentRequestError("invalid invitation action")));
    }
    let session =
        crate::read_cache::open_profile_session(registry, profile, timeout, cancellation.clone())?;
    let credentials = ClientCredentials::open(state_dir)?;
    if let Some(name) = action.remote_profile() {
        let remote =
            crate::read_cache::open_profile_session(registry, name, timeout, cancellation)?;
        return checked_sessions(&credentials, &session, &remote, |session, remote| {
            let master = credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            let parent = pin
                .map(|p| {
                    let a = vault.yubi_account(alias)?;
                    Ok::<_, Box<dyn std::error::Error>>(
                        HardwareYubiProvider::new()
                            .open(&a.locator, Some(&Pin::new(p.expose())?))?,
                    )
                })
                .transpose()?;
            let action = serde_json::from_value(serde_json::to_value(action)?)?;
            Ok(session.remote_invitation_action(
                remote,
                alias,
                action,
                parent.as_deref().map(|p| p as _),
                &mut vault,
                &master,
            )?)
        });
    }
    checked_session(&credentials, &session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        let mut vault = AccountVault::new(&mut store);
        let parent = pin
            .map(|p| {
                let a = vault.yubi_account(alias)?;
                Ok::<_, Box<dyn std::error::Error>>(
                    HardwareYubiProvider::new().open(&a.locator, Some(&Pin::new(p.expose())?))?,
                )
            })
            .transpose()?;
        let action = serde_json::from_value(serde_json::to_value(action)?)?;
        Ok(session.invitation_action(
            alias,
            action,
            parent.as_deref().map(|p| p as _),
            &mut vault,
            &master,
        )?)
    })
}
