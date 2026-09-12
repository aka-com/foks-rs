//! Account-bound browser authentication; frontends receive no provider credentials.
use super::*;
use account::PendingSignup;
use foks_client::{FederationCredential, SsoIntent, SsoProgress, SsoSigningKey};
use foks_client_db::{SsoFlow, SsoFlowState};
use foks_oidc::ProviderHttp;

#[derive(Clone, Copy, Debug)]
pub enum SsoAction {
    Status,
    Poll,
    Cancel,
    FinishLogin,
}
#[derive(Clone, Serialize)]
pub struct SsoReport {
    pub operation_id: String,
    pub account_alias: String,
    pub for_login: bool,
    pub state: &'static str,
    pub browser_url: Option<String>,
    pub expires_at_ms: u64,
    pub service_access: bool,
}
impl std::fmt::Debug for SsoReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoReport")
            .field("state", &self.state)
            .field("service_access", &self.service_access)
            .finish_non_exhaustive()
    }
}
pub struct SsoSignupInput {
    pub device_name: String,
    pub invite: String,
    pub passphrase: Option<Passphrase>,
}
impl Drop for SsoSignupInput {
    fn drop(&mut self) {
        self.invite.zeroize();
    }
}
pub(super) fn report(alias: &str, for_login: bool, p: SsoProgress) -> SsoReport {
    use SsoFlowState::*;
    SsoReport {
        operation_id: hex(&p.operation_id),
        account_alias: alias.into(),
        for_login,
        state: match p.state {
            Prepared => "prepared",
            AwaitingBrowser => "waiting",
            Ready => "ready",
            Binding => "submitting",
            Complete => "complete",
            Cancelled => "cancelled",
            Expired => "expired",
            Unknown => "submission-unknown",
            Rejected => "rejected",
            Denied => "denied",
        },
        browser_url: p.browser_url,
        expires_at_ms: p.expires_at_ms,
        service_access: false,
    }
}
fn apply_service_result<T>(report: &mut SsoReport, result: foks_client::Result<T>) {
    match result {
        Ok(_) => report.service_access = true,
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1069, .. })) => {
            report.state = "reauthentication-required"
        }
        Err(_) => report.state = "service-unavailable",
    }
}
fn pending_intent(p: &PendingSignup) -> Result<SsoIntent> {
    let mut uid =
        derive_shared_verify_key(&SecretSeed::new(p.puk_seed), ENTITY_PUK_VERIFY)?.into_bytes();
    uid[0] = ENTITY_USER;
    Ok(SsoIntent {
        uid: EntityId::from_bytes(uid)?,
        device: derive_device_public(&SecretSeed::new(p.device_seed))?.id,
        for_login: false,
    })
}
impl CheckedProfileSession<'_> {
    fn sso_identity(
        &self,
        alias: &str,
        login: bool,
        vault: &mut AccountVault<'_>,
    ) -> Result<SsoIntent> {
        validate_name(alias)?;
        if login {
            match vault.account(alias) {
                Ok(a) => Ok(SsoIntent {
                    uid: a.credential.uid.clone(),
                    device: a.credential.public_material()?.id,
                    for_login: true,
                }),
                Err(Error::AccountMissing) => {
                    let a = vault.yubi_account(alias)?;
                    Ok(SsoIntent {
                        uid: a.uid,
                        device: EntityId::from_bytes(
                            [
                                vec![foks_proto::ENTITY_YUBI],
                                a.locator.signing_public_key.to_vec(),
                            ]
                            .concat(),
                        )?,
                        for_login: true,
                    })
                }
                Err(e) => Err(e),
            }
        } else {
            match vault.pending(alias) {
                Ok(p) => pending_intent(&p),
                Err(Error::AccountMissing) => vault.pending_yubi_sso_intent(alias),
                Err(e) => Err(e),
            }
        }
    }
    pub(super) fn checked_sso_flow(
        &self,
        alias: &str,
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
    ) -> Result<SsoFlow> {
        let host = self.pinned_host()?;
        let flow = HardStateStore::open(&self.paths.hard_database)?
            .sso_flow(&id)?
            .ok_or(Error::InvalidAccount("authentication flow is missing"))?;
        // After signup commits, its account credential replaces the pending seeds.
        let intent = match self.sso_identity(alias, flow.for_login, vault) {
            Err(Error::AccountMissing) if !flow.for_login => {
                self.sso_identity(alias, true, vault)?
            }
            value => value?,
        };
        if flow.host != host.host_id().as_bytes()
            || flow.uid != intent.uid.as_bytes()
            || flow.device != intent.device.as_bytes()
        {
            return Err(Error::InvalidAccount(
                "authentication flow belongs to another account or device",
            ));
        }
        self.profile.require(if flow.for_login {
            Capability::UserSync
        } else {
            Capability::Signup
        })?;
        Ok(flow)
    }
    pub fn begin_account_sso(
        &self,
        alias: &str,
        login: bool,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
        http: &ProviderHttp,
    ) -> Result<SsoReport> {
        validate_name(alias)?;
        self.profile.require(if login {
            Capability::UserSync
        } else {
            Capability::Signup
        })?;
        let host = self.pinned_host()?;
        if self.client.registration_server_config(&host)?.sso.is_none() {
            return Err(Error::InvalidAccount("host does not require SSO"));
        }
        if !login {
            match vault.pending(alias) {
                Ok(p) => {
                    if p.journal_operation(&self.pinned_host()?, &self.paths.hard_database)?
                        .is_some()
                    {
                        return Err(Error::InvalidAccount(
                            "resume the existing signup before beginning authentication",
                        ));
                    }
                }
                Err(Error::AccountMissing) => {
                    if vault.contains(alias)? {
                        return Err(Error::AccountExists);
                    }
                    vault.put_pending(&PendingSignup::random(alias, alias)?)?;
                }
                Err(e) => return Err(e),
            }
        }
        let intent = self.sso_identity(alias, login, vault)?;
        self.begin_bound_sso(alias, intent, master, http)
    }
    pub(super) fn begin_bound_sso(
        &self,
        alias: &str,
        intent: SsoIntent,
        master: &[u8; 32],
        http: &ProviderHttp,
    ) -> Result<SsoReport> {
        let login = intent.for_login;
        let host = self.pinned_host()?;
        let mut protected = self.mutation_store(master)?;
        // Reopening a browser flow never sends init again, even when its reply was lost.
        for flow in HardStateStore::open(&self.paths.hard_database)?
            .sso_flows(host.host_id().as_bytes(), intent.uid.as_bytes())?
        {
            if flow.device == intent.device.as_bytes() && flow.for_login == login {
                let p = self.client.sso_progress(&host, flow.id, &mut protected)?;
                if matches!(
                    p.state,
                    SsoFlowState::Prepared
                        | SsoFlowState::AwaitingBrowser
                        | SsoFlowState::Ready
                        | SsoFlowState::Binding
                ) {
                    return Ok(report(alias, login, p));
                }
            }
        }
        Ok(report(
            alias,
            login,
            self.client.begin_sso(&host, intent, http, &mut protected)?,
        ))
    }
    pub fn account_sso(
        &self,
        alias: &str,
        id: [u8; 16],
        action: SsoAction,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
        http: &ProviderHttp,
    ) -> Result<SsoReport> {
        let flow = self.checked_sso_flow(alias, id, vault)?;
        let host = self.pinned_host()?;
        let mut protected = self.mutation_store(master)?;
        let result = match action {
            SsoAction::Status => self.client.sso_progress(&host, id, &mut protected),
            // Short isolated waits release the profile lock between polls.
            SsoAction::Poll => self.client.poll_sso(&host, id, 250, http, &mut protected),
            SsoAction::Cancel => self.client.cancel_sso(&host, id, &mut protected),
            SsoAction::FinishLogin => {
                let loaded = vault.account(alias)?;
                self.client.finish_sso_login(
                    &host,
                    id,
                    FederationCredential::Software(&loaded.credential),
                    http,
                    &mut protected,
                )
            }
        };
        let mut result = match result {
            Ok(p) => report(alias, flow.for_login, p),
            Err(e) => {
                let state = match &e {
                    foks_client::Error::Oidc(foks_oidc::Error::ProviderUnavailable) => {
                        "provider-unavailable"
                    }
                    foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                        code: 1069, ..
                    }) => "reauthentication-required",
                    foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                        code: 1067,
                        detail,
                    }) if detail.detail().is_some_and(|d| {
                        d == "authorization denied" || d.contains("access_denied")
                    }) =>
                    {
                        "denied"
                    }
                    foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                        code: 1067,
                        detail,
                    }) if detail.detail() == Some("provider unavailable") => "provider-unavailable",
                    _ => return Err(e.into()),
                };
                let mut p = report(
                    alias,
                    flow.for_login,
                    self.client.sso_progress(&host, id, &mut protected)?,
                );
                p.state = state;
                p.browser_url = None;
                return Ok(p);
            }
        };
        if result.state == "complete" && flow.for_login {
            match vault.account(alias) {
                Ok(loaded) => {
                    apply_service_result(&mut result, self.client.ping(&host, &loaded.credential));
                }
                Err(Error::AccountMissing) => {
                    result.state = "hardware-verification-required";
                }
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }
    pub fn finish_yubi_sso_login(
        &self,
        alias: &str,
        id: [u8; 16],
        parent: &dyn foks_crypto::YubiDevice,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
        http: &ProviderHttp,
    ) -> Result<SsoReport> {
        let flow = self.checked_sso_flow(alias, id, vault)?;
        if !flow.for_login || flow.device != parent.entity_id().as_bytes() {
            return Err(Error::InvalidAccount(
                "hardware authentication binding differs",
            ));
        }
        let loaded = vault.yubi_account(alias)?;
        let credential = loaded.credential(parent);
        let host = self.pinned_host()?;
        let mut protected = self.mutation_store(master)?;
        let p = self.client.finish_sso_login(
            &host,
            id,
            FederationCredential::Yubi(&credential),
            http,
            &mut protected,
        )?;
        let mut result = report(alias, true, p);
        if result.state == "complete" {
            apply_service_result(
                &mut result,
                self.client.authenticate_yubi_and_pin(&host, &credential),
            );
        }
        Ok(result)
    }
    pub fn finish_account_sso_signup(
        &self,
        alias: &str,
        id: [u8; 16],
        mut input: SsoSignupInput,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
        http: &ProviderHttp,
    ) -> Result<SsoReport> {
        let flow = self.checked_sso_flow(alias, id, vault)?;
        if flow.for_login {
            return Err(Error::InvalidAccount("login flow cannot create an account"));
        }
        let host = self.pinned_host()?;
        let mut protected = self.mutation_store(master)?;
        if input.passphrase.is_some() {
            self.profile.require(Capability::Passphrases)?;
        }
        if flow.final_operation.is_some() {
            self.resume_account(alias, vault, master)?;
        } else {
            let mut pending = vault.pending(alias)?;
            let auth = self.client.authorize_sso_signup(
                &host,
                id,
                SsoSigningKey::Software(&SecretSeed::new(pending.device_seed)),
                http,
                &mut protected,
            )?;
            pending.username = auth.username().into();
            vault.put_pending(&pending)?;
            let created = self.client.create_software_account_with_sso(
                &host,
                SoftwareAccountRequest {
                    username_utf8: auth.username().into(),
                    device_name: input.device_name.clone(),
                    email: auth.email().into(),
                    invite_code: InviteCode::from_user_input(&input.invite, true)?,
                    passphrase: input.passphrase.take(),
                },
                pending.secrets()?,
                &self.paths.soft_database,
                &auth,
                &mut protected,
            )?;
            vault.commit_created(alias, &pending.username, &created.credential)?;
            self.register_default_refresh_jobs_for(&created.credential.uid, now_microseconds()?)?;
            MutationCoordinator::new(&self.paths.hard_database, &mut protected)
                .finalize(&created.operation_id)?;
            vault.remove_pending_signup(alias)?;
        }
        let mut result = report(
            alias,
            false,
            self.client.sso_progress(&host, id, &mut protected)?,
        );
        apply_service_result(
            &mut result,
            self.client.ping(&host, &vault.account(alias)?.credential),
        );
        Ok(result)
    }
}
#[cfg(test)]
mod tests;
