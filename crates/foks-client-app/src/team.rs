use super::*;

impl CheckedProfileSession<'_> {
    pub fn create_named_team(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_name: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        if vault.contains_team(team_alias)? {
            return Err(Error::AccountExists);
        }
        let account = vault.account(account_alias)?;
        let mut stored = StoredTeam::random_named(team_alias, account_alias, team_name)?;
        vault.put_team(&stored)?;
        let host = self.pinned_host()?;
        let secrets = stored.named_secrets()?;
        let created = self.client.create_single_owner_named_team(
            &host,
            &account.credential,
            team_name,
            &secrets,
        )?;
        let report =
            self.ensure_team_root(team_alias, &account, created.authenticated, master_key)?;
        stored.active = true;
        vault.put_team(&stored)?;
        Ok(report)
    }

    pub fn create_adhoc_team(
        &self,
        account_alias: &str,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        if vault.contains_team(team_alias)? {
            return Err(Error::AccountExists);
        }
        let account = vault.account(account_alias)?;
        let mut stored = StoredTeam::random_adhoc(team_alias, account_alias)?;
        vault.put_team(&stored)?;
        let host = self.pinned_host()?;
        let secrets = stored.adhoc_secrets()?;
        let created =
            self.client
                .create_single_owner_adhoc_team(&host, &account.credential, &secrets)?;
        let report =
            self.ensure_team_root(team_alias, &account, created.authenticated, master_key)?;
        stored.active = true;
        vault.put_team(&stored)?;
        Ok(report)
    }

    pub fn resume_team_creation(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        let mut stored = vault.team(team_alias)?;
        let account = vault.account(&stored.account_alias)?;
        let host = self.pinned_host()?;
        let authenticated = match stored.kind {
            StoredTeamKind::Named => {
                self.client
                    .resume_single_owner_named_team(
                        &host,
                        &account.credential,
                        stored
                            .name
                            .as_deref()
                            .ok_or(Error::InvalidAccount("named team has no stored name"))?,
                        &stored.named_secrets()?,
                    )?
                    .authenticated
            }
            StoredTeamKind::AdHoc => {
                self.client
                    .resume_single_owner_adhoc_team(
                        &host,
                        &account.credential,
                        &stored.adhoc_secrets()?,
                    )?
                    .authenticated
            }
        };
        let report = self.ensure_team_root(team_alias, &account, authenticated, master_key)?;
        stored.active = true;
        vault.put_team(&stored)?;
        Ok(report)
    }

    pub fn list_teams(&self, vault: &mut AccountVault<'_>) -> Result<Vec<TeamSummary>> {
        self.profile.require(Capability::Teams)?;
        vault
            .team_aliases()?
            .into_iter()
            .map(|alias| {
                let team = vault.team(&alias)?;
                Ok(TeamSummary {
                    alias,
                    account_alias: team.account_alias.clone(),
                    team_id_hex: hex(&team.team_id),
                    kind: match team.kind {
                        StoredTeamKind::Named => "named",
                        StoredTeamKind::AdHoc => "ad-hoc",
                    }
                    .to_owned(),
                    name: team.name.clone(),
                    active: team.active,
                })
            })
            .collect()
    }

    pub fn sync_team(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<TeamSyncReport> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Kv)?;
        let stored = vault.team(team_alias)?;
        if !stored.active {
            return Err(Error::InvalidAccount("team creation is still pending"));
        }
        let account = vault.account(&stored.account_alias)?;
        let host = self.pinned_host()?;
        let user = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        let team_id = EntityId::from_bytes(stored.team_id.clone())?;
        let team = self.client.load_and_pin_team(
            &host,
            &account.credential,
            &user.verified,
            &user.puks,
            &team_id,
        )?;
        let tree = self.client.sync_team_kv(
            &host,
            &account.credential,
            &team,
            &self.paths.soft_database,
        )?;
        Ok(TeamSyncReport::new(team_alias, &team, &tree))
    }

    fn ensure_team_root(
        &self,
        team_alias: &str,
        account: &LoadedAccount,
        authenticated: foks_client::AuthenticatedTeamOutcome,
        master_key: &[u8; 32],
    ) -> Result<TeamSyncReport> {
        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.team_kv_write_session(
            &host,
            &account.credential,
            &authenticated,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.ensure_root(Role::OWNER, Role::OWNER)?;
        Ok(TeamSyncReport::new(team_alias, &authenticated, &tree))
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamSummary {
    pub alias: String,
    pub account_alias: String,
    pub team_id_hex: String,
    pub kind: String,
    pub name: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamSyncReport {
    pub alias: String,
    pub team_id_hex: String,
    pub team_chain_sequence: u64,
    pub directories: usize,
    pub entries: usize,
}

impl TeamSyncReport {
    fn new(
        alias: &str,
        team: &foks_client::AuthenticatedTeamOutcome,
        tree: &[KvDirectoryProjection],
    ) -> Self {
        Self {
            alias: alias.to_owned(),
            team_id_hex: hex(team.verified.team().as_bytes()),
            team_chain_sequence: team.verified.chain_seqno(),
            directories: tree.len(),
            entries: tree.iter().map(|directory| directory.entries.len()).sum(),
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum StoredTeamKind {
    Named,
    AdHoc,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredTeam {
    pub(super) version: u32,
    pub(super) alias: String,
    pub(super) account_alias: String,
    pub(super) kind: StoredTeamKind,
    pub(super) name: Option<String>,
    pub(super) team_id: Vec<u8>,
    pub(super) member_min: [u8; 32],
    pub(super) member: [u8; 32],
    pub(super) admin: [u8; 32],
    pub(super) owner: [u8; 32],
    pub(super) removal_key: Option<[u8; 32]>,
    pub(super) name_commitment: Option<[u8; 16]>,
    pub(super) active: bool,
    #[serde(default)]
    pub(super) federated_members: Vec<StoredFederatedMembership>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredFederatedMembership {
    pub(super) remote_profile: String,
    pub(super) remote_team_alias: String,
    pub(super) remote_host_id: Vec<u8>,
    pub(super) remote_team_id: Vec<u8>,
    pub(super) destination: crate::FederationDestinationRole,
    pub(super) removal_key: [u8; 32],
    pub(super) operation_id: Option<[u8; 16]>,
    pub(super) active: bool,
}

impl StoredTeam {
    pub(super) fn random_named(alias: &str, account_alias: &str, name: &str) -> Result<Self> {
        if name.trim().is_empty() || name.len() > 256 {
            return Err(Error::InvalidAccount("team name is missing or excessive"));
        }
        let mut stored = Self::random(alias, account_alias, StoredTeamKind::Named)?;
        stored.name = Some(name.to_owned());
        stored.removal_key = Some(random_array()?);
        stored.name_commitment = Some(random_array()?);
        stored.team_id = stored.named_secrets()?.team_id()?.into_bytes();
        Ok(stored)
    }

    fn random_adhoc(alias: &str, account_alias: &str) -> Result<Self> {
        let mut stored = Self::random(alias, account_alias, StoredTeamKind::AdHoc)?;
        stored.team_id = stored.adhoc_secrets()?.team_id()?.into_bytes();
        Ok(stored)
    }

    fn random(alias: &str, account_alias: &str, kind: StoredTeamKind) -> Result<Self> {
        validate_name(alias)?;
        validate_name(account_alias)?;
        Ok(Self {
            version: CREDENTIAL_VERSION,
            alias: alias.to_owned(),
            account_alias: account_alias.to_owned(),
            kind,
            name: None,
            team_id: Vec::new(),
            member_min: random_array()?,
            member: random_array()?,
            admin: random_array()?,
            owner: random_array()?,
            removal_key: None,
            name_commitment: None,
            active: false,
            federated_members: Vec::new(),
        })
    }

    fn named_secrets(&self) -> Result<NamedTeamSecrets> {
        if self.kind != StoredTeamKind::Named {
            return Err(Error::InvalidAccount("team is not named"));
        }
        Ok(NamedTeamSecrets {
            member_min: SecretSeed::new(self.member_min),
            member: SecretSeed::new(self.member),
            admin: SecretSeed::new(self.admin),
            owner: SecretSeed::new(self.owner),
            removal_key: SecretSeed::new(
                self.removal_key
                    .ok_or(Error::InvalidAccount("named team has no removal key"))?,
            ),
            team_name_commitment_key: self.name_commitment.ok_or(Error::InvalidAccount(
                "named team has no name commitment key",
            ))?,
        })
    }

    fn adhoc_secrets(&self) -> Result<AdHocTeamSecrets> {
        if self.kind != StoredTeamKind::AdHoc {
            return Err(Error::InvalidAccount("team is not ad-hoc"));
        }
        Ok(AdHocTeamSecrets {
            member_min: SecretSeed::new(self.member_min),
            member: SecretSeed::new(self.member),
            admin: SecretSeed::new(self.admin),
            owner: SecretSeed::new(self.owner),
        })
    }
}

impl Drop for StoredTeam {
    fn drop(&mut self) {
        self.member_min.zeroize();
        self.member.zeroize();
        self.admin.zeroize();
        self.owner.zeroize();
        self.removal_key.zeroize();
        self.name_commitment.zeroize();
        for member in &mut self.federated_members {
            member.removal_key.zeroize();
        }
    }
}

impl AccountVault<'_> {
    pub fn team_aliases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| key.strip_prefix("team.").map(str::to_owned))
            .collect())
    }
    fn contains_team(&mut self, alias: &str) -> Result<bool> {
        validate_name(alias)?;
        Ok(self.store.keys()?.iter().any(|key| key == &team_key(alias)))
    }

    pub(super) fn team(&mut self, alias: &str) -> Result<StoredTeam> {
        validate_name(alias)?;
        let bytes = self
            .store
            .get(&team_key(alias))
            .map_err(|error| match error {
                foks_keystore::Error::Missing => Error::AccountMissing,
                other => Error::Keystore(other),
            })?;
        let team: StoredTeam = serde_json::from_slice(&bytes)?;
        validate_stored_team(&team, alias)?;
        Ok(team)
    }

    pub(super) fn put_team(&mut self, team: &StoredTeam) -> Result<()> {
        validate_stored_team(team, &team.alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(team)?);
        self.store.put(&team_key(&team.alias), &encoded)?;
        Ok(())
    }
}
fn validate_stored_team(team: &StoredTeam, expected_alias: &str) -> Result<()> {
    if team.version != CREDENTIAL_VERSION || team.alias != expected_alias {
        return Err(Error::InvalidAccount(
            "team version or alias binding changed",
        ));
    }
    validate_name(&team.alias)?;
    validate_name(&team.account_alias)?;
    let id = EntityId::from_bytes(team.team_id.clone())?;
    let derived = match team.kind {
        StoredTeamKind::Named => {
            if team.name.as_deref().is_none_or(str::is_empty)
                || team.removal_key.is_none()
                || team.name_commitment.is_none()
            {
                return Err(Error::InvalidAccount("named team material is incomplete"));
            }
            team.named_secrets()?.team_id()?
        }
        StoredTeamKind::AdHoc => {
            if team.name.is_some() || team.removal_key.is_some() || team.name_commitment.is_some() {
                return Err(Error::InvalidAccount("ad-hoc team has named-team material"));
            }
            team.adhoc_secrets()?.team_id()?
        }
    };
    if id != derived {
        return Err(Error::InvalidAccount(
            "team ID does not match protected PTKs",
        ));
    }
    let mut bindings = std::collections::BTreeSet::new();
    for member in &team.federated_members {
        validate_name(&member.remote_profile)?;
        validate_name(&member.remote_team_alias)?;
        let host = EntityId::from_bytes(member.remote_host_id.clone())?
            .require_type(foks_proto::ENTITY_HOST)?;
        let party = EntityId::from_bytes(member.remote_team_id.clone())?;
        if !matches!(
            party.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        ) || !bindings.insert((host.into_bytes(), party.into_bytes()))
            || member.destination.role() == Role::NONE
            || member.active != member.operation_id.is_some()
        {
            return Err(Error::InvalidAccount(
                "federated team membership binding is invalid",
            ));
        }
    }
    Ok(())
}
