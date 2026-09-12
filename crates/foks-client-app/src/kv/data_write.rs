//! Durable adapter intents. Public journals contain fingerprints and immutable IDs;
//! paths and arguments live only in the existing protected mutation store.
use super::*;
use foks_client::MutationDraft;
use foks_client_db::MutationOperation;

mod apply;
use apply::apply_write;

const INTENT_HASH: u64 = 0x3c93_0d54_c728_9a30;
const MAX_BODY: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataWriteKind {
    Put,
    Mkdir,
    Remove,
    Move,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataWriteSpec {
    pub kind: DataWriteKind,
    pub path: String,
    pub destination: Option<String>,
    pub team_selector: Option<String>,
    pub overwrite: bool,
    pub mkdir_p: bool,
    pub recursive: bool,
    pub body_length: u64,
    pub body_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataWriteStatus {
    Prepared,
    Committed,
    Rejected,
    SubmissionUnknown,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DataWriteOutcome {
    pub submission_id: String,
    pub status: DataWriteStatus,
    /// True when ancillary namespace steps committed but completion is unproven.
    pub partial: bool,
    pub node_id: Option<String>,
}

impl CheckedProfileSession<'_> {
    /// Idempotent preparation. Resolve names only for an unseen submission ID.
    pub fn prepare_data_write(
        &self,
        alias: &str,
        id: [u8; 16],
        spec: DataWriteSpec,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<DataWriteOutcome> {
        self.profile.require(Capability::Kv)?;
        validate_spec(&spec)?;
        let (host_id, user_id) = self.data_identity(alias, vault)?;
        let host = entity_id_from_hex(&host_id)?;
        let user = entity_id_from_hex(&user_id)?;
        let bytes = Zeroizing::new(serde_json::to_vec(&spec)?);
        let hash = prefixed_hash(INTENT_HASH, &bytes);
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        if let Some(existing) = hard.mutation(&id)? {
            check_actor(&existing, host.as_bytes(), user.as_bytes())?;
            if existing.request_hash != hash {
                return Err(Error::InvalidAccount(
                    "submission ID is bound to different inputs",
                ));
            }
            return outcome(&hard, &existing);
        }
        let all = hard.adapter_mutations(host.as_bytes(), user.as_bytes())?;
        if all.len() >= 4096
            || all
                .iter()
                .filter(|op| {
                    matches!(
                        op.state,
                        MutationState::Prepared
                            | MutationState::Submitting
                            | MutationState::SubmissionUnknown
                            | MutationState::RemoteVerified
                    )
                })
                .count()
                >= 64
        {
            return Err(Error::InvalidAccount(
                "adapter submission inventory is full",
            ));
        }
        let team = spec
            .team_selector
            .as_deref()
            .map(|selector| {
                self.resolve_data_team(alias, selector, vault)
                    .and_then(|id| entity_id_from_hex(&id))
            })
            .transpose()?;
        let mut protected = self.data_mutations(master)?;
        let op = MutationCoordinator::new(&self.paths.hard_database, &mut protected).prepare(
            MutationDraft {
                operation_id: id,
                kind: MutationKind::KvAdapter,
                host_id: host.as_bytes().to_vec(),
                scope_id: user.as_bytes().to_vec(),
                subject_id: team.map(|id| id.as_bytes().to_vec()).unwrap_or_default(),
                expected_version: None,
                request_hash: hash,
            },
            bytes,
        )?;
        outcome(&hard, &op)
    }

    pub fn pending_data_writes(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<DataWriteOutcome>> {
        let (host, user) = self.data_identity(alias, vault)?;
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        hard.adapter_mutations(
            entity_id_from_hex(&host)?.as_bytes(),
            entity_id_from_hex(&user)?.as_bytes(),
        )?
        .iter()
        .filter(|op| !matches!(op.state, MutationState::Finalized | MutationState::Rejected))
        .map(|op| outcome(&hard, op))
        .collect()
    }

    /// Status performs proof reads only; it never sends a prepared namespace request.
    pub fn data_write_status(
        &self,
        alias: &str,
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<DataWriteOutcome> {
        let op = self.data_operation(alias, id, vault)?;
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        if !matches!(
            op.state,
            MutationState::Submitting
                | MutationState::SubmissionUnknown
                | MutationState::RemoteVerified
        ) {
            return outcome(&hard, &op);
        }
        let children = hard.mutation_children(&id)?;
        let team = (!op.subject_id.is_empty()).then(|| hex(&op.subject_id));
        if children.iter().any(|(child, last)| {
            *last
                && matches!(
                    child.state,
                    MutationState::Submitting
                        | MutationState::SubmissionUnknown
                        | MutationState::RemoteVerified
                )
        }) {
            let (account, user, team) = self.data_context(alias, team.as_deref(), vault)?;
            let host = self.pinned_host()?;
            let mut protected = self.data_mutations(master)?;
            let mut session = match &team {
                Some(team) => self.client.team_kv_write_session(
                    &host,
                    &account.credential,
                    team,
                    &self.paths.soft_database,
                    &mut protected,
                )?,
                None => self.client.user_kv_write_session(
                    &host,
                    &account.credential,
                    &user.verified,
                    &user.puks,
                    &self.paths.soft_database,
                    &mut protected,
                )?,
            };
            for (child, last) in children {
                if last
                    && matches!(
                        child.state,
                        MutationState::Submitting
                            | MutationState::SubmissionUnknown
                            | MutationState::RemoteVerified
                    )
                {
                    // Failure to observe an exact committed dirent remains unknown.
                    let _ = session.inspect_namespace_mutation(child.operation_id);
                }
            }
        }
        let current = outcome(&hard, &op)?;
        if current.status == DataWriteStatus::Committed {
            let mut protected = self.data_mutations(master)?;
            MutationCoordinator::new(&self.paths.hard_database, &mut protected)
                .remote_verified_and_finalize(&id)?;
        }
        Ok(current)
    }

    /// Execute once. The body is bounded and hash-checked before crossing the
    /// durable no-replay boundary. A repeated execute only returns stored status.
    pub fn execute_data_write<R: Read>(
        &self,
        alias: &str,
        id: [u8; 16],
        reader: &mut R,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<DataWriteOutcome> {
        self.profile.require(Capability::Kv)?;
        let op = self.data_operation(alias, id, vault)?;
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        if op.state != MutationState::Prepared {
            return outcome(&hard, &op);
        }
        let mut protected = self.data_mutations(master)?;
        let material = MutationCoordinator::new(&self.paths.hard_database, &mut protected)
            .load_bound_material(&op)?;
        if prefixed_hash(INTENT_HASH, &material) != op.request_hash {
            return Err(Error::InvalidAccount("adapter input binding changed"));
        }
        let spec: DataWriteSpec = serde_json::from_slice(&material)?;
        validate_spec(&spec)?;
        let mut body = Zeroizing::new(Vec::new());
        reader.take((MAX_BODY + 1) as u64).read_to_end(&mut body)?;
        if body.len() > MAX_BODY
            || body.len() as u64 != spec.body_length
            || foks_crypto::kv_adapter_body_hash(&body) != spec.body_hash
        {
            return Err(Error::InvalidAccount(
                "upload does not match prepared input",
            ));
        }
        let team = (!op.subject_id.is_empty()).then(|| hex(&op.subject_id));
        let (account, user, team) = self.data_context(alias, team.as_deref(), vault)?;
        let host = self.pinned_host()?;
        MutationCoordinator::new(&self.paths.hard_database, &mut protected)
            .begin_submission(&id)?;
        let mut remote_possible = false;
        let result = (|| -> Result<Option<String>> {
            let mut session = match &team {
                Some(team) => self.client.team_kv_write_session(
                    &host,
                    &account.credential,
                    team,
                    &self.paths.soft_database,
                    &mut protected,
                )?,
                None => self.client.user_kv_write_session(
                    &host,
                    &account.credential,
                    &user.verified,
                    &user.puks,
                    &self.paths.soft_database,
                    &mut protected,
                )?,
            };
            session.bind_adapter_intent(id)?;
            apply_write(
                &mut session,
                &spec,
                &mut body.as_slice(),
                team.is_some(),
                &mut remote_possible,
            )
        })();
        let mut coordinator = MutationCoordinator::new(&self.paths.hard_database, &mut protected);
        match result {
            Ok(node_id) => {
                coordinator.remote_verified_and_finalize(&id)?;
                Ok(DataWriteOutcome {
                    submission_id: hex(&id),
                    status: DataWriteStatus::Committed,
                    partial: false,
                    node_id,
                })
            }
            Err(_) => {
                // Even a local failure may follow root creation or an uploaded
                // object. Retain the handle; never reinterpret it as safe replay.
                if remote_possible {
                    coordinator.submission_unknown(&id)?;
                } else {
                    coordinator.rejected(&id)?;
                }
                outcome(
                    &hard,
                    &hard
                        .mutation(&id)?
                        .ok_or(Error::InvalidAccount("missing adapter intent"))?,
                )
            }
        }
    }

    fn data_operation(
        &self,
        alias: &str,
        id: [u8; 16],
        vault: &mut AccountVault<'_>,
    ) -> Result<MutationOperation> {
        let (host, user) = self.data_identity(alias, vault)?;
        let op = HardStateStore::open(&self.paths.hard_database)?
            .mutation(&id)?
            .ok_or(Error::InvalidAccount("unknown submission ID"))?;
        check_actor(
            &op,
            entity_id_from_hex(&host)?.as_bytes(),
            entity_id_from_hex(&user)?.as_bytes(),
        )?;
        Ok(op)
    }

    fn data_mutations(&self, master: &[u8; 32]) -> Result<EncryptedFileMutationStore> {
        Ok(EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?)
    }
}

fn check_actor(op: &MutationOperation, host: &[u8], user: &[u8]) -> Result<()> {
    if op.kind != MutationKind::KvAdapter || op.host_id != host || op.scope_id != user {
        return Err(Error::InvalidAccount(
            "submission belongs to another account",
        ));
    }
    Ok(())
}

fn outcome(hard: &HardStateStore, op: &MutationOperation) -> Result<DataWriteOutcome> {
    let children = hard.mutation_children(&op.operation_id)?;
    let committed = |child: &MutationOperation| {
        matches!(
            child.state,
            MutationState::RemoteVerified | MutationState::Finalized
        )
    };
    let status = match op.state {
        MutationState::Prepared => DataWriteStatus::Prepared,
        MutationState::RemoteVerified | MutationState::Finalized => DataWriteStatus::Committed,
        MutationState::Rejected => DataWriteStatus::Rejected,
        _ if children
            .iter()
            .any(|(child, last)| *last && committed(child)) =>
        {
            DataWriteStatus::Committed
        }
        _ => DataWriteStatus::SubmissionUnknown,
    };
    Ok(DataWriteOutcome {
        submission_id: hex(&op.operation_id),
        status,
        partial: status == DataWriteStatus::SubmissionUnknown
            && children.iter().any(|(child, _)| committed(child)),
        node_id: None,
    })
}

fn validate_spec(spec: &DataWriteSpec) -> Result<()> {
    if spec.path.len() > 4096
        || spec
            .destination
            .as_ref()
            .is_some_and(|path| path.len() > 4096)
        || spec
            .team_selector
            .as_ref()
            .is_some_and(|team| team.len() > 512)
        || spec.body_length > MAX_BODY as u64
    {
        return Err(Error::InvalidAccount("adapter input exceeds limits"));
    }
    split_parent(&spec.path)?;
    if path_components(&spec.path)?.len() > 64 {
        return Err(Error::InvalidKvPath("adapter path depth exceeds limit"));
    }
    if (spec.kind == DataWriteKind::Move) != spec.destination.is_some()
        || (spec.kind != DataWriteKind::Put && (spec.body_length != 0 || spec.overwrite))
        || (spec.kind != DataWriteKind::Remove && spec.recursive)
        || (!matches!(spec.kind, DataWriteKind::Put | DataWriteKind::Mkdir) && spec.mkdir_p)
    {
        return Err(Error::InvalidAccount("invalid adapter write arguments"));
    }
    if let Some(destination) = &spec.destination {
        split_parent(destination)?;
        if path_components(destination)?.len() > 64 {
            return Err(Error::InvalidKvPath("adapter path depth exceeds limit"));
        }
    }
    Ok(())
}
