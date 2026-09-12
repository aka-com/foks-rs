//! Scoped adapter reads over the existing verified user, team and KV workflows.
use super::*;

impl CheckedProfileSession<'_> {
    pub fn data_members(
        &self,
        alias: &str,
        team_id: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<DataMemberReport>> {
        self.profile.require(Capability::Teams)?;
        let (account, _, team) = self.data_context(alias, Some(team_id), vault)?;
        let team = team.ok_or(Error::InvalidAccount("team scope required"))?;
        let host = self.pinned_host()?;
        let mut admissions = BTreeMap::new();
        for sequence in 1..=team.verified.chain_seqno() {
            let change = team.verified.group_change_at(sequence)?;
            for member in change.changes {
                let key = (
                    member.party.as_bytes().to_vec(),
                    member.scoped_host.as_ref().map(|h| h.as_bytes().to_vec()),
                    member.source_role,
                );
                if member.role == Role::NONE {
                    admissions.remove(&key);
                } else {
                    admissions.entry(key).or_insert(change.time);
                }
            }
        }
        if team.verified.members().len() > 1000 {
            return Err(Error::InvalidAccount("roster exceeds adapter limit"));
        }
        team.verified
            .members()
            .iter()
            .map(|member| {
                let local = member
                    .scoped_host
                    .as_ref()
                    .is_none_or(|id| id == host.host_id());
                let name = if local && member.party.entity_type() == ENTITY_USER {
                    let user = self.client.load_and_pin_user_as_local_team(
                        &host,
                        &account.credential,
                        &member.party,
                        &team.view_token,
                    )?;
                    String::from_utf8(user.username_utf8().to_vec())
                        .map_err(|_| Error::InvalidAccount("username is not UTF-8"))?
                } else {
                    // The verified immutable party ID remains usable when a remote
                    // profile is unavailable. Do not substitute an unverified name.
                    let id = hex(member.party.as_bytes());
                    if member.party.entity_type() == ENTITY_USER {
                        id
                    } else {
                        format!("{id} (team)")
                    }
                };
                let key = (
                    member.party.as_bytes().to_vec(),
                    member.scoped_host.as_ref().map(|h| h.as_bytes().to_vec()),
                    member.source_role,
                );
                Ok(DataMemberReport {
                    name,
                    host: if local {
                        "-".into()
                    } else {
                        hex(member
                            .scoped_host
                            .as_ref()
                            .ok_or(Error::InvalidAccount("missing remote host"))?
                            .as_bytes())
                    },
                    source_role: data_role(member.source_role)?,
                    destination_role: data_role(member.role)?,
                    added_millis: *admissions.get(&key).ok_or(Error::InvalidAccount(
                        "roster admission evidence is missing",
                    ))?,
                })
            })
            .collect()
    }

    pub fn data_memberships(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<DataMembershipsReport> {
        self.profile.require(Capability::Teams)?;
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let user = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        let memberships = self.client.authenticated_user_team_memberships(
            &host,
            &account.credential,
            &user.verified,
        )?;
        let mut complete = !memberships
            .active()
            .any(|event| event.membership.team_host != *host.host_id());
        let graph = self.client.discover_local_team_graph(
            &host,
            &account.credential,
            &user.verified,
            &user.puks,
        )?;
        let mut rows = Vec::new();
        for team in &graph.teams {
            let chain =
                self.client
                    .authenticated_team_memberships(&host, &account.credential, team)?;
            complete &= !chain
                .active()
                .any(|event| event.membership.team_host != *host.host_id());
            let team_name = data_team_name(&team.verified)?;
            for member in team.verified.members() {
                if member
                    .scoped_host
                    .as_ref()
                    .is_some_and(|id| id != host.host_id())
                {
                    continue;
                }
                let via =
                    if member.party == *user.verified.uid() {
                        None
                    } else if graph.edges.iter().any(|(child, parent)| {
                        child == &member.party && parent == team.verified.team()
                    }) {
                        Some(data_team_name(
                            &graph
                                .team(&member.party)
                                .ok_or(Error::InvalidAccount("membership graph lost an actor"))?
                                .verified,
                        )?)
                    } else {
                        continue;
                    };
                let range = member
                    .index_range
                    .as_ref()
                    .unwrap_or(team.verified.index_range());
                rows.push(DataMembershipReport {
                    team: team_name.clone(),
                    team_id: hex(team.verified.team().as_bytes()),
                    source_role: data_role(member.source_role)?,
                    destination_role: data_role(member.role)?,
                    via,
                    index_range: format!(
                        "{}-{}",
                        data_rational(&range.low)?,
                        data_rational(&range.high)?
                    ),
                });
                if rows.len() > 1000 {
                    return Err(Error::InvalidAccount(
                        "membership result exceeds adapter limit",
                    ));
                }
            }
        }
        Ok(DataMembershipsReport {
            complete,
            teams: rows,
        })
    }

    /// Authenticate the selected account before exporting a binding to local adapters.
    pub fn data_identity(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(String, String)> {
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let user = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        Ok((
            hex(host.host_id().as_bytes()),
            hex(user.verified.uid().as_bytes()),
        ))
    }

    pub fn check_data_identity(
        &self,
        alias: &str,
        host_id: &str,
        user_id: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        if hex(host.host_id().as_bytes()) != host_id
            || hex(account.credential.uid.as_bytes()) != user_id
        {
            return Err(Error::InvalidAccount("selected account identity changed"));
        }
        Ok(())
    }

    /// Resolve a local name, persisted alias or immutable ID through the verified graph.
    pub fn resolve_data_team(
        &self,
        alias: &str,
        selector: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<String> {
        self.profile.require(Capability::Teams)?;
        if selector.is_empty() || selector.len() > 1024 {
            return Err(Error::InvalidAccount("invalid team selector"));
        }
        let account = vault.account(alias)?;
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
        let stored_id = match vault.team(selector) {
            Ok(team) if team.active && team.account_alias == alias => Some(hex(&team.team_id)),
            Ok(_) | Err(Error::AccountMissing) => None,
            Err(error) => return Err(error),
        };
        let mut matches = graph.teams.iter().filter(|team| {
            let id = hex(team.verified.team().as_bytes());
            id == selector
                || stored_id.as_ref() == Some(&id)
                || team.verified.team_name_utf8() == selector.as_bytes()
                || team.verified.team_name() == selector.as_bytes()
        });
        let team = matches.next().ok_or(Error::InvalidAccount(
            "team was not found in the verified local membership graph",
        ))?;
        if matches.next().is_some() {
            return Err(Error::InvalidAccount(
                "ambiguous team selector; use its immutable ID",
            ));
        }
        Ok(hex(team.verified.team().as_bytes()))
    }

    fn data_context(
        &self,
        alias: &str,
        team_id: Option<&str>,
        vault: &mut AccountVault<'_>,
    ) -> Result<(
        LoadedAccount,
        AuthenticatedUserOutcome,
        Option<AuthenticatedTeamOutcome>,
    )> {
        let account = vault.account(alias)?;
        let host = self.pinned_host()?;
        let user = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        let team = if let Some(id) = team_id {
            self.profile.require(Capability::Teams)?;
            let graph = self.client.discover_local_team_graph(
                &host,
                &account.credential,
                &user.verified,
                &user.puks,
            )?;
            Some(
                graph
                    .teams
                    .into_iter()
                    .find(|team| hex(team.verified.team().as_bytes()) == id)
                    .ok_or(Error::InvalidAccount(
                        "team is no longer accessible to the selected account",
                    ))?,
            )
        } else {
            None
        };
        Ok((account, user, team))
    }

    fn data_tree(
        &self,
        account: &LoadedAccount,
        user: &AuthenticatedUserOutcome,
        team: Option<&AuthenticatedTeamOutcome>,
    ) -> Result<Vec<KvDirectoryProjection>> {
        self.profile.require(Capability::Kv)?;
        let host = self.pinned_host()?;
        match team {
            Some(team) => Ok(self.client.list_team_kv_metadata(
                &host,
                &account.credential,
                team,
                &self.paths.soft_database,
            )?),
            None => Ok(self.client.list_user_kv_metadata(
                &host,
                &account.credential,
                &user.verified,
                &user.puks,
                &self.paths.soft_database,
            )?),
        }
    }

    pub fn data_catalog(
        &self,
        alias: &str,
        team_id: Option<&str>,
        vault: &mut AccountVault<'_>,
    ) -> Result<DataCatalogReport> {
        let (account, user, team) = self.data_context(alias, team_id, vault)?;
        let tree = self.data_tree(&account, &user, team.as_ref())?;
        let catalog = KvCatalogReport::from_tree(&tree)?;
        if catalog.entries.len() > 1_000 {
            return Err(Error::InvalidKvPath(
                "catalog exceeds bounded adapter limit; use the file client",
            ));
        }
        let entries = catalog
            .entries
            .into_iter()
            .map(|entry| {
                let source = checked_entry(&tree, &entry.path, entry.version)?;
                Ok(DataCatalogEntryReport {
                    node_id: hex(&source.node_id),
                    modified_microseconds: source.creation_time,
                    metadata: entry,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(DataCatalogReport {
            user_chain_sequence: user.verified.chain_seqno(),
            team_chain_sequence: team.as_ref().map(|t| t.verified.chain_seqno()),
            root_id: tree.first().map(|r| hex(&r.root_directory_id)),
            snapshot_version: catalog.snapshot_version,
            entries,
        })
    }

    pub fn data_entry(
        &self,
        alias: &str,
        team_id: Option<&str>,
        path: &str,
        version: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvReadReport> {
        let (account, user, team) = self.data_context(alias, team_id, vault)?;
        let tree = self.data_tree(&account, &user, team.as_ref())?;
        let entry = checked_entry(&tree, path, version)?;
        let host = self.pinned_host()?;
        let node = match team.as_ref() {
            Some(team) => self.client.read_team_kv_node(
                &host,
                &account.credential,
                team,
                KvNodeId(entry.node_id),
            )?,
            None => self.client.read_user_kv_node(
                &host,
                &account.credential,
                &user.verified,
                &user.puks,
                KvNodeId(entry.node_id),
            )?,
        };
        read_report_from_fetched(&tree, path, version, node)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn data_chunk(
        &self,
        alias: &str,
        team_id: Option<&str>,
        path: &str,
        version: u64,
        offset: u64,
        length: usize,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvChunkReport> {
        if length == 0 || length > 128 * 1024 {
            return Err(Error::InvalidKvPath("chunk length exceeds adapter limit"));
        }
        let (account, user, team) = self.data_context(alias, team_id, vault)?;
        let tree = self.data_tree(&account, &user, team.as_ref())?;
        let entry = checked_entry(&tree, path, version)?;
        let host = self.pinned_host()?;
        let chunk = match team.as_ref() {
            Some(team) => self.client.read_team_kv_chunk(
                &host,
                &account.credential,
                team,
                KvNodeId(entry.node_id),
                offset,
                length,
            )?,
            None => self.client.read_user_kv_chunk(
                &host,
                &account.credential,
                &user.verified,
                &user.puks,
                KvNodeId(entry.node_id),
                offset,
                length,
            )?,
        };
        Ok(KvChunkReport {
            path: path.to_owned(),
            version,
            offset,
            content: chunk.content,
            eof: chunk.eof,
        })
    }

    pub fn data_usage(
        &self,
        alias: &str,
        team_id: Option<&str>,
        vault: &mut AccountVault<'_>,
    ) -> Result<(u64, u64)> {
        self.profile.require(Capability::Kv)?;
        let (account, _, team) = self.data_context(alias, team_id, vault)?;
        let usage =
            self.client
                .kv_usage(&self.pinned_host()?, &account.credential, team.as_ref())?;
        let files = usage
            .small
            .number
            .checked_add(usage.large.base.number)
            .ok_or(Error::InvalidAccount("usage count overflow"))?;
        let bytes = usage
            .small
            .bytes
            .checked_add(usage.large.base.bytes)
            .ok_or(Error::InvalidAccount("usage bytes overflow"))?;
        Ok((files, bytes))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DataCatalogEntryReport {
    pub metadata: KvCatalogEntry,
    pub node_id: String,
    pub modified_microseconds: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DataCatalogReport {
    pub user_chain_sequence: u64,
    pub team_chain_sequence: Option<u64>,
    pub root_id: Option<String>,
    pub snapshot_version: u64,
    pub entries: Vec<DataCatalogEntryReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DataMemberReport {
    pub name: String,
    pub host: String,
    pub source_role: String,
    pub destination_role: String,
    pub added_millis: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DataMembershipReport {
    pub team: String,
    pub team_id: String,
    pub source_role: String,
    pub destination_role: String,
    pub via: Option<String>,
    pub index_range: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DataMembershipsReport {
    pub complete: bool,
    pub teams: Vec<DataMembershipReport>,
}

fn data_role(role: Role) -> Result<String> {
    match role.kind() {
        foks_proto::RoleType::Owner => Ok("o".into()),
        foks_proto::RoleType::Admin => Ok("a".into()),
        foks_proto::RoleType::Member => Ok(format!("m/{}", role.visibility().unwrap_or(0))),
        _ => Err(Error::InvalidAccount("unsupported membership role")),
    }
}
fn data_team_name(team: &foks_verify::VerifiedTeamState) -> Result<String> {
    if team.team_name_utf8().is_empty() {
        return Ok(hex(team.team().as_bytes()));
    }
    String::from_utf8(team.team_name_utf8().to_vec())
        .map_err(|_| Error::InvalidAccount("team name is not UTF-8"))
}
fn data_rational(value: &foks_proto::Rational) -> Result<String> {
    if value.infinity {
        return Ok("∞".into());
    }
    if value.exponent.unsigned_abs() > 4096 || value.base.len() > 32 {
        return Err(Error::InvalidAccount("invalid membership index range"));
    }
    let mut bytes = value.base.clone();
    if value.exponent < 0 && value.exponent.unsigned_abs() as usize > bytes.len() {
        let mut prefix = vec![0; value.exponent.unsigned_abs() as usize - bytes.len()];
        prefix.extend(bytes);
        bytes = prefix;
    } else if value.exponent > 0 {
        bytes.resize(bytes.len() + value.exponent as usize, 0);
    }
    let point = (bytes.len() as i64 + value.exponent).max(0) as usize;
    let (mut integer, mut fraction) = bytes.split_at(point.min(bytes.len()));
    while integer.first() == Some(&0) {
        integer = &integer[1..];
    }
    while fraction.last() == Some(&0) {
        fraction = &fraction[..fraction.len() - 1];
    }
    if integer.is_empty() && fraction.is_empty() {
        return Ok("00".into());
    }
    Ok(if fraction.is_empty() {
        hex(integer)
    } else {
        format!("{}.{}", hex(integer), hex(fraction))
    })
}
