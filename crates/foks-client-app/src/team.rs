use super::*;

pub(super) const BACKGROUND_TEAM_ALIAS_PREFIX: &str = "bg_";

pub(super) fn is_background_team_alias(alias: &str) -> bool {
    alias.starts_with(BACKGROUND_TEAM_ALIAS_PREFIX)
}

#[cfg(test)]
static TEST_FAIL_AFTER_MEMBER_EDIT_COMMIT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
static TEST_FAIL_AFTER_DISCOVERY_PERSIST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
        if stored.origin != StoredTeamOrigin::CreatedHere {
            return Err(Error::InvalidAccount(
                "a discovered team has no local creation to resume",
            ));
        }
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
                Ok(TeamSummary::from_stored(alias, &team))
            })
            .collect()
    }

    /// Discovers teams that the selected account can currently authenticate,
    /// gives previously unknown teams stable local aliases, and persists the
    /// bindings before returning them. Each team record is independent, so a
    /// retry after interruption reuses every binding already written.
    pub fn discover_teams(
        &self,
        account_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<TeamDiscoveryReport> {
        self.profile.require(Capability::Teams)?;
        let account = vault.account(account_alias)?;
        let host = self.pinned_host()?;
        let user = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        let graph = self.client.discover_local_team_graph(
            &host,
            &account.credential,
            &user.verified,
            &user.puks,
        )?;

        let mut discovered = std::collections::BTreeMap::<Vec<u8>, StoredTeam>::new();
        for authenticated in graph.teams {
            if authenticated.verified.host() != host.host_id() {
                return Err(foks_client::Error::TeamBinding(
                    "team discovery response changed the authenticated host",
                )
                .into());
            }
            // The authenticated graph also contains teams reachable through
            // another local team. Ordinary desktop store operations reopen a
            // team with this account's user credential, so persist only teams
            // whose current authenticated roster directly contains that user.
            if !authenticated
                .verified
                .members()
                .iter()
                .any(|member| member.party == *user.verified.uid() && member.scoped_host.is_none())
            {
                continue;
            }
            let direct = self.client.load_and_pin_team(
                &host,
                &account.credential,
                &user.verified,
                &user.puks,
                authenticated.verified.team(),
            )?;
            if direct.verified.team() != authenticated.verified.team()
                || direct.verified.host() != authenticated.verified.host()
            {
                return Err(foks_client::Error::TeamBinding(
                    "direct team discovery changed authenticated identity",
                )
                .into());
            }
            let identity = StoredTeam::discovered(account_alias, &direct)?;
            let key = identity.team_id.clone();
            if let Some(prior) = discovered.get(&key) {
                if prior.kind != identity.kind || prior.name != identity.name {
                    return Err(foks_client::Error::TeamBinding(
                        "one discovered team has conflicting authenticated identities",
                    )
                    .into());
                }
            } else {
                discovered.insert(key, identity);
            }
        }

        let mut teams = Vec::with_capacity(discovered.len());
        for identity in discovered.into_values() {
            let alias = discovery_alias(vault, &identity)?;
            let stored = match vault.team(&alias) {
                Ok(mut existing) => {
                    bind_existing_discovery(&existing, &identity)?;
                    if !existing.active {
                        existing.active = true;
                        vault.put_team(&existing)?;
                    }
                    existing
                }
                Err(Error::AccountMissing) => {
                    let mut identity = identity;
                    identity.alias = alias.clone();
                    vault.put_team(&identity)?;
                    identity
                }
                Err(error) => return Err(error),
            };
            teams.push(TeamSummary::from_stored(alias, &stored));

            #[cfg(test)]
            if TEST_FAIL_AFTER_DISCOVERY_PERSIST.swap(false, std::sync::atomic::Ordering::SeqCst) {
                return Err(Error::InvalidAccount(
                    "test interrupted team discovery after one persisted binding",
                ));
            }
        }
        teams.sort_by(|left, right| left.alias.cmp(&right.alias));
        Ok(TeamDiscoveryReport {
            account_alias: account_alias.to_owned(),
            teams,
        })
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

    /// Returns one live metadata snapshot for a team store. Every component
    /// of the caller's store reference is checked before network access so a
    /// cursor or selection cannot be replayed against another party.
    pub fn list_team_kv_metadata(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvCatalogReport> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Kv)?;
        let stored = vault.team(team_alias)?;
        if !stored.active {
            return Err(Error::InvalidAccount("team creation is still pending"));
        }
        if stored.account_alias != account_alias || hex(&stored.team_id) != team_id_hex {
            return Err(Error::InvalidAccount(
                "team store reference does not match the stored team",
            ));
        }
        let account = vault.account(account_alias)?;
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
        let tree = self.client.list_team_kv_metadata(
            &host,
            &account.credential,
            &team,
            &self.paths.soft_database,
        )?;
        KvCatalogReport::from_tree(&tree)
    }

    pub fn list_team_members(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<TeamMemberSummary>> {
        self.profile.require(Capability::Teams)?;
        let context = self.load_local_team_context(team_alias, vault)?;
        context
            .team
            .verified
            .members()
            .iter()
            .map(|member| {
                let local_user = member.scoped_host.is_none()
                    && member.party.entity_type() == foks_proto::ENTITY_USER;
                let username = context
                    .users
                    .get(member.party.as_bytes())
                    .map(|user| String::from_utf8(user.username_utf8().to_vec()))
                    .transpose()
                    .map_err(|_| Error::InvalidAccount("authenticated username is not UTF-8"))?;
                let party_kind = match member.party.entity_type() {
                    foks_proto::ENTITY_USER => "user",
                    foks_proto::ENTITY_NAMED_TEAM => "named-team",
                    foks_proto::ENTITY_AD_HOC_TEAM => "ad-hoc-team",
                    _ => "unknown",
                };
                Ok(TeamMemberSummary {
                    username,
                    party_id_hex: hex(member.party.as_bytes()),
                    scoped_host_id_hex: member
                        .scoped_host
                        .as_ref()
                        .map(|host| hex(host.as_bytes())),
                    party_kind: party_kind.to_owned(),
                    source_role: TeamMemberRole::from_role(member.source_role)?,
                    destination_role: TeamMemberRole::from_role(member.role)?,
                    generation: member.generation,
                    locally_manageable: local_user,
                })
            })
            .collect()
    }

    pub fn add_local_team_member(
        &self,
        team_alias: &str,
        username: &str,
        destination: TeamMemberRole,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let context = self.load_local_team_context(team_alias, vault)?;
        let uid = self.client.resolve_username(
            &context.host,
            &context.account.credential,
            username,
            true,
        )?;
        let target = self.client.load_and_pin_open_local_user(
            &context.host,
            &context.account.credential,
            &uid,
        )?;
        self.add_verified_local_team_member(team_alias, &target, destination, vault, master_key)
    }

    pub(super) fn add_verified_local_team_member(
        &self,
        team_alias: &str,
        target: &foks_verify::VerifiedUserState,
        destination: TeamMemberRole,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let mut stored = vault.team(team_alias)?;
        require_named_active_team(&stored)?;
        if stored.local_members.iter().any(|member| !member.active)
            || stored
                .invitation_members
                .iter()
                .any(|member| !member.membership.active)
            || vault.team_member_edit(team_alias)?.is_some()
            || vault.team_rekey(team_alias)?.is_some()
        {
            return Err(Error::InvalidAccount(
                "team has a pending membership mutation; resume it first",
            ));
        }
        let context = self.load_local_team_context(team_alias, vault)?;
        if target.host() != context.host.host_id()
            || target.uid() == &context.account.credential.uid
            || context
                .team
                .verified
                .members()
                .iter()
                .any(|m| &m.party == target.uid() && m.source_role == Role::OWNER)
        {
            return Err(foks_client::Error::TeamRequest(
                "target is already a member or belongs to another host",
            )
            .into());
        }
        if target.tree_root() != context.team.verified.tree_root() {
            return Err(foks_client::Error::TeamRequest(
                "team addition snapshot does not match the latest authenticated Merkle root",
            )
            .into());
        }
        let canonical_username = String::from_utf8(target.username_utf8().to_vec())
            .map_err(|_| Error::InvalidAccount("authenticated username is not UTF-8"))?;
        let removal_key = SecretSeed::new(random_array()?);
        let request = foks_client::AddLocalTeamMemberRequest {
            target_user: target,
            destination_role: destination.role(),
            removal_key: &removal_key,
        };
        let plan = self.client.local_team_member_addition_plan(
            &context.account.credential.uid,
            &context.team_id,
            &context.team,
            &request,
        )?;
        stored.local_members.push(StoredLocalMembership {
            username: canonical_username.clone(),
            target_id: plan.target_id.as_bytes().to_vec(),
            target_verify_key: plan.target_verify_key.as_bytes().to_vec(),
            target_generation: plan.target_generation,
            target_source_role: StoredTeamRole::from_role(plan.target_source_role),
            destination_role: StoredTeamRole::from_role(plan.destination_role),
            removal_key_commitment: plan.removal_key_commitment,
            removal_key: *removal_key.as_bytes(),
            expected_seqno: plan.expected_seqno,
            operation_id: plan.operation_id,
            active: false,
        });
        vault.put_team(&stored)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let added = self.client.add_local_user_to_named_team_durable(
            &context.host,
            &context.account.credential,
            &context.team_id,
            &plan,
            &request,
            &mut mutations,
        )?;
        mark_local_addition_active(vault, team_alias, &plan.operation_id)?;
        Ok(team_member_report(
            team_alias,
            &context.team_id,
            &canonical_username,
            &plan.target_id,
            Some(destination),
            &plan.operation_id,
            added.authenticated.verified.chain_seqno(),
        ))
    }

    pub fn resume_local_team_member_addition(
        &self,
        team_alias: &str,
        username: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let stored = vault.team(team_alias)?;
        require_named_active_team(&stored)?;
        let pending = stored
            .local_members
            .iter()
            .find(|member| !member.active && member.username == username)
            .ok_or(Error::InvalidAccount(
                "pending local team-member addition is missing",
            ))?;
        let plan = local_addition_plan(pending)?;
        let removal_key = SecretSeed::new(pending.removal_key);
        let context = self.load_local_team_context(team_alias, vault)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let added = if HardStateStore::open(&self.paths.hard_database)?
            .team_mutation(&plan.operation_id)?
            .is_some()
        {
            self.client.resume_durable_local_team_member_addition(
                &context.host,
                &context.account.credential,
                &context.team_id,
                &plan,
                &mut mutations,
            )?
        } else {
            let target = self.client.load_and_pin_open_local_user(
                &context.host,
                &context.account.credential,
                &plan.target_id,
            )?;
            let request = foks_client::AddLocalTeamMemberRequest {
                target_user: &target,
                destination_role: plan.destination_role,
                removal_key: &removal_key,
            };
            let current = self.client.local_team_member_addition_plan(
                &context.account.credential.uid,
                &context.team_id,
                &context.team,
                &request,
            )?;
            if current != plan {
                return Err(foks_client::Error::OperationBinding(
                    "pending addition no longer matches the authenticated team head",
                )
                .into());
            }
            self.client.add_local_user_to_named_team_durable(
                &context.host,
                &context.account.credential,
                &context.team_id,
                &plan,
                &request,
                &mut mutations,
            )?
        };
        mark_local_addition_active(vault, team_alias, &plan.operation_id)?;
        Ok(team_member_report(
            team_alias,
            &context.team_id,
            username,
            &plan.target_id,
            Some(TeamMemberRole::from_role(plan.destination_role)?),
            &plan.operation_id,
            added.authenticated.verified.chain_seqno(),
        ))
    }

    pub fn demote_local_team_member(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        destination: TeamMemberRole,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.change_local_team_member(
            team_alias,
            party_id_hex,
            Some(destination),
            None,
            vault,
            master_key,
        )
    }

    pub fn remove_local_team_member(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.change_local_team_member(team_alias, party_id_hex, None, None, vault, master_key)
    }

    pub(super) fn demote_local_team_member_with_parties(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        destination: TeamMemberRole,
        parties: &std::collections::BTreeMap<
            super::runtime::TeamRefreshPartyKey,
            super::runtime::TeamRefreshParty,
        >,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.change_local_team_member(
            team_alias,
            party_id_hex,
            Some(destination),
            Some(parties),
            vault,
            master_key,
        )
    }

    pub(super) fn remove_local_team_member_with_parties(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        parties: &std::collections::BTreeMap<
            super::runtime::TeamRefreshPartyKey,
            super::runtime::TeamRefreshParty,
        >,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.change_local_team_member(
            team_alias,
            party_id_hex,
            None,
            Some(parties),
            vault,
            master_key,
        )
    }

    pub fn resume_local_team_member_edit(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.resume_local_team_member_edit_with_parties(team_alias, None, vault, master_key)
    }

    pub(super) fn resume_local_team_member_edit_with_authenticated_parties(
        &self,
        team_alias: &str,
        parties: &std::collections::BTreeMap<
            super::runtime::TeamRefreshPartyKey,
            super::runtime::TeamRefreshParty,
        >,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.resume_local_team_member_edit_with_parties(
            team_alias,
            Some(parties),
            vault,
            master_key,
        )
    }

    fn resume_local_team_member_edit_with_parties(
        &self,
        team_alias: &str,
        supplied_parties: Option<
            &std::collections::BTreeMap<
                super::runtime::TeamRefreshPartyKey,
                super::runtime::TeamRefreshParty,
            >,
        >,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let pending = vault
            .team_member_edit(team_alias)?
            .ok_or(Error::InvalidAccount("pending team member edit is missing"))?;
        let context = self.load_local_team_context(team_alias, vault)?;
        validate_edit_context(&pending, &context)?;
        let seeds = pending
            .rotations
            .iter()
            .map(|rotation| SecretSeed::new(rotation.seed))
            .collect::<Vec<_>>();
        let rotations = pending
            .rotations
            .iter()
            .zip(&seeds)
            .map(|(rotation, seed)| {
                Ok(foks_client::TeamPtkRotationSeed {
                    role: rotation.role.role()?,
                    seed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let reserved_sequence_is_observable =
            context.team.verified.chain_seqno() >= pending.expected_seqno;
        let result = if reserved_sequence_is_observable {
            self.client.finish_recorded_team_member_change(
                &context.host,
                &context.account.credential,
                foks_client::TeamMutationRecovery {
                    team: &context.team_id,
                    expected_seqno: pending.expected_seqno,
                    expected_operation_id: &pending.operation_id,
                },
                pending.removal_key_commitment,
                &rotations,
                &mut mutations,
            )
        } else {
            let target_id = EntityId::from_bytes(pending.target_id.clone())?;
            let (target_member, target_user, remaining) =
                local_edit_parties(&context, &target_id, supplied_parties)?;
            if target_member.source_role != pending.source_role.role()? {
                return Err(foks_client::Error::OperationBinding(
                    "pending member edit source role changed",
                )
                .into());
            }
            let destination = pending.destination_role.role()?;
            let request = foks_client::ChangeTeamMemberRequest {
                target: foks_client::TeamMemberSelector {
                    party: &target_id,
                    host: None,
                    source_role: target_member.source_role,
                },
                destination_role: destination,
                replacement: (destination != Role::NONE)
                    .then_some(foks_client::VerifiedMemberParty::User(target_user)),
                rotations: &rotations,
                remaining_parties: &remaining,
            };
            let current_id = self
                .client
                .change_team_member_and_rotate_ptks_operation_id(
                    &context.account.credential.uid,
                    &context.team_id,
                    &context.team,
                    &request,
                )?;
            if current_id != pending.operation_id {
                return Err(foks_client::Error::OperationBinding(
                    "pending member edit no longer matches the authenticated team head",
                )
                .into());
            }
            if HardStateStore::open(&self.paths.hard_database)?
                .team_mutation(&pending.operation_id)?
                .is_some()
            {
                self.client.resume_change_team_member_and_rotate_ptks(
                    &context.host,
                    &context.account.credential,
                    foks_client::TeamMutationRecovery {
                        team: &context.team_id,
                        expected_seqno: pending.expected_seqno,
                        expected_operation_id: &pending.operation_id,
                    },
                    &request,
                    &mut mutations,
                )
            } else {
                self.client.change_team_member_and_rotate_ptks(
                    &context.host,
                    &context.account.credential,
                    &context.team_id,
                    &request,
                    &mut mutations,
                )
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(error @ foks_client::Error::OperationBinding(_))
                if reserved_sequence_is_observable =>
            {
                // Another administrator won the reserved sequence. Make both
                // durable layers terminal before surfacing the conflict so a
                // single lost race cannot block all future member management.
                self.client.supersede_recorded_team_member_change(
                    &context.host,
                    &context.account.credential,
                    &context.team_id,
                    pending.expected_seqno,
                    &pending.operation_id,
                    &mut mutations,
                )?;
                vault.remove_team_member_edit(team_alias)?;
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        };
        finish_stored_team_member_edit(vault, &pending)?;
        Ok(team_member_report(
            team_alias,
            &context.team_id,
            &pending.target_username,
            &EntityId::from_bytes(pending.target_id.clone())?,
            optional_member_role(pending.destination_role.role()?)?,
            &pending.operation_id,
            result.authenticated.verified.chain_seqno(),
        ))
    }

    fn change_local_team_member(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        destination: Option<TeamMemberRole>,
        supplied_parties: Option<
            &std::collections::BTreeMap<
                super::runtime::TeamRefreshPartyKey,
                super::runtime::TeamRefreshParty,
            >,
        >,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let stored = vault.team(team_alias)?;
        require_named_active_team(&stored)?;
        if stored.local_members.iter().any(|member| !member.active)
            || stored
                .invitation_members
                .iter()
                .any(|member| !member.membership.active)
            || vault.team_member_edit(team_alias)?.is_some()
            || vault.team_rekey(team_alias)?.is_some()
        {
            return Err(Error::InvalidAccount(
                "team has a pending membership mutation; resume it first",
            ));
        }
        let context = self.load_local_team_context(team_alias, vault)?;
        let target_id = entity_id_from_hex(party_id_hex)?;
        target_id.clone().require_type(foks_proto::ENTITY_USER)?;
        let (target_member, target_user, remaining) =
            local_edit_parties(&context, &target_id, supplied_parties)?;
        let destination_role = destination.map_or(Role::NONE, TeamMemberRole::role);
        if destination_role >= target_member.role {
            return Err(foks_client::Error::TeamRequest(
                "member edits must be strict demotions; promotion needs a separate key-distribution flow",
            )
            .into());
        }
        let replacement = (destination_role != Role::NONE)
            .then_some(foks_client::VerifiedMemberParty::User(target_user));
        let selector = foks_client::TeamMemberSelector {
            party: &target_id,
            host: None,
            source_role: target_member.source_role,
        };
        let roles = self.client.team_member_rotation_roles(
            &context.team,
            selector,
            destination_role,
            replacement,
        )?;
        let seeds = roles
            .iter()
            .map(|_| random_array().map(SecretSeed::new))
            .collect::<Result<Vec<_>>>()?;
        let rotations = roles
            .iter()
            .zip(&seeds)
            .map(|(role, seed)| foks_client::TeamPtkRotationSeed { role: *role, seed })
            .collect::<Vec<_>>();
        let request = foks_client::ChangeTeamMemberRequest {
            target: selector,
            destination_role,
            replacement,
            rotations: &rotations,
            remaining_parties: &remaining,
        };
        let operation_id = self
            .client
            .change_team_member_and_rotate_ptks_operation_id(
                &context.account.credential.uid,
                &context.team_id,
                &context.team,
                &request,
            )?;
        let removal_key_commitment =
            target_member
                .removal_key_commitment
                .ok_or(foks_client::Error::TeamBinding(
                    "target team member has no removal-key commitment",
                ))?;
        let expected_seqno = context
            .team
            .verified
            .chain_seqno()
            .checked_add(1)
            .ok_or(foks_client::Error::TeamRequest("team sequence overflow"))?;
        let canonical_username = String::from_utf8(target_user.username_utf8().to_vec())
            .map_err(|_| Error::InvalidAccount("authenticated username is not UTF-8"))?;
        let pending = StoredTeamMemberEdit {
            version: CREDENTIAL_VERSION,
            team_alias: team_alias.to_owned(),
            team_id: context.team_id.as_bytes().to_vec(),
            actor_uid: context.account.credential.uid.as_bytes().to_vec(),
            actor_device_id: context
                .account
                .credential
                .public_material()?
                .id
                .as_bytes()
                .to_vec(),
            target_username: canonical_username.clone(),
            target_id: target_id.as_bytes().to_vec(),
            source_role: StoredTeamRole::from_role(target_member.source_role),
            destination_role: StoredTeamDestinationRole::from_role(destination_role),
            removal_key_commitment,
            expected_seqno,
            operation_id,
            rotations: roles
                .iter()
                .zip(&seeds)
                .map(|(role, seed)| StoredTeamPtkRotation {
                    role: StoredTeamRole::from_role(*role),
                    seed: *seed.as_bytes(),
                })
                .collect(),
        };
        vault.put_team_member_edit(&pending)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let result = self.client.change_team_member_and_rotate_ptks(
            &context.host,
            &context.account.credential,
            &context.team_id,
            &request,
            &mut mutations,
        )?;
        #[cfg(test)]
        if TEST_FAIL_AFTER_MEMBER_EDIT_COMMIT.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::InvalidAccount(
                "test failpoint after authenticated team-member commit",
            ));
        }
        finish_stored_team_member_edit(vault, &pending)?;
        Ok(team_member_report(
            team_alias,
            &context.team_id,
            &canonical_username,
            &target_id,
            destination,
            &operation_id,
            result.authenticated.verified.chain_seqno(),
        ))
    }

    pub(super) fn load_local_team_context(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<LocalTeamContext> {
        let stored = vault.team(team_alias)?;
        if !stored.active {
            return Err(Error::InvalidAccount("team creation is still pending"));
        }
        let team_id = EntityId::from_bytes(stored.team_id.clone())?;
        let host = self.pinned_host()?;
        let mut last_race = None;
        for _ in 0..3 {
            let account = vault.account(&stored.account_alias)?;
            let actor = self
                .client
                .authenticate_and_pin(&host, &account.credential)?;
            let team = self.client.load_and_pin_team(
                &host,
                &account.credential,
                &actor.verified,
                &actor.puks,
                &team_id,
            )?;
            let mut users = std::collections::BTreeMap::new();
            users.insert(
                actor.verified.uid().as_bytes().to_vec(),
                actor.verified.clone(),
            );
            for member in team.verified.members() {
                if member.scoped_host.is_none()
                    && member.party.entity_type() == foks_proto::ENTITY_USER
                    && !users.contains_key(member.party.as_bytes())
                {
                    let user = self.client.load_and_pin_user_as_local_team(
                        &host,
                        &account.credential,
                        &member.party,
                        &team.view_token,
                    )?;
                    users.insert(member.party.as_bytes().to_vec(), user);
                }
            }
            if actor.verified.tree_root() == team.verified.tree_root()
                && users
                    .values()
                    .all(|user| user.tree_root() == team.verified.tree_root())
            {
                return Ok(LocalTeamContext {
                    host,
                    account,
                    actor,
                    team_id,
                    team,
                    users,
                });
            }
            last_race = Some(foks_client::Error::TeamRequest(
                "team roster snapshot does not match the latest authenticated Merkle root",
            ));
        }
        Err(last_race.expect("bounded roster loop executes").into())
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
        // Both team kinds provision this low-visibility Member PTK.
        // Share root discovery while retaining Owner-only root writes.
        let tree = session.ensure_root(Role::member(-0x4000), Role::OWNER)?;
        Ok(TeamSyncReport::new(team_alias, &authenticated, &tree))
    }
}

pub(super) struct LocalTeamContext {
    pub(super) host: foks_client::PinnedHost,
    pub(super) account: LoadedAccount,
    pub(super) actor: AuthenticatedUserOutcome,
    pub(super) team_id: EntityId,
    pub(super) team: AuthenticatedTeamOutcome,
    pub(super) users: std::collections::BTreeMap<Vec<u8>, foks_verify::VerifiedUserState>,
}

fn discovery_alias(vault: &mut AccountVault<'_>, identity: &StoredTeam) -> Result<String> {
    let aliases = vault.team_aliases()?;
    let mut exact = Vec::new();
    for alias in &aliases {
        if vault.team(alias)?.team_id == identity.team_id {
            exact.push(alias.clone());
        }
    }
    match exact.as_slice() {
        // Team refresh rejects duplicate local aliases for one team ID. Reuse
        // that sole binding here; `bind_existing_discovery` then verifies it
        // belongs to the selected account rather than silently rebinding it.
        [alias] => return Ok(alias.clone()),
        [] => {}
        _ => {
            return Err(Error::InvalidAccount(
                "more than one local alias names the discovered team",
            ))
        }
    }

    let occupied = aliases
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if !occupied.contains(&identity.alias) {
        return Ok(identity.alias.clone());
    }
    let id = hex(&identity.team_id);
    for suffix_bytes in (6..=16).step_by(2) {
        let suffix = &id[..suffix_bytes * 2];
        let keep = 64usize
            .checked_sub(suffix.len() + 1)
            .ok_or(Error::InvalidAccount(
                "discovered team alias exceeds maximum length",
            ))?;
        let base = &identity.alias[..identity.alias.len().min(keep)];
        let candidate = format!("{base}_{suffix}");
        if !occupied.contains(&candidate) {
            validate_name(&candidate)?;
            return Ok(candidate);
        }
    }
    Err(Error::InvalidAccount(
        "discovered team alias namespace is exhausted",
    ))
}

fn bind_existing_discovery(existing: &StoredTeam, identity: &StoredTeam) -> Result<()> {
    if existing.team_id != identity.team_id
        || existing.account_alias != identity.account_alias
        || existing.kind != identity.kind
        || existing.name != identity.name
    {
        return Err(foks_client::Error::TeamBinding(
            "stored team account or identity differs from authenticated discovery",
        )
        .into());
    }
    Ok(())
}

fn require_named_active_team(team: &StoredTeam) -> Result<()> {
    if !team.active || team.kind != StoredTeamKind::Named {
        return Err(Error::InvalidAccount(
            "member management requires an active named team",
        ));
    }
    Ok(())
}

fn local_edit_parties<'a>(
    context: &'a LocalTeamContext,
    target_id: &EntityId,
    supplied_parties: Option<
        &'a std::collections::BTreeMap<
            super::runtime::TeamRefreshPartyKey,
            super::runtime::TeamRefreshParty,
        >,
    >,
) -> Result<(
    &'a foks_verify::VerifiedTeamMemberState,
    &'a foks_verify::VerifiedUserState,
    Vec<foks_client::VerifiedMemberParty<'a>>,
)> {
    let mut matches = context
        .team
        .verified
        .members()
        .iter()
        .filter(|member| member.party == *target_id && member.scoped_host.is_none());
    let target = matches.next().ok_or(foks_client::Error::TeamRequest(
        "target user is not a current local team member",
    ))?;
    if matches.next().is_some() {
        return Err(foks_client::Error::TeamBinding("target local membership is ambiguous").into());
    }
    let target_user =
        context
            .users
            .get(target_id.as_bytes())
            .ok_or(foks_client::Error::TeamBinding(
                "target local user state is unavailable",
            ))?;
    if target.party.entity_type() != foks_proto::ENTITY_USER {
        return Err(foks_client::Error::TeamRequest(
            "only an authenticated local user row can be edited",
        )
        .into());
    }
    let mut remaining = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for member in context.team.verified.members() {
        if (member.party == context.account.credential.uid && member.scoped_host.is_none())
            || (member.party == *target_id && member.scoped_host.is_none())
        {
            continue;
        }
        let key = (
            member.party.as_bytes().to_vec(),
            member
                .scoped_host
                .as_ref()
                .map(|host| host.as_bytes().to_vec()),
        );
        if !seen.insert(key.clone()) {
            return Err(
                foks_client::Error::TeamBinding("authenticated roster party is ambiguous").into(),
            );
        }
        if member.scoped_host.is_none() && member.party.entity_type() == foks_proto::ENTITY_USER {
            let user = context.users.get(member.party.as_bytes()).ok_or(
                foks_client::Error::TeamBinding("remaining local user state is unavailable"),
            )?;
            remaining.push(foks_client::VerifiedMemberParty::User(user));
        } else {
            let party = supplied_parties
                .and_then(|parties| parties.get(&key))
                .ok_or(foks_client::Error::TeamBinding(
                    "remaining non-local roster party lacks an authenticated recipient",
                ))?;
            remaining.push(party.verified());
        }
    }
    Ok((target, target_user, remaining))
}

fn validate_edit_context(edit: &StoredTeamMemberEdit, context: &LocalTeamContext) -> Result<()> {
    if edit.team_id != context.team_id.as_bytes()
        || edit.actor_uid != context.account.credential.uid.as_bytes()
        || edit.actor_uid != context.actor.verified.uid().as_bytes()
        || edit.actor_device_id != context.account.credential.public_material()?.id.as_bytes()
    {
        return Err(foks_client::Error::OperationBinding(
            "pending member edit belongs to another team or credential",
        )
        .into());
    }
    Ok(())
}

pub(super) fn local_addition_plan(
    stored: &StoredLocalMembership,
) -> Result<foks_client::LocalTeamMemberAdditionPlan> {
    Ok(foks_client::LocalTeamMemberAdditionPlan {
        target_id: EntityId::from_bytes(stored.target_id.clone())?,
        target_verify_key: EntityId::from_bytes(stored.target_verify_key.clone())?,
        target_generation: stored.target_generation,
        target_source_role: stored.target_source_role.role()?,
        destination_role: stored.destination_role.role()?,
        removal_key_commitment: stored.removal_key_commitment,
        expected_seqno: stored.expected_seqno,
        operation_id: stored.operation_id,
    })
}

fn mark_local_addition_active(
    vault: &mut AccountVault<'_>,
    team_alias: &str,
    operation_id: &[u8; 16],
) -> Result<()> {
    let mut team = vault.team(team_alias)?;
    let mut matches = team
        .local_members
        .iter_mut()
        .filter(|member| member.operation_id == *operation_id);
    let member = matches.next().ok_or(Error::InvalidAccount(
        "completed local-team addition has no protected binding",
    ))?;
    if matches.next().is_some() {
        return Err(Error::InvalidAccount(
            "completed local-team addition binding is ambiguous",
        ));
    }
    member.active = true;
    vault.put_team(&team)
}

fn finish_stored_team_member_edit(
    vault: &mut AccountVault<'_>,
    edit: &StoredTeamMemberEdit,
) -> Result<()> {
    let mut team = vault.team(&edit.team_alias)?;
    if team.team_id != edit.team_id {
        return Err(foks_client::Error::OperationBinding(
            "completed member edit belongs to another protected team",
        )
        .into());
    }
    let destination = edit.destination_role.role()?;
    if destination == Role::NONE {
        team.local_members
            .retain(|member| member.target_id != edit.target_id);
    } else if let Some(member) = team
        .local_members
        .iter_mut()
        .find(|member| member.target_id == edit.target_id)
    {
        member.destination_role = StoredTeamRole::from_role(destination);
    }
    // Update the durable projection before deleting the intent. A crash
    // between these writes is harmless because replay is idempotent.
    vault.put_team(&team)?;
    vault.remove_team_member_edit(&edit.team_alias)
}

fn optional_member_role(role: Role) -> Result<Option<TeamMemberRole>> {
    if role == Role::NONE {
        Ok(None)
    } else {
        TeamMemberRole::from_role(role).map(Some)
    }
}

fn team_member_report(
    team_alias: &str,
    team_id: &EntityId,
    username: &str,
    target: &EntityId,
    destination: Option<TeamMemberRole>,
    operation_id: &[u8; 16],
    sequence: u64,
) -> TeamMemberMutationReport {
    TeamMemberMutationReport {
        team_alias: team_alias.to_owned(),
        team_id_hex: hex(team_id.as_bytes()),
        target_username: username.to_owned(),
        target_uid_hex: hex(target.as_bytes()),
        destination_role: destination,
        team_chain_sequence: sequence,
        operation_id_hex: hex(operation_id),
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

impl TeamSummary {
    fn from_stored(alias: String, team: &StoredTeam) -> Self {
        Self {
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
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamDiscoveryReport {
    pub account_alias: String,
    pub teams: Vec<TeamSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamSyncReport {
    pub alias: String,
    pub team_id_hex: String,
    pub team_chain_sequence: u64,
    pub directories: usize,
    pub entries: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TeamMemberRole {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl TeamMemberRole {
    pub(super) fn role(self) -> Role {
        match self {
            Self::Member { visibility } => Role::member(visibility),
            Self::Admin => Role::ADMIN,
            Self::Owner => Role::OWNER,
        }
    }

    fn from_role(role: Role) -> Result<Self> {
        match role.kind() {
            foks_proto::RoleType::Member => Ok(Self::Member {
                visibility: role.visibility().unwrap_or(0),
            }),
            foks_proto::RoleType::Admin => Ok(Self::Admin),
            foks_proto::RoleType::Owner => Ok(Self::Owner),
            _ => Err(Error::InvalidAccount("team member has an invalid role")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamMemberSummary {
    pub username: Option<String>,
    pub party_id_hex: String,
    pub scoped_host_id_hex: Option<String>,
    pub party_kind: String,
    pub source_role: TeamMemberRole,
    pub destination_role: TeamMemberRole,
    pub generation: u64,
    pub locally_manageable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TeamMemberMutationReport {
    pub team_alias: String,
    pub team_id_hex: String,
    pub target_username: String,
    pub target_uid_hex: String,
    pub destination_role: Option<TeamMemberRole>,
    pub team_chain_sequence: u64,
    pub operation_id_hex: String,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum StoredTeamOrigin {
    CreatedHere,
    Discovered,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredTeam {
    pub(super) version: u32,
    pub(super) alias: String,
    pub(super) account_alias: String,
    pub(super) origin: StoredTeamOrigin,
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
    #[serde(default)]
    pub(super) local_members: Vec<StoredLocalMembership>,
    #[serde(default)]
    pub(super) invitation_members: Vec<crate::invitations::StoredInvitationMembership>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredLocalMembership {
    pub(super) username: String,
    pub(super) target_id: Vec<u8>,
    pub(super) target_verify_key: Vec<u8>,
    pub(super) target_generation: u64,
    pub(super) target_source_role: StoredTeamRole,
    pub(super) destination_role: StoredTeamRole,
    pub(super) removal_key_commitment: [u8; 32],
    pub(super) removal_key: [u8; 32],
    pub(super) expected_seqno: u64,
    pub(super) operation_id: [u8; 16],
    pub(super) active: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredTeamMemberEdit {
    pub(super) version: u32,
    pub(super) team_alias: String,
    pub(super) team_id: Vec<u8>,
    pub(super) actor_uid: Vec<u8>,
    pub(super) actor_device_id: Vec<u8>,
    pub(super) target_username: String,
    pub(super) target_id: Vec<u8>,
    pub(super) source_role: StoredTeamRole,
    pub(super) destination_role: StoredTeamDestinationRole,
    pub(super) removal_key_commitment: [u8; 32],
    pub(super) expected_seqno: u64,
    pub(super) operation_id: [u8; 16],
    pub(super) rotations: Vec<StoredTeamPtkRotation>,
}

impl Drop for StoredTeamMemberEdit {
    fn drop(&mut self) {
        for rotation in &mut self.rotations {
            rotation.seed.zeroize();
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredFederatedExpulsion {
    pub(super) version: u32,
    pub(super) local_team_alias: String,
    pub(super) local_team_id: Vec<u8>,
    pub(super) local_host_id: Vec<u8>,
    pub(super) remote_profile: String,
    pub(super) remote_team_alias: String,
    pub(super) remote_team_id: Vec<u8>,
    pub(super) remote_host_id: Vec<u8>,
    pub(super) source_role: StoredTeamRole,
    pub(super) destination_role: StoredTeamRole,
    pub(super) removal_key_commitment: [u8; 32],
    pub(super) actor_uid: Vec<u8>,
    pub(super) actor_device_id: Vec<u8>,
    pub(super) actor_source_role: StoredTeamRole,
    pub(super) actor_generation: u64,
    pub(super) expected_seqno: u64,
    pub(super) operation_id: [u8; 16],
    pub(super) scheduler_job_id: [u8; 16],
    pub(super) rotations: Vec<StoredTeamPtkRotation>,
}

impl Drop for StoredFederatedExpulsion {
    fn drop(&mut self) {
        for rotation in &mut self.rotations {
            rotation.seed.zeroize();
        }
    }
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct StoredTeamRole {
    kind: u8,
    visibility: i16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct StoredTeamDestinationRole {
    kind: u8,
    visibility: i16,
}

impl StoredTeamDestinationRole {
    fn from_role(role: Role) -> Self {
        Self {
            kind: role.protocol_value() as u8,
            visibility: role.visibility().unwrap_or(0),
        }
    }

    fn role(self) -> Result<Role> {
        match (self.kind, self.visibility) {
            (0, 0) => Ok(Role::NONE),
            (1, visibility) => Ok(Role::member(visibility)),
            (2, 0) => Ok(Role::ADMIN),
            (3, 0) => Ok(Role::OWNER),
            _ => Err(Error::InvalidAccount(
                "pending member edit destination role is invalid",
            )),
        }
    }
}

impl StoredTeamRole {
    pub(super) fn from_role(role: Role) -> Self {
        Self {
            kind: role.protocol_value() as u8,
            visibility: role.visibility().unwrap_or(0),
        }
    }

    pub(super) fn role(self) -> Result<Role> {
        match (self.kind, self.visibility) {
            (1, visibility) => Ok(Role::member(visibility)),
            (2, 0) => Ok(Role::ADMIN),
            (3, 0) => Ok(Role::OWNER),
            _ => Err(Error::InvalidAccount("pending team rekey role is invalid")),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredTeamRekeyChange {
    pub(super) party: Vec<u8>,
    #[serde(default)]
    pub(super) host: Option<Vec<u8>>,
    pub(super) source_role: StoredTeamRole,
    pub(super) destination_role: StoredTeamRole,
    pub(super) generation: u64,
    pub(super) verify_key: Vec<u8>,
    pub(super) hepk_fingerprint: [u8; 32],
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredTeamPtkRotation {
    pub(super) role: StoredTeamRole,
    pub(super) seed: [u8; 32],
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredTeamRekey {
    pub(super) version: u32,
    pub(super) team_alias: String,
    pub(super) team_id: Vec<u8>,
    pub(super) transport_uid: Vec<u8>,
    pub(super) actor_uid: Vec<u8>,
    pub(super) actor_source_role: StoredTeamRole,
    pub(super) actor_generation: u64,
    pub(super) actor_device_id: Vec<u8>,
    pub(super) operation_id: [u8; 16],
    pub(super) expected_seqno: u64,
    pub(super) changes: Vec<StoredTeamRekeyChange>,
    pub(super) rotations: Vec<StoredTeamPtkRotation>,
}

impl Drop for StoredTeamRekey {
    fn drop(&mut self) {
        for rotation in &mut self.rotations {
            rotation.seed.zeroize();
        }
    }
}

impl StoredTeam {
    pub(super) fn random_named(alias: &str, account_alias: &str, name: &str) -> Result<Self> {
        if name.trim().is_empty() || name.len() > 256 {
            return Err(Error::InvalidAccount(
                "team name is empty or exceeds 256 bytes",
            ));
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
        if is_background_team_alias(alias) {
            return Err(Error::InvalidAccount(
                "team alias uses the reserved background-sync namespace",
            ));
        }
        Ok(Self {
            version: CREDENTIAL_VERSION,
            alias: alias.to_owned(),
            account_alias: account_alias.to_owned(),
            origin: StoredTeamOrigin::CreatedHere,
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
            invitation_members: Vec::new(),
            local_members: Vec::new(),
        })
    }

    fn discovered(account_alias: &str, authenticated: &AuthenticatedTeamOutcome) -> Result<Self> {
        validate_name(account_alias)?;
        let kind = match authenticated.verified.team().entity_type() {
            foks_proto::ENTITY_NAMED_TEAM => StoredTeamKind::Named,
            foks_proto::ENTITY_AD_HOC_TEAM => StoredTeamKind::AdHoc,
            _ => {
                return Err(foks_client::Error::TeamBinding(
                    "discovery returned an unsupported team identity",
                )
                .into())
            }
        };
        let (alias, name) = match kind {
            StoredTeamKind::Named => {
                let name = String::from_utf8(authenticated.verified.team_name_utf8().to_vec())
                    .map_err(|_| Error::InvalidAccount("authenticated team name is not UTF-8"))?;
                if name.trim().is_empty() || name.len() > 256 {
                    return Err(Error::InvalidAccount(
                        "authenticated team name is missing or excessive",
                    ));
                }
                let mut alias = String::from_utf8(authenticated.verified.team_name().to_vec())
                    .map_err(|_| {
                        Error::InvalidAccount("authenticated normalized team name is not UTF-8")
                    })?;
                if is_background_team_alias(&alias) {
                    alias = format!("group_{alias}");
                }
                validate_name(&alias)?;
                (alias, Some(name))
            }
            StoredTeamKind::AdHoc => {
                let id = hex(authenticated.verified.team().as_bytes());
                (format!("adhoc_{}", &id[..16]), None)
            }
        };
        Ok(Self {
            version: CREDENTIAL_VERSION,
            alias,
            account_alias: account_alias.to_owned(),
            origin: StoredTeamOrigin::Discovered,
            kind,
            name,
            team_id: authenticated.verified.team().as_bytes().to_vec(),
            member_min: [0; 32],
            member: [0; 32],
            admin: [0; 32],
            owner: [0; 32],
            removal_key: None,
            name_commitment: None,
            active: true,
            federated_members: Vec::new(),
            invitation_members: Vec::new(),
            local_members: Vec::new(),
        })
    }

    fn named_secrets(&self) -> Result<NamedTeamSecrets> {
        if self.kind != StoredTeamKind::Named || self.origin != StoredTeamOrigin::CreatedHere {
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
        if self.kind != StoredTeamKind::AdHoc || self.origin != StoredTeamOrigin::CreatedHere {
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
        for member in &mut self.local_members {
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
    pub(super) fn contains_team(&mut self, alias: &str) -> Result<bool> {
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

    pub(super) fn team_rekey(&mut self, alias: &str) -> Result<Option<StoredTeamRekey>> {
        validate_name(alias)?;
        let bytes = match self.store.get(&team_rekey_key(alias)) {
            Ok(bytes) => bytes,
            Err(foks_keystore::Error::Missing) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let pending: StoredTeamRekey = serde_json::from_slice(&bytes)?;
        validate_stored_team_rekey(&pending, alias)?;
        Ok(Some(pending))
    }

    pub(super) fn team_rekey_aliases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .store
            .keys()?
            .into_iter()
            .filter_map(|key| key.strip_prefix("team-rekey.").map(str::to_owned))
            .collect())
    }

    pub(super) fn team_rekey_for_team(
        &mut self,
        team_id: &[u8],
    ) -> Result<Option<(String, StoredTeamRekey)>> {
        let mut found = None;
        for alias in self.team_rekey_aliases()? {
            let Some(pending) = self.team_rekey(&alias)? else {
                continue;
            };
            if pending.team_id != team_id {
                continue;
            }
            if found.is_some() {
                return Err(Error::InvalidAccount(
                    "more than one caller-durable CLKR intent names the same team",
                ));
            }
            found = Some((alias, pending));
        }
        Ok(found)
    }

    pub(super) fn put_team_rekey(&mut self, pending: &StoredTeamRekey) -> Result<()> {
        validate_stored_team_rekey(pending, &pending.team_alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(pending)?);
        self.store
            .put(&team_rekey_key(&pending.team_alias), &encoded)?;
        Ok(())
    }

    pub(super) fn remove_team_rekey(&mut self, alias: &str) -> Result<()> {
        validate_name(alias)?;
        self.store.remove(&team_rekey_key(alias))?;
        Ok(())
    }

    pub(super) fn team_member_edit(&mut self, alias: &str) -> Result<Option<StoredTeamMemberEdit>> {
        validate_name(alias)?;
        let bytes = match self.store.get(&team_member_edit_key(alias)) {
            Ok(bytes) => bytes,
            Err(foks_keystore::Error::Missing) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let edit: StoredTeamMemberEdit = serde_json::from_slice(&bytes)?;
        validate_stored_team_member_edit(&edit, alias)?;
        Ok(Some(edit))
    }

    pub(super) fn put_team_member_edit(&mut self, edit: &StoredTeamMemberEdit) -> Result<()> {
        validate_stored_team_member_edit(edit, &edit.team_alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(edit)?);
        self.store
            .put(&team_member_edit_key(&edit.team_alias), &encoded)?;
        Ok(())
    }

    pub(super) fn remove_team_member_edit(&mut self, alias: &str) -> Result<()> {
        validate_name(alias)?;
        self.store.remove(&team_member_edit_key(alias))?;
        Ok(())
    }

    pub(super) fn federation_expulsion(
        &mut self,
        alias: &str,
    ) -> Result<Option<StoredFederatedExpulsion>> {
        validate_name(alias)?;
        let bytes = match self.store.get(&federation_expulsion_key(alias)) {
            Ok(bytes) => bytes,
            Err(foks_keystore::Error::Missing) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let intent: StoredFederatedExpulsion = serde_json::from_slice(&bytes)?;
        validate_stored_federated_expulsion(&intent, alias)?;
        Ok(Some(intent))
    }

    pub(super) fn put_federation_expulsion(
        &mut self,
        intent: &StoredFederatedExpulsion,
    ) -> Result<()> {
        validate_stored_federated_expulsion(intent, &intent.local_team_alias)?;
        let encoded = Zeroizing::new(serde_json::to_vec(intent)?);
        self.store.put(
            &federation_expulsion_key(&intent.local_team_alias),
            &encoded,
        )?;
        Ok(())
    }

    pub(super) fn remove_federation_expulsion(&mut self, alias: &str) -> Result<()> {
        validate_name(alias)?;
        self.store.remove(&federation_expulsion_key(alias))?;
        Ok(())
    }
}

fn validate_stored_federated_expulsion(
    intent: &StoredFederatedExpulsion,
    expected_alias: &str,
) -> Result<()> {
    if intent.version != CREDENTIAL_VERSION
        || intent.local_team_alias != expected_alias
        || intent.expected_seqno < 2
        || intent.rotations.is_empty()
        || intent.rotations.len() > 64
    {
        return Err(Error::InvalidAccount(
            "pending federation expulsion header is invalid",
        ));
    }
    validate_name(&intent.local_team_alias)?;
    validate_name(&intent.remote_profile)?;
    validate_name(&intent.remote_team_alias)?;
    EntityId::from_bytes(intent.local_team_id.clone())?
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    EntityId::from_bytes(intent.local_host_id.clone())?.require_type(foks_proto::ENTITY_HOST)?;
    let remote = EntityId::from_bytes(intent.remote_team_id.clone())?;
    if !matches!(
        remote.entity_type(),
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) {
        return Err(Error::InvalidAccount(
            "pending federation expulsion target is invalid",
        ));
    }
    EntityId::from_bytes(intent.remote_host_id.clone())?.require_type(foks_proto::ENTITY_HOST)?;
    EntityId::from_bytes(intent.actor_uid.clone())?.require_type(foks_proto::ENTITY_USER)?;
    EntityId::from_bytes(intent.actor_device_id.clone())?
        .require_type(foks_proto::ENTITY_DEVICE)?;
    if intent.actor_source_role.role()? == Role::NONE
        || intent.actor_generation == 0
        || intent.source_role.role()? == Role::NONE
        || !matches!(
            intent.destination_role.role()?.kind(),
            foks_proto::RoleType::Member
        )
    {
        return Err(Error::InvalidAccount(
            "pending federation expulsion role binding is invalid",
        ));
    }
    let mut prior = None;
    let mut seeds = std::collections::BTreeSet::new();
    for rotation in &intent.rotations {
        let role = rotation.role.role()?;
        if prior.is_some_and(|old| old >= role) || !seeds.insert(rotation.seed) {
            return Err(Error::InvalidAccount(
                "pending federation expulsion PTKs are invalid",
            ));
        }
        prior = Some(role);
    }
    Ok(())
}

fn validate_stored_team_member_edit(
    edit: &StoredTeamMemberEdit,
    expected_alias: &str,
) -> Result<()> {
    if edit.version != CREDENTIAL_VERSION
        || edit.team_alias != expected_alias
        || edit.expected_seqno < 2
        || edit.rotations.len() > 64
    {
        return Err(Error::InvalidAccount(
            "pending team member edit header is invalid",
        ));
    }
    validate_name(&edit.team_alias)?;
    if foks_verify::normalize_username(edit.target_username.as_bytes()).as_deref()
        != Some(edit.target_username.as_bytes())
    {
        return Err(Error::InvalidAccount(
            "pending team member edit username is invalid",
        ));
    }
    EntityId::from_bytes(edit.team_id.clone())?.require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    EntityId::from_bytes(edit.actor_uid.clone())?.require_type(foks_proto::ENTITY_USER)?;
    EntityId::from_bytes(edit.actor_device_id.clone())?.require_type(foks_proto::ENTITY_DEVICE)?;
    EntityId::from_bytes(edit.target_id.clone())?.require_type(foks_proto::ENTITY_USER)?;
    let source = edit.source_role.role()?;
    let destination = edit.destination_role.role()?;
    if destination >= source {
        return Err(Error::InvalidAccount(
            "pending team member edit is not a demotion or removal",
        ));
    }
    let mut prior = None;
    let mut seeds = std::collections::BTreeSet::new();
    for rotation in &edit.rotations {
        let role = rotation.role.role()?;
        if prior.is_some_and(|old| old >= role) || !seeds.insert(rotation.seed) {
            return Err(Error::InvalidAccount(
                "pending team member edit PTKs are invalid",
            ));
        }
        prior = Some(role);
    }
    Ok(())
}

fn validate_stored_team_rekey(pending: &StoredTeamRekey, expected_alias: &str) -> Result<()> {
    if pending.version != CREDENTIAL_VERSION
        || pending.team_alias != expected_alias
        || pending.expected_seqno < 2
        || pending.changes.is_empty()
        || pending.changes.len() > 4096
        || pending.rotations.len() > 64
    {
        return Err(Error::InvalidAccount(
            "pending team rekey header is invalid",
        ));
    }
    validate_name(&pending.team_alias)?;
    EntityId::from_bytes(pending.team_id.clone())?.require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    EntityId::from_bytes(pending.transport_uid.clone())?.require_type(foks_proto::ENTITY_USER)?;
    let actor = EntityId::from_bytes(pending.actor_uid.clone())?;
    if !matches!(
        actor.entity_type(),
        foks_proto::ENTITY_USER | foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) || pending.actor_source_role.role()? == Role::NONE
        || pending.actor_generation == 0
    {
        return Err(Error::InvalidAccount(
            "pending team rekey signer binding is invalid",
        ));
    }
    let actor_device = EntityId::from_bytes(pending.actor_device_id.clone())?;
    if !matches!(
        actor_device.entity_type(),
        foks_proto::ENTITY_DEVICE | foks_proto::ENTITY_YUBI
    ) {
        return Err(Error::InvalidAccount("pending team rekey actor is invalid"));
    }
    let mut parties = std::collections::BTreeSet::new();
    for change in &pending.changes {
        let party = EntityId::from_bytes(change.party.clone())?;
        let expected_verify_key_type = match party.entity_type() {
            foks_proto::ENTITY_USER => foks_proto::ENTITY_PUK_VERIFY,
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => {
                foks_proto::ENTITY_PTK_VERIFY
            }
            _ => {
                return Err(Error::InvalidAccount("pending team rekey party is invalid"));
            }
        };
        if let Some(host) = &change.host {
            EntityId::from_bytes(host.clone())?.require_type(foks_proto::ENTITY_HOST)?;
        }
        let source = change.source_role.role()?;
        if change.destination_role.role()? == Role::NONE
            || change.generation < 2
            || EntityId::from_bytes(change.verify_key.clone())?.entity_type()
                != expected_verify_key_type
            || !parties.insert((change.party.clone(), change.host.clone(), source))
        {
            return Err(Error::InvalidAccount(
                "pending team rekey change is invalid",
            ));
        }
    }
    let mut prior = None;
    let mut seeds = std::collections::BTreeSet::new();
    for rotation in &pending.rotations {
        let role = rotation.role.role()?;
        if prior.is_some_and(|old| old >= role) || !seeds.insert(rotation.seed) {
            return Err(Error::InvalidAccount("pending team rekey PTKs are invalid"));
        }
        prior = Some(role);
    }
    Ok(())
}
fn validate_stored_team(team: &StoredTeam, expected_alias: &str) -> Result<()> {
    if team.version != CREDENTIAL_VERSION || team.alias != expected_alias {
        return Err(Error::InvalidAccount(
            "team version or alias binding changed",
        ));
    }
    validate_name(&team.alias)?;
    validate_name(&team.account_alias)?;
    if is_background_team_alias(&team.alias) {
        return Err(Error::InvalidAccount(
            "team alias uses the reserved background-sync namespace",
        ));
    }
    let id = EntityId::from_bytes(team.team_id.clone())?;
    if team.origin == StoredTeamOrigin::Discovered {
        let identity_is_valid = match team.kind {
            StoredTeamKind::Named => {
                id.entity_type() == foks_proto::ENTITY_NAMED_TEAM
                    && team
                        .name
                        .as_deref()
                        .is_some_and(|name| !name.trim().is_empty() && name.len() <= 256)
            }
            StoredTeamKind::AdHoc => {
                id.entity_type() == foks_proto::ENTITY_AD_HOC_TEAM && team.name.is_none()
            }
        };
        if !identity_is_valid
            || !team.active
            || team.member_min != [0; 32]
            || team.member != [0; 32]
            || team.admin != [0; 32]
            || team.owner != [0; 32]
            || team.removal_key.is_some()
            || team.name_commitment.is_some()
        {
            return Err(Error::InvalidAccount(
                "discovered team identity or creator material is invalid",
            ));
        }
    } else {
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
                if team.name.is_some()
                    || team.removal_key.is_some()
                    || team.name_commitment.is_some()
                {
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
    let mut local_users = std::collections::BTreeSet::new();
    for member in &team.local_members {
        if foks_verify::normalize_username(member.username.as_bytes()).as_deref()
            != Some(member.username.as_bytes())
            || !local_users.insert(member.target_id.clone())
            || member.target_generation == 0
            || member.expected_seqno < 2
            || member.target_source_role.role()? != Role::OWNER
            || member.destination_role.role()? == Role::NONE
            || foks_crypto::team_removal_key_commitment(&SecretSeed::new(member.removal_key))?
                != member.removal_key_commitment
        {
            return Err(Error::InvalidAccount(
                "local team membership binding is invalid",
            ));
        }
        EntityId::from_bytes(member.target_id.clone())?.require_type(foks_proto::ENTITY_USER)?;
        EntityId::from_bytes(member.target_verify_key.clone())?
            .require_type(foks_proto::ENTITY_PUK_VERIFY)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_keystore::EncryptedFileSecretStore;
    use foks_server_testkit::TestEnvironment;

    #[test]
    fn background_team_alias_namespace_cannot_be_user_owned() {
        let alias = format!("{BACKGROUND_TEAM_ALIAS_PREFIX}{}", "a".repeat(60));
        assert!(StoredTeam::random_named(&alias, "account", "reserved").is_err());
        assert!(StoredTeam::random_adhoc(&alias, "account").is_err());
    }

    #[test]
    fn caller_durable_team_rekey_accepts_only_software_or_yubi_actors() {
        let entity = |kind, size: usize, fill| {
            let mut bytes = vec![fill; size];
            bytes[0] = kind;
            bytes
        };
        let mut pending = StoredTeamRekey {
            version: CREDENTIAL_VERSION,
            team_alias: "hardware-rekey".to_owned(),
            team_id: entity(foks_proto::ENTITY_NAMED_TEAM, 33, 0x21),
            transport_uid: entity(foks_proto::ENTITY_USER, 33, 0x22),
            actor_uid: entity(foks_proto::ENTITY_USER, 33, 0x22),
            actor_source_role: StoredTeamRole::from_role(Role::OWNER),
            actor_generation: 1,
            actor_device_id: entity(foks_proto::ENTITY_YUBI, 34, 0x02),
            operation_id: [0x23; 16],
            expected_seqno: 2,
            changes: vec![StoredTeamRekeyChange {
                party: entity(foks_proto::ENTITY_USER, 33, 0x24),
                host: None,
                source_role: StoredTeamRole::from_role(Role::OWNER),
                destination_role: StoredTeamRole::from_role(Role::OWNER),
                generation: 2,
                verify_key: entity(foks_proto::ENTITY_PUK_VERIFY, 33, 0x25),
                hepk_fingerprint: [0x26; 32],
            }],
            rotations: vec![StoredTeamPtkRotation {
                role: StoredTeamRole::from_role(Role::OWNER),
                seed: [0x27; 32],
            }],
        };
        let encoded = serde_json::to_vec(&pending).unwrap();
        let decoded: StoredTeamRekey = serde_json::from_slice(&encoded).unwrap();
        validate_stored_team_rekey(&decoded, "hardware-rekey").unwrap();

        pending.actor_uid = entity(foks_proto::ENTITY_NAMED_TEAM, 33, 0x29);
        pending.actor_source_role = StoredTeamRole::from_role(Role::ADMIN);
        pending.actor_generation = 3;
        validate_stored_team_rekey(&pending, "hardware-rekey").unwrap();

        pending.transport_uid = entity(foks_proto::ENTITY_NAMED_TEAM, 33, 0x2a);
        assert!(validate_stored_team_rekey(&pending, "hardware-rekey").is_err());
        pending.transport_uid = entity(foks_proto::ENTITY_USER, 33, 0x22);

        pending.actor_device_id = entity(foks_proto::ENTITY_SUBKEY, 33, 0x28);
        assert!(validate_stored_team_rekey(&pending, "hardware-rekey").is_err());
    }

    #[test]
    fn durable_team_rekey_binds_remote_team_recipients_to_their_host() {
        let entity = |kind, size: usize, fill| {
            let mut bytes = vec![fill; size];
            bytes[0] = kind;
            bytes
        };
        let team = entity(foks_proto::ENTITY_NAMED_TEAM, 33, 0x31);
        let host_a = entity(foks_proto::ENTITY_HOST, 33, 0x32);
        let host_b = entity(foks_proto::ENTITY_HOST, 33, 0x33);
        let change = |host| StoredTeamRekeyChange {
            party: team.clone(),
            host: Some(host),
            source_role: StoredTeamRole::from_role(Role::ADMIN),
            destination_role: StoredTeamRole::from_role(Role::ADMIN),
            generation: 2,
            verify_key: entity(foks_proto::ENTITY_PTK_VERIFY, 33, 0x34),
            hepk_fingerprint: [0x35; 32],
        };
        let mut pending = StoredTeamRekey {
            version: CREDENTIAL_VERSION,
            team_alias: "remote-team-rekey".to_owned(),
            team_id: entity(foks_proto::ENTITY_NAMED_TEAM, 33, 0x36),
            transport_uid: entity(foks_proto::ENTITY_USER, 33, 0x37),
            actor_uid: entity(foks_proto::ENTITY_USER, 33, 0x37),
            actor_source_role: StoredTeamRole::from_role(Role::OWNER),
            actor_generation: 1,
            actor_device_id: entity(foks_proto::ENTITY_DEVICE, 33, 0x38),
            operation_id: [0x39; 16],
            expected_seqno: 2,
            changes: vec![change(host_a.clone()), change(host_b)],
            rotations: vec![StoredTeamPtkRotation {
                role: StoredTeamRole::from_role(Role::ADMIN),
                seed: [0x3a; 32],
            }],
        };
        validate_stored_team_rekey(&pending, "remote-team-rekey").unwrap();

        pending.changes[1].host = Some(host_a);
        assert!(validate_stored_team_rekey(&pending, "remote-team-rekey").is_err());
        pending.changes[1].verify_key = entity(foks_proto::ENTITY_PUK_VERIFY, 33, 0x3b);
        assert!(validate_stored_team_rekey(&pending, "remote-team-rekey").is_err());
    }

    #[test]
    fn new_team_roots_share_discovery_without_sharing_root_writes() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment
            .client_path("shared-root-policy", "state")
            .unwrap();
        let root = environment
            .client_path("shared-root-policy", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        crate::ClientCredentials::initialize(&state, crate::CredentialBackend::PrivateFile)
            .unwrap();
        let mut registry = crate::ProfileRegistry::open(&state).unwrap();
        registry
            .add(crate::Profile {
                name: "local".to_owned(),
                label: None,
                probe: format!("localhost:{}", addresses.probe.port()),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let profile = crate::ProfileSession::open(&registry, "local").unwrap();
        let credentials = crate::ClientCredentials::open(&state).unwrap();
        let master = credentials.master_key().unwrap();
        credentials
            .with_checked_session(&profile, |session| {
                session.probe_and_pin()?;
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.create_account(
                    "owner",
                    "rootpolicyowner",
                    "laptop",
                    "owner@example.test",
                    "",
                    None,
                    &mut vault,
                    &master,
                )?;
                session.create_named_team(
                    "owner",
                    "named",
                    "shared-root-policy",
                    &mut vault,
                    &master,
                )?;
                session.create_adhoc_team("owner", "adhoc", &mut vault, &master)?;
                let host = session.pinned_host()?;
                let cache = foks_client_db::SoftStateStore::open(&session.paths().soft_database)?;
                for alias in ["named", "adhoc"] {
                    let team = vault.team(alias)?;
                    let tree = cache.tree(host.host_id().as_bytes(), &team.team_id)?;
                    assert_eq!(tree.len(), 1);
                    let root = foks_proto::KvRoot::decode(&tree[0].root_bytes)?;
                    let directory = foks_proto::KvDirectoryPair::decode(&tree[0].directory_bytes)?;
                    assert_eq!(root.key.role, Role::member(-0x4000));
                    assert_eq!(directory.active.key.role, Role::member(-0x4000));
                    assert_eq!(directory.active.write_role, Role::OWNER);
                }
                Ok::<_, crate::Error>(())
            })
            .unwrap();
    }

    #[test]
    fn discovery_persists_and_supports_invited_admin_mutations_after_interruption() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();

        let initialize = |label: &str| {
            let state = environment.client_path(label, "state").unwrap();
            let root = environment.client_path(label, "probe-root.der").unwrap();
            environment.write_probe_root(&root).unwrap();
            crate::ClientCredentials::initialize(&state, crate::CredentialBackend::PrivateFile)
                .unwrap();
            let mut registry = crate::ProfileRegistry::open(&state).unwrap();
            registry
                .add(crate::Profile {
                    name: "local".to_owned(),
                    label: None,
                    probe: format!("localhost:{}", addresses.probe.port()),
                    protocol: crate::ProtocolPolicy::V019,
                    trust: crate::TrustRoot::CertificateDer { path: root },
                })
                .unwrap();
            let session = crate::ProfileSession::open(&registry, "local").unwrap();
            let credentials = crate::ClientCredentials::open(&state).unwrap();
            credentials
                .with_checked_session(&session, |session| {
                    session.probe_and_pin()?;
                    Ok::<_, crate::Error>(())
                })
                .unwrap();
            state
        };

        let member_state = initialize("team-discovery-member");
        let member_registry = crate::ProfileRegistry::open(&member_state).unwrap();
        let member_session = crate::ProfileSession::open(&member_registry, "local").unwrap();
        let member_credentials = crate::ClientCredentials::open(&member_state).unwrap();
        let member_master = member_credentials.master_key().unwrap();
        member_credentials
            .with_checked_session(&member_session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&member_master),
                )?;
                session.create_account(
                    "personal",
                    "invitedmember",
                    "member laptop",
                    "member@example.test",
                    "",
                    None,
                    &mut AccountVault::new(&mut store),
                    &member_master,
                )?;
                Ok::<_, crate::Error>(())
            })
            .unwrap();

        let owner_state = initialize("team-discovery-owner");
        let owner_registry = crate::ProfileRegistry::open(&owner_state).unwrap();
        let owner_session = crate::ProfileSession::open(&owner_registry, "local").unwrap();
        let owner_credentials = crate::ClientCredentials::open(&owner_state).unwrap();
        let owner_master = owner_credentials.master_key().unwrap();
        owner_credentials
            .with_checked_session(&owner_session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&owner_master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.create_account(
                    "owner",
                    "discoveryowner",
                    "owner laptop",
                    "owner@example.test",
                    "",
                    None,
                    &mut vault,
                    &owner_master,
                )?;
                session.create_account(
                    "target",
                    "discoverytarget",
                    "target laptop",
                    "target@example.test",
                    "",
                    None,
                    &mut vault,
                    &owner_master,
                )?;
                session.create_named_team(
                    "owner",
                    "alpha-local",
                    "invited-alpha",
                    &mut vault,
                    &owner_master,
                )?;
                session.add_local_team_member(
                    "alpha-local",
                    "invitedmember",
                    TeamMemberRole::Admin,
                    &mut vault,
                    &owner_master,
                )?;
                Ok::<_, crate::Error>(())
            })
            .unwrap();

        member_credentials
            .with_checked_session(&member_session, |session| {
                let collision_id = {
                    let mut store = EncryptedFileSecretStore::open(
                        &session.paths().credential_store,
                        crate::derive_vault_key(&member_master),
                    )?;
                    let mut vault = AccountVault::new(&mut store);
                    let collision = StoredTeam::random_adhoc("invited_alpha", "personal")?;
                    let collision_id = collision.team_id.clone();
                    vault.put_team(&collision)?;
                    TEST_FAIL_AFTER_DISCOVERY_PERSIST
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    let interrupted = session
                        .discover_teams("personal", &mut vault)
                        .expect_err("the discovery failpoint interrupts after one durable record");
                    assert_eq!(
                        session.list_teams(&mut vault)?.len(),
                        2,
                        "unexpected discovery error: {interrupted:?}"
                    );
                    collision_id
                };

                // Reopen the protected store to model process interruption,
                // not merely a retry through the same in-memory vault value.
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&member_master),
                )?;
                let mut vault = AccountVault::new(&mut store);

                let report = session.discover_teams("personal", &mut vault)?;
                assert_eq!(report.account_alias, "personal");
                assert_eq!(report.teams.len(), 1);
                assert_eq!(
                    report
                        .teams
                        .iter()
                        .filter_map(|team| team.name.as_deref())
                        .collect::<std::collections::BTreeSet<_>>(),
                    std::collections::BTreeSet::from(["invited-alpha"])
                );
                assert!(report.teams.iter().all(|team| {
                    team.account_alias == "personal" && team.kind == "named" && team.active
                }));
                assert!(report.teams.iter().any(|team| {
                    team.name.as_deref() == Some("invited-alpha")
                        && team.alias.starts_with("invited_alpha_")
                }));
                let preserved_collision = vault.team("invited_alpha")?;
                assert_eq!(preserved_collision.team_id, collision_id);
                assert_eq!(preserved_collision.origin, StoredTeamOrigin::CreatedHere);
                assert!(!preserved_collision.active);

                let repeated = session.discover_teams("personal", &mut vault)?;
                assert_eq!(repeated, report);
                assert_eq!(session.list_teams(&mut vault)?.len(), 2);

                // One team ID has one account-bound local store. A second
                // account must not silently reuse or replace that binding;
                // an explicit rebind policy would be needed to change it.
                let bound = vault.team(&report.teams[0].alias)?;
                let mut other_account = vault.team(&report.teams[0].alias)?;
                other_account.account_alias = "other-account".to_owned();
                assert!(matches!(
                    bind_existing_discovery(&bound, &other_account),
                    Err(Error::Client(foks_client::Error::TeamBinding(
                        "stored team account or identity differs from authenticated discovery"
                    )))
                ));

                let admin_alias = report
                    .teams
                    .iter()
                    .find(|team| team.name.as_deref() == Some("invited-alpha"))
                    .map(|team| team.alias.as_str())
                    .expect("the invited admin team was discovered");
                session.add_local_team_member(
                    admin_alias,
                    "discoverytarget",
                    TeamMemberRole::Member { visibility: 0 },
                    &mut vault,
                    &member_master,
                )?;
                let discovered_target = session
                    .list_team_members(admin_alias, &mut vault)?
                    .into_iter()
                    .find(|member| member.username.as_deref() == Some("discoverytarget"))
                    .expect("the added discovery target is in the authenticated roster");
                let mut malformed_journal = vault.team(admin_alias)?;
                malformed_journal.local_members[0].expected_seqno = 0;
                assert!(vault.put_team(&malformed_journal).is_err());
                session.remove_local_team_member(
                    admin_alias,
                    &discovered_target.party_id_hex,
                    &mut vault,
                    &member_master,
                )?;
                assert!(session
                    .list_team_members(admin_alias, &mut vault)?
                    .iter()
                    .all(|member| member.username.as_deref() != Some("discoverytarget")));
                for team in &report.teams {
                    assert!(session
                        .list_team_members(&team.alias, &mut vault)?
                        .iter()
                        .any(|member| member.username.as_deref() == Some("invitedmember")));
                    let stored = vault.team(&team.alias)?;
                    assert_eq!(stored.origin, StoredTeamOrigin::Discovered);
                    assert_eq!(stored.member, [0; 32]);
                }
                Ok::<_, crate::Error>(())
            })
            .unwrap();
    }

    #[test]
    fn local_member_surface_adds_demotes_lists_and_removes() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let state = environment.client_path("team-members", "state").unwrap();
        let root = environment
            .client_path("team-members", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        crate::ClientCredentials::initialize(&state, crate::CredentialBackend::PrivateFile)
            .unwrap();
        let addresses = environment.addresses().unwrap();
        let mut registry = crate::ProfileRegistry::open(&state).unwrap();
        registry
            .add(crate::Profile {
                name: "local".to_owned(),
                label: None,
                probe: format!("localhost:{}", addresses.probe.port()),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        let credentials = crate::ClientCredentials::open(&state).unwrap();
        credentials
            .with_checked_session(&session, |session| {
                session.probe_and_pin()?;
                Ok::<_, crate::Error>(())
            })
            .unwrap();
        let master = credentials.master_key().unwrap();
        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.create_account(
                    "owner",
                    "managedowner",
                    "team owner",
                    "owner@example.test",
                    "",
                    None,
                    &mut vault,
                    &master,
                )?;
                session.create_account(
                    "member",
                    "managedmember",
                    "team member",
                    "member@example.test",
                    "",
                    None,
                    &mut vault,
                    &master,
                )?;
                session.create_named_team(
                    "owner",
                    "managed-team",
                    "managed-team-name",
                    &mut vault,
                    &master,
                )?;

                let added = session
                    .add_local_team_member(
                        "managed-team",
                        "managedmember",
                        TeamMemberRole::Admin,
                        &mut vault,
                        &master,
                    )
                    .expect("application local-member addition succeeds");
                assert_eq!(added.team_chain_sequence, 2);
                assert_eq!(added.destination_role, Some(TeamMemberRole::Admin));
                let members = session
                    .list_team_members("managed-team", &mut vault)
                    .expect("application roster listing succeeds after addition");
                assert_eq!(members.len(), 2);
                assert!(members.iter().any(|member| {
                    member.username.as_deref() == Some("managedmember")
                        && member.destination_role == TeamMemberRole::Admin
                        && member.locally_manageable
                }));
                let managed_party_id = members
                    .iter()
                    .find(|member| member.username.as_deref() == Some("managedmember"))
                    .map(|member| member.party_id_hex.clone())
                    .expect("the managed member has an authenticated party ID");
                let member = vault.account("member")?;
                let host = session.pinned_host()?;
                assert_eq!(
                    session
                        .client
                        .local_team_list(&host, &member.credential)
                        .expect("added member can query its server-trust team list")
                        .len(),
                    1
                );

                TEST_FAIL_AFTER_MEMBER_EDIT_COMMIT.store(true, std::sync::atomic::Ordering::SeqCst);
                session
                    .demote_local_team_member(
                        "managed-team",
                        &managed_party_id,
                        TeamMemberRole::Member { visibility: 0 },
                        &mut vault,
                        &master,
                    )
                    .expect_err("test failpoint retains the app-level edit after commit");
                let demoted = session
                    .resume_local_team_member_edit("managed-team", &mut vault, &master)
                    .expect("committed demotion resumes after the exact frame was removed");
                assert_eq!(demoted.team_chain_sequence, 3);
                assert_eq!(
                    demoted.destination_role,
                    Some(TeamMemberRole::Member { visibility: 0 })
                );
                assert!(session
                    .list_team_members("managed-team", &mut vault)
                    .expect("application roster listing succeeds after demotion")
                    .iter()
                    .any(|member| {
                        member.username.as_deref() == Some("managedmember")
                            && member.destination_role == TeamMemberRole::Member { visibility: 0 }
                    }));

                let removed = session
                    .remove_local_team_member(
                        "managed-team",
                        &managed_party_id,
                        &mut vault,
                        &master,
                    )
                    .expect("application local-member removal succeeds");
                assert_eq!(removed.team_chain_sequence, 4);
                assert_eq!(removed.destination_role, None);
                let members = session
                    .list_team_members("managed-team", &mut vault)
                    .expect("application roster listing succeeds after removal");
                assert_eq!(members.len(), 1);
                assert!(members
                    .iter()
                    .all(|member| member.username.as_deref() != Some("managedmember")));
                assert!(session
                    .client
                    .local_team_list(&host, &member.credential)
                    .expect("removed member can query its server-trust team list")
                    .is_empty());
                Ok::<_, crate::Error>(())
            })
            .unwrap();
    }
}
