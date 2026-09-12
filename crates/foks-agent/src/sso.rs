use super::*;
use foks_agent_proto::sso::SsoAction;
pub(super) fn handle(
    state_dir: &Path,
    registry: &ProfileRegistry,
    profile: &str,
    alias: &str,
    action: SsoAction,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if !action.validate() {
        return Err(Box::new(AgentRequestError("invalid authentication action")));
    }
    let session = ProfileSession::open_with_control(registry, profile, timeout, cancellation)?;
    let credentials = ClientCredentials::open(state_dir)?;
    let http = foks_oidc::ProviderHttp::new(foks_oidc::NetworkPolicy::default())?;
    checked_session(&credentials, &session, |session| {
        let master = credentials.master_key()?;
        let mut store = EncryptedFileSecretStore::open(
            &session.paths().credential_store,
            derive_vault_key(&master),
        )?;
        let mut vault = AccountVault::new(&mut store);
        let result = match action {
            SsoAction::BeginYubiSignup {
                card_serial,
                signing_slot,
                pq_slot,
                pin,
                device_name,
                invite,
            } => {
                let provider = HardwareYubiProvider::new();
                let input = YubiSignupInput {
                    alias: alias.into(),
                    username: alias.into(),
                    device_name,
                    email: String::new(),
                    invite: invite.expose().into(),
                    passphrase: None,
                    card: yubi_card(&provider, card_serial)?,
                    signing_slot: SlotId::new(signing_slot)?,
                    pq_slot: SlotId::new(pq_slot)?,
                    retry_configuration: None,
                };
                session.begin_yubi_sso_signup(
                    input,
                    Pin::new(pin.expose())?,
                    &provider,
                    &mut vault,
                    &master,
                    &http,
                )?
            }
            SsoAction::FinishYubiSignup { operation_id, pin } => session.finish_yubi_sso_signup(
                alias,
                operation(&operation_id)?,
                Pin::new(pin.expose())?,
                &HardwareYubiProvider::new(),
                &mut vault,
                &master,
                &http,
            )?,
            SsoAction::Begin { for_login } => {
                session.begin_account_sso(alias, for_login, &mut vault, &master, &http)?
            }
            SsoAction::Status { operation_id } => session.account_sso(
                alias,
                operation(&operation_id)?,
                foks_client_app::SsoAction::Status,
                &mut vault,
                &master,
                &http,
            )?,
            SsoAction::Poll { operation_id } => session.account_sso(
                alias,
                operation(&operation_id)?,
                foks_client_app::SsoAction::Poll,
                &mut vault,
                &master,
                &http,
            )?,
            SsoAction::Cancel { operation_id } => session.account_sso(
                alias,
                operation(&operation_id)?,
                foks_client_app::SsoAction::Cancel,
                &mut vault,
                &master,
                &http,
            )?,
            SsoAction::FinishLogin { operation_id, pin } => {
                if let Some(pin) = pin {
                    let loaded = vault.yubi_account(alias)?;
                    let device = HardwareYubiProvider::new()
                        .open(&loaded.locator, Some(&Pin::new(pin.expose())?))?;
                    session.finish_yubi_sso_login(
                        alias,
                        operation(&operation_id)?,
                        device.as_ref(),
                        &mut vault,
                        &master,
                        &http,
                    )?
                } else {
                    session.account_sso(
                        alias,
                        operation(&operation_id)?,
                        foks_client_app::SsoAction::FinishLogin,
                        &mut vault,
                        &master,
                        &http,
                    )?
                }
            }
            SsoAction::FinishSignup {
                operation_id,
                device_name,
                invite,
                passphrase,
            } => session.finish_account_sso_signup(
                alias,
                operation(&operation_id)?,
                foks_client_app::SsoSignupInput {
                    device_name,
                    invite: invite.expose().into(),
                    passphrase: passphrase
                        .as_ref()
                        .map(|p| Passphrase::new(p.expose()))
                        .transpose()?,
                },
                &mut vault,
                &master,
                &http,
            )?,
        };
        Ok(serde_json::to_value(result)?)
    })
}
fn operation(value: &str) -> Result<[u8; 16], Box<dyn std::error::Error>> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Box::new(AgentRequestError(
            "authentication handle must be 32 lowercase hex characters",
        )));
    }
    let mut out = [0; 16];
    for (b, pair) in out.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        *b = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(out)
}
