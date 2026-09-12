use super::*;
#[derive(Serialize)]
pub struct BotRevocationReport {
    pub operation_id: Option<String>,
    pub device_id: String,
    pub state: &'static str,
    pub currently_active: bool,
}
impl CheckedProfileSession<'_> {
    pub fn revoke_bot_account_credential(
        &self,
        owner: &str,
        target_hex: &str,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<BotRevocationReport> {
        self.profile.require(Capability::DeviceAdministration)?;
        let target =
            entity_id_from_hex(target_hex)?.require_type(foks_proto::ENTITY_BOT_TOKEN_KEY)?;
        let host = self.pinned_host()?;
        let mut protected = self.mutation_store(master)?;
        self.with_bot_owner(owner, parent, vault, |credential| {
            let prior = HardStateStore::open(&self.paths.hard_database)?
                .latest_mutation_for_binding(
                    host.host_id().as_bytes(),
                    MutationKind::DeviceRevoke,
                    credential.uid().as_bytes(),
                    target.as_bytes(),
                )?;
            if let Some(prior) = prior.filter(|op| !op.state.is_terminal()) {
                let op = self.client.bot_revocation_progress(
                    &host,
                    credential,
                    prior.operation_id,
                    true,
                    &mut protected,
                )?;
                if op.state == MutationState::RemoteVerified {
                    MutationCoordinator::new(&self.paths.hard_database, &mut protected)
                        .finalize(&op.operation_id)?;
                }
                let current = self
                    .client
                    .authenticate_credential_and_pin(&host, credential)?;
                return Ok(BotRevocationReport {
                    operation_id: Some(hex(&op.operation_id)),
                    device_id: target_hex.into(),
                    state: if matches!(
                        op.state,
                        MutationState::RemoteVerified | MutationState::Finalized
                    ) {
                        "complete"
                    } else if op.state == MutationState::Rejected {
                        "rejected"
                    } else {
                        "submission-unknown"
                    },
                    currently_active: current.verified.devices().iter().any(|d| d.id == target),
                });
            }
            let current = self
                .client
                .authenticate_credential_and_pin(&host, credential)?;
            let Some(role) = current
                .verified
                .devices()
                .iter()
                .find(|d| d.id == target)
                .map(|d| d.role)
            else {
                return Ok(BotRevocationReport {
                    operation_id: None,
                    device_id: target_hex.into(),
                    state: "absent",
                    currently_active: false,
                });
            };
            let updated = match credential {
                foks_client::FederationCredential::Software(c) => {
                    if c.key_kind != foks_client::SoftwareKeyKind::Device {
                        return Err(Error::InvalidAccount(
                            "use a permanent owner to administer bot credentials",
                        ));
                    }
                    let (rotations, no_passphrase) =
                        self.software_revocation_material(&host, c, &current, role)?;
                    self.client.revoke_user_credential_with_software_device(
                        &host,
                        c,
                        &target,
                        &rotations,
                        no_passphrase,
                        &mut protected,
                    )?
                }
                foks_client::FederationCredential::Yubi(c) => {
                    let mut rotations = Vec::new();
                    for public in current
                        .verified
                        .shared_keys()
                        .iter()
                        .filter(|p| p.role <= role)
                    {
                        let previous = self
                            .client
                            .load_puks_for_role_yubi(&host, c, &current.verified, public.role)?
                            .into_iter()
                            .find(|k| k.generation == public.generation && k.role == public.role)
                            .ok_or(Error::InvalidAccount("missing bot revocation PUK"))?;
                        rotations.push(foks_client::UserPukRotation {
                            role: public.role,
                            previous_generation: public.generation,
                            previous_seed: previous.seed,
                            new_seed: SecretSeed::new(random_array()?),
                        });
                    }
                    let no_passphrase = if role == Role::OWNER {
                        self.profile.require(Capability::Passphrases)?;
                        if !self.client.passphrase_is_configured_yubi(&host, c)?
                            && HardStateStore::open(&self.paths.hard_database)?
                                .user_has_no_passphrase_attestation(
                                    host.host_id().as_bytes(),
                                    c.uid.as_bytes(),
                                )?
                        {
                            Some(foks_client::NoPassphraseConfigured)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    self.client.revoke_bot_credential_with_yubi(
                        &host,
                        c,
                        &target,
                        &rotations,
                        no_passphrase,
                        &mut protected,
                    )?
                }
            };
            let op = HardStateStore::open(&self.paths.hard_database)?.latest_mutation_for_binding(
                host.host_id().as_bytes(),
                MutationKind::DeviceRevoke,
                credential.uid().as_bytes(),
                target.as_bytes(),
            )?;
            Ok(BotRevocationReport {
                operation_id: op.map(|o| hex(&o.operation_id)),
                device_id: target_hex.into(),
                state: "complete",
                currently_active: updated.verified.devices().iter().any(|d| d.id == target),
            })
        })
    }
}
