use super::*;
use foks_client::{FoksScheduler, ScheduledJobRegistration, SchedulerConfig};

const FEDERATION_JOB_TYPE_ID: u64 = 0xc426_0c8c_25bb_912d;
const FEDERATION_REFRESH_INTERVAL_MICROS: u64 = 24 * 60 * 60 * 1_000_000;
const FEDERATION_FIRST_RETRY_MICROS: u64 = 5 * 60 * 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FederationDestinationRole {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl FederationDestinationRole {
    pub(crate) fn role(self) -> Role {
        match self {
            Self::Member { visibility } => Role::member(visibility),
            Self::Admin => Role::ADMIN,
            Self::Owner => Role::OWNER,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FederationAdmissionReport {
    pub operation_id_hex: String,
    pub local_profile: String,
    pub local_team_alias: String,
    pub remote_profile: String,
    pub remote_team_alias: String,
    pub remote_team_id_hex: String,
    pub destination: FederationDestinationRole,
    pub scheduled_job_id_hex: String,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FederatedMembershipSummary {
    pub local_team_alias: String,
    pub remote_profile: String,
    pub remote_team_alias: String,
    pub remote_host_id_hex: String,
    pub remote_team_id_hex: String,
    pub destination: FederationDestinationRole,
    pub operation_id_hex: Option<String>,
    pub active: bool,
}

impl CheckedProfileSession<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn admit_federated_team(
        &self,
        remote: &CheckedProfileSession<'_>,
        local_team_alias: &str,
        remote_team_alias: &str,
        destination: FederationDestinationRole,
        local_vault: &mut AccountVault<'_>,
        remote_vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<FederationAdmissionReport> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Federation)?;
        remote.profile.require(Capability::Teams)?;
        remote.profile.require(Capability::Federation)?;
        if self.profile.name == remote.profile.name {
            return Err(Error::InvalidConfig(
                "federation requires distinct local and remote profiles",
            ));
        }
        let local_host = self.pinned_host()?;
        let remote_host = remote.pinned_host()?;
        if local_host.host_id() == remote_host.host_id() {
            return Err(Error::InvalidConfig(
                "federation profiles resolve to the same host",
            ));
        }

        let mut local_team = local_vault.team(local_team_alias)?;
        let remote_team = remote_vault.team(remote_team_alias)?;
        if local_team.kind != super::team::StoredTeamKind::Named
            || !local_team.active
            || !remote_team.active
        {
            return Err(Error::InvalidAccount(
                "federation requires active teams and a named local team",
            ));
        }
        let local_team_id = EntityId::from_bytes(local_team.team_id.clone())?;
        let remote_team_id = EntityId::from_bytes(remote_team.team_id.clone())?;
        let local_account = local_vault.account(&local_team.account_alias)?;
        let remote_account = remote_vault.account(&remote_team.account_alias)?;

        let binding = local_team.federated_members.iter().position(|member| {
            member.remote_host_id == remote_host.host_id().as_bytes()
                && member.remote_team_id == remote_team_id.as_bytes()
        });
        let removal_key = match binding {
            Some(index) => {
                let stored = &local_team.federated_members[index];
                if stored.remote_profile != remote.profile.name
                    || stored.remote_team_alias != remote_team_alias
                    || stored.destination != destination
                {
                    return Err(Error::InvalidAccount(
                        "federated membership binding cannot be repurposed",
                    ));
                }
                stored.removal_key
            }
            None => {
                let removal_key = random_array()?;
                local_team
                    .federated_members
                    .push(super::team::StoredFederatedMembership {
                        remote_profile: remote.profile.name.clone(),
                        remote_team_alias: remote_team_alias.to_owned(),
                        remote_host_id: remote_host.host_id().as_bytes().to_vec(),
                        remote_team_id: remote_team_id.as_bytes().to_vec(),
                        destination,
                        removal_key,
                        operation_id: None,
                        active: false,
                    });
                local_vault.put_team(&local_team)?;
                removal_key
            }
        };

        let scheduled_job_id = federation_job_id(
            local_host.host_id(),
            &local_team_id,
            remote_host.host_id(),
            &remote_team_id,
            &removal_key,
        )?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let removal_key = SecretSeed::new(removal_key);
        let outcome = self.client.admit_remote_team_to_named_team(
            &foks_client::FederatedTeamAdmissionRequest {
                remote_host: &remote_host,
                remote_credential: &remote_account.credential,
                remote_team: &remote_team_id,
                local_host: &local_host,
                local_credential: &local_account.credential,
                local_team: &local_team_id,
                destination_role: destination.role(),
                removal_key: &removal_key,
            },
            &mut mutations,
        )?;

        let stored = local_team
            .federated_members
            .iter_mut()
            .find(|member| {
                member.remote_host_id == remote_host.host_id().as_bytes()
                    && member.remote_team_id == remote_team_id.as_bytes()
            })
            .ok_or(Error::InvalidAccount(
                "federated membership disappeared from protected storage",
            ))?;
        stored.operation_id = Some(outcome.operation_id);
        stored.active = true;
        local_vault.put_team(&local_team)?;

        let now = now_microseconds()?;
        FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?.register(
            ScheduledJobRegistration {
                job_id: scheduled_job_id,
                kind: ScheduledJobKind::FederationRefresh,
                host_id: local_host.host_id().as_bytes().to_vec(),
                // The public job row identifies the wake-up only. Profile and
                // team aliases remain authoritative in the encrypted record.
                scope_id: Vec::new(),
                interval_micros: FEDERATION_REFRESH_INTERVAL_MICROS,
                first_run_at: now
                    .checked_add(FEDERATION_FIRST_RETRY_MICROS)
                    .ok_or(Error::InvalidConfig("federation refresh time overflow"))?,
                registered_at: now,
            },
        )?;

        Ok(FederationAdmissionReport {
            operation_id_hex: hex(&outcome.operation_id),
            local_profile: self.profile.name.clone(),
            local_team_alias: local_team_alias.to_owned(),
            remote_profile: remote.profile.name.clone(),
            remote_team_alias: remote_team_alias.to_owned(),
            remote_team_id_hex: hex(remote_team_id.as_bytes()),
            destination,
            scheduled_job_id_hex: hex(&scheduled_job_id),
            active: true,
        })
    }

    pub fn list_federated_memberships(
        &self,
        local_team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<FederatedMembershipSummary>> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Federation)?;
        let team = vault.team(local_team_alias)?;
        Ok(team
            .federated_members
            .iter()
            .map(|member| FederatedMembershipSummary {
                local_team_alias: local_team_alias.to_owned(),
                remote_profile: member.remote_profile.clone(),
                remote_team_alias: member.remote_team_alias.clone(),
                remote_host_id_hex: hex(&member.remote_host_id),
                remote_team_id_hex: hex(&member.remote_team_id),
                destination: member.destination,
                operation_id_hex: member.operation_id.map(|operation| hex(&operation)),
                active: member.active,
            })
            .collect())
    }

    fn refresh_federated_team(
        &self,
        remote: &CheckedProfileSession<'_>,
        binding: &ScheduledFederationBinding,
        local_vault: &mut AccountVault<'_>,
        remote_vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        self.profile.require(Capability::Federation)?;
        self.profile.require(Capability::Teams)?;
        remote.profile.require(Capability::Federation)?;
        remote.profile.require(Capability::Teams)?;
        let local_host = self.pinned_host()?;
        let remote_host = remote.pinned_host()?;
        let local_team = local_vault.team(&binding.local_team_alias)?;
        let remote_team = remote_vault.team(&binding.remote_team_alias)?;
        let local_team_id = EntityId::from_bytes(local_team.team_id.clone())?;
        let remote_team_id = EntityId::from_bytes(remote_team.team_id.clone())?;
        let member = local_team
            .federated_members
            .iter()
            .find(|member| {
                member.remote_profile == remote.profile.name
                    && member.remote_team_alias == binding.remote_team_alias
                    && member.remote_host_id == remote_host.host_id().as_bytes()
                    && member.remote_team_id == remote_team_id.as_bytes()
            })
            .ok_or(Error::InvalidAccount(
                "scheduled federation membership disappeared",
            ))?;
        if !local_team.active
            || !remote_team.active
            || !member.active
            || member.operation_id.is_none()
            || member.destination != binding.destination
        {
            return Err(Error::InvalidAccount(
                "scheduled federation membership is not active",
            ));
        }
        let local_account = local_vault.account(&local_team.account_alias)?;
        let remote_account = remote_vault.account(&remote_team.account_alias)?;
        self.client.refresh_federated_team_capability(
            &foks_client::FederatedTeamRefreshRequest {
                remote_host: &remote_host,
                remote_credential: &remote_account.credential,
                remote_team: &remote_team_id,
                local_host: &local_host,
                local_credential: &local_account.credential,
                local_team: &local_team_id,
            },
        )?;
        Ok(())
    }

    /// Runs ordinary local jobs plus cross-profile federation refreshes. The
    /// caller already holds this profile's checked operation lock; the remote
    /// lock is attempted without waiting so inverse profile jobs cannot
    /// deadlock each other.
    pub fn run_due_jobs_with_federation(
        &self,
        now: u64,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<JobRunReport> {
        self.profile.require(Capability::UserSync)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        self.run_due_jobs_locked_with(now, local_vault, |job, local_vault| {
            let binding = self
                .protected_job_binding(job, local_vault)
                .map_err(|error| error.to_string())?;
            let remote_session = self
                .related_profile(registry, &binding.remote_profile)
                .map_err(|error| error.to_string())?;
            let attempted = credentials
                .try_with_checked_session(&remote_session, |remote| {
                    let mut remote_store = foks_keystore::EncryptedFileSecretStore::open(
                        &remote.paths.credential_store,
                        derive_vault_key(master_key),
                    )?;
                    let mut remote_vault = AccountVault::new(&mut remote_store);
                    self.refresh_federated_team(remote, &binding, local_vault, &mut remote_vault)?;
                    Ok::<_, Error>(())
                })
                .map_err(|error| error.to_string())?;
            attempted.ok_or_else(|| "remote federation profile is busy; retry later".to_owned())
        })
    }

    fn protected_job_binding(
        &self,
        job: &foks_client_db::ScheduledJob,
        vault: &mut AccountVault<'_>,
    ) -> Result<ScheduledFederationBinding> {
        if !job.scope_id.is_empty() {
            return Err(Error::InvalidAccount(
                "federation job contains an untrusted public scope",
            ));
        }
        let local_host = self.pinned_host()?;
        if job.host_id != local_host.host_id().as_bytes() {
            return Err(Error::InvalidAccount(
                "federation job is bound to another local host",
            ));
        }
        let mut found = None;
        for local_team_alias in vault.team_aliases()? {
            let team = vault.team(&local_team_alias)?;
            let local_team = EntityId::from_bytes(team.team_id.clone())?;
            for member in &team.federated_members {
                let remote_host = EntityId::from_bytes(member.remote_host_id.clone())?;
                let remote_team = EntityId::from_bytes(member.remote_team_id.clone())?;
                let candidate = federation_job_id(
                    local_host.host_id(),
                    &local_team,
                    &remote_host,
                    &remote_team,
                    &member.removal_key,
                )?;
                if candidate != job.job_id {
                    continue;
                }
                if found.is_some() {
                    return Err(Error::InvalidAccount(
                        "federation job matches multiple protected bindings",
                    ));
                }
                found = Some(ScheduledFederationBinding {
                    local_team_alias: local_team_alias.clone(),
                    remote_profile: member.remote_profile.clone(),
                    remote_team_alias: member.remote_team_alias.clone(),
                    destination: member.destination,
                });
            }
        }
        found.ok_or(Error::InvalidAccount(
            "federation job has no protected binding",
        ))
    }
}

struct ScheduledFederationBinding {
    local_team_alias: String,
    remote_profile: String,
    remote_team_alias: String,
    destination: FederationDestinationRole,
}

fn federation_job_id(
    local_host: &EntityId,
    local_team: &EntityId,
    remote_host: &EntityId,
    remote_team: &EntityId,
    removal_key: &[u8; 32],
) -> Result<[u8; 16]> {
    let removal_commitment =
        foks_crypto::team_removal_key_commitment(&SecretSeed::new(*removal_key))?;
    let mut binding = Vec::with_capacity(33 * 4 + 32);
    binding.extend_from_slice(local_host.as_bytes());
    binding.extend_from_slice(local_team.as_bytes());
    binding.extend_from_slice(remote_host.as_bytes());
    binding.extend_from_slice(remote_team.as_bytes());
    binding.extend_from_slice(&removal_commitment);
    Ok(prefixed_hash(FEDERATION_JOB_TYPE_ID, &binding)[..16]
        .try_into()
        .expect("hash prefix has fixed length"))
}
