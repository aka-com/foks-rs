//! Durable adapter intents. Public journals contain fingerprints and immutable IDs;
//! paths and arguments live only in the existing protected mutation store.
use super::*;
use foks_client::MutationDraft;
use foks_client::ProtectedMutationStore as _;
use foks_client_db::{AdapterLedgerState, AdapterSubmission, MutationOperation};

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
    Expired,
    NotRecorded,
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
        handle: SubmissionHandle,
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
        let mut hard = HardStateStore::open(&self.paths.hard_database)?;
        if let Some(existing) = hard.adapter_submission(handle)? {
            check_binding(&existing, host.as_bytes(), user.as_bytes())?;
            if existing.input_hash != hash {
                return Err(foks_client_db::Error::AdapterIdentityConflict.into());
            }
            return outcome(&hard, &existing);
        }
        let sample = self.adapter_clock.sample()?;
        match hard.check_adapter_admission(host.as_bytes(), user.as_bytes(), handle, sample) {
            Err(foks_client_db::Error::AdapterExpired) => {
                return Ok(absent_outcome(handle, DataWriteStatus::Expired))
            }
            result => result?,
        }
        // One bounded local cleanup batch can release eligible capacity. Any
        // failed erasure preserves full ownership and the reserved ledger slot.
        let deadline = std::time::Instant::now() + Duration::from_millis(50);
        for entry in hard.adapter_submission_batch(host.as_bytes(), user.as_bytes(), false)? {
            if std::time::Instant::now() >= deadline {
                break;
            }
            if entry.terminal_at.is_none() {
                hard.finish_adapter_submission(
                    entry.handle,
                    entry.state == AdapterLedgerState::Committed,
                    entry.ancillary_committed,
                    entry.node_id,
                    Some(sample),
                )?;
            }
            match self.compact_data_write(&mut hard, &entry, master, deadline) {
                Ok(())
                | Err(Error::ProtectedStore(_))
                | Err(Error::ClientDatabase(foks_client_db::Error::AdapterCleanupDeferred)) => (),
                Err(error) => return Err(error),
            }
        }
        // Live and cleanup-deferred entries cannot match pruning eligibility.
        hard.prune_adapter_submissions(host.as_bytes(), user.as_bytes(), sample)?;
        let mut id = [0; 16];
        let mut allocated = false;
        for _ in 0..32 {
            getrandom::fill(&mut id).map_err(|_| Error::Randomness)?;
            if hard.mutation(&id)?.is_none() {
                allocated = true;
                break;
            }
        }
        if !allocated {
            return Err(Error::Randomness);
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
        let admission = MutationCoordinator::new(&self.paths.hard_database, &mut protected)
            .prepare_adapter_submission(
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
                handle,
                sample,
            );
        match admission {
            Err(foks_client::Error::Database(foks_client_db::Error::AdapterExpired)) => {
                return Ok(absent_outcome(handle, DataWriteStatus::Expired));
            }
            result => {
                result?;
            }
        }
        outcome(
            &hard,
            &hard
                .adapter_submission(handle)?
                .ok_or(foks_client_db::Error::AdapterIdentityConflict)?,
        )
    }

    pub fn pending_data_writes(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<DataWriteOutcome>> {
        let (host, user) = self.data_identity(alias, vault)?;
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        hard.adapter_submission_batch(
            entity_id_from_hex(&host)?.as_bytes(),
            entity_id_from_hex(&user)?.as_bytes(),
            true,
        )?
        .iter()
        .map(|entry| outcome(&hard, entry))
        .collect()
    }

    /// Status performs proof reads only; it never sends a prepared namespace request.
    pub fn data_write_status(
        &self,
        alias: &str,
        handle: SubmissionHandle,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<DataWriteOutcome> {
        let Some(entry) = self.data_operation(alias, handle, vault)? else {
            return self.unseen_data_status(alias, handle, vault);
        };
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        if entry.state != AdapterLedgerState::Live {
            return outcome(&hard, &entry);
        }
        let id = entry
            .internal_id
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        let op = hard
            .mutation(&id)?
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        if !matches!(
            op.state,
            MutationState::Submitting
                | MutationState::SubmissionUnknown
                | MutationState::RemoteVerified
        ) {
            return outcome(&hard, &entry);
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
        let entry = hard
            .adapter_submission(handle)?
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        let current = outcome(&hard, &entry)?;
        if current.status == DataWriteStatus::Committed {
            self.finish_data_write(handle, true, current.partial, entry.node_id, master)?;
        }
        Ok(current)
    }

    /// Execute once. The body is bounded and hash-checked before crossing the
    /// durable no-replay boundary. A repeated execute only returns stored status.
    pub fn execute_data_write<R: Read>(
        &self,
        alias: &str,
        handle: SubmissionHandle,
        reader: &mut R,
        vault: &mut AccountVault<'_>,
        master: &[u8; 32],
    ) -> Result<DataWriteOutcome> {
        self.profile.require(Capability::Kv)?;
        let Some(entry) = self.data_operation(alias, handle, vault)? else {
            let absent = self.unseen_data_status(alias, handle, vault)?;
            return if absent.status == DataWriteStatus::Expired {
                Ok(absent)
            } else {
                Err(Error::InvalidAccount(
                    "submission is not recorded; prepare before execution",
                ))
            };
        };
        let hard = HardStateStore::open(&self.paths.hard_database)?;
        if entry.state != AdapterLedgerState::Live {
            return outcome(&hard, &entry);
        }
        let id = entry
            .internal_id
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        let op = hard
            .mutation(&id)?
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        if op.state != MutationState::Prepared {
            return outcome(&hard, &entry);
        }
        let mut protected = self.data_mutations(master)?;
        let material = MutationCoordinator::new(&self.paths.hard_database, &mut protected)
            .load_bound_request(&op)?;
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
        match result {
            Ok(node_id) => {
                let node_id = node_id.as_deref().map(parse_node_id).transpose()?;
                self.finish_data_write(handle, true, false, node_id, master)?;
            }
            Err(_) => {
                // Remote ambiguity is permanent until exact authenticated proof.
                if remote_possible {
                    MutationCoordinator::new(&self.paths.hard_database, &mut protected)
                        .submission_unknown(&id)?;
                } else {
                    self.finish_data_write(handle, false, false, None, master)?;
                }
            }
        }
        outcome(
            &hard,
            &hard
                .adapter_submission(handle)?
                .ok_or(foks_client_db::Error::AdapterIdentityConflict)?,
        )
    }

    fn data_operation(
        &self,
        alias: &str,
        handle: SubmissionHandle,
        vault: &mut AccountVault<'_>,
    ) -> Result<Option<AdapterSubmission>> {
        let (host, user) = self.data_identity(alias, vault)?;
        let entry = HardStateStore::open(&self.paths.hard_database)?.adapter_submission(handle)?;
        if let Some(entry) = &entry {
            check_binding(
                entry,
                entity_id_from_hex(&host)?.as_bytes(),
                entity_id_from_hex(&user)?.as_bytes(),
            )?;
        }
        Ok(entry)
    }

    fn unseen_data_status(
        &self,
        alias: &str,
        handle: SubmissionHandle,
        vault: &mut AccountVault<'_>,
    ) -> Result<DataWriteOutcome> {
        let (host, user) = self.data_identity(alias, vault)?;
        let status = match HardStateStore::open(&self.paths.hard_database)?.check_adapter_admission(
            entity_id_from_hex(&host)?.as_bytes(),
            entity_id_from_hex(&user)?.as_bytes(),
            handle,
            self.adapter_clock.sample()?,
        ) {
            Ok(()) => DataWriteStatus::NotRecorded,
            Err(foks_client_db::Error::AdapterExpired) => DataWriteStatus::Expired,
            Err(error) => return Err(error.into()),
        };
        Ok(absent_outcome(handle, status))
    }

    fn finish_data_write(
        &self,
        handle: SubmissionHandle,
        committed: bool,
        ancillary: bool,
        node_id: Option<[u8; 17]>,
        master: &[u8; 32],
    ) -> Result<()> {
        let mut hard = HardStateStore::open(&self.paths.hard_database)?;
        hard.finish_adapter_submission(
            handle,
            committed,
            ancillary,
            node_id,
            self.adapter_clock.sample().ok(),
        )?;
        // A failed erasure retains terminal ownership and its reserved ledger slot.
        // Returning the committed outcome remains safe; maintenance retries cleanup.
        let entry = hard
            .adapter_submission(handle)?
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        match self.compact_data_write(
            &mut hard,
            &entry,
            master,
            std::time::Instant::now() + Duration::from_millis(50),
        ) {
            Ok(())
            | Err(Error::ProtectedStore(_))
            | Err(Error::ClientDatabase(foks_client_db::Error::AdapterCleanupDeferred)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn compact_data_write(
        &self,
        hard: &mut HardStateStore,
        entry: &AdapterSubmission,
        master: &[u8; 32],
        deadline: std::time::Instant,
    ) -> Result<()> {
        let Some(id) = entry.internal_id else {
            return Ok(());
        };
        let parent = hard
            .mutation(&id)?
            .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
        let children = hard.mutation_children(&id)?;
        if !parent.state.is_terminal()
            || children.iter().any(|(child, _)| !child.state.is_terminal())
        {
            return Err(foks_client_db::Error::AdapterCleanupDeferred.into());
        }
        let mut protected = self.data_mutations(master)?;
        for operation in std::iter::once(&parent).chain(children.iter().map(|(child, _)| child)) {
            if std::time::Instant::now() >= deadline {
                return Err(foks_client_db::Error::AdapterCleanupDeferred.into());
            }
            match protected.remove(&operation.material_ref) {
                Ok(()) | Err(foks_client::ProtectedStoreError::Missing) => (),
                Err(error) => return Err(error.into()),
            }
        }
        protected.sync()?;
        hard.compact_adapter_submission(entry.handle)?;
        Ok(())
    }

    fn data_mutations(&self, master: &[u8; 32]) -> Result<EncryptedFileMutationStore> {
        Ok(EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master),
        )?)
    }
}

fn check_binding(entry: &AdapterSubmission, host: &[u8], user: &[u8]) -> Result<()> {
    if entry.host_id != host || entry.user_id != user {
        return Err(foks_client_db::Error::AdapterIdentityConflict.into());
    }
    Ok(())
}

fn absent_outcome(handle: SubmissionHandle, status: DataWriteStatus) -> DataWriteOutcome {
    DataWriteOutcome {
        submission_id: handle.to_string(),
        status,
        partial: false,
        node_id: None,
    }
}

fn parse_node_id(input: &str) -> Result<[u8; 17]> {
    if input.len() != 34 || !input.bytes().all(|b| matches!(b,b'0'..=b'9'|b'a'..=b'f')) {
        return Err(Error::InvalidAccount("invalid proven adapter node ID"));
    }
    let mut node = [0; 17];
    for (index, byte) in node.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&input[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::InvalidAccount("invalid proven adapter node ID"))?;
    }
    Ok(node)
}

fn outcome(hard: &HardStateStore, entry: &AdapterSubmission) -> Result<DataWriteOutcome> {
    let (status, partial) = match entry.state {
        AdapterLedgerState::Committed => (DataWriteStatus::Committed, false),
        AdapterLedgerState::Rejected => (DataWriteStatus::Rejected, entry.ancillary_committed),
        AdapterLedgerState::Live => {
            let id = entry
                .internal_id
                .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
            let op = hard
                .mutation(&id)?
                .ok_or(foks_client_db::Error::AdapterIdentityConflict)?;
            if op.kind != MutationKind::KvAdapter
                || op.host_id != entry.host_id
                || op.scope_id != entry.user_id
                || op.subject_id != entry.team_id
                || op.request_hash != entry.input_hash
            {
                return Err(foks_client_db::Error::AdapterIdentityConflict.into());
            }
            let children = hard.mutation_children(&id)?;
            let proven = |op: &MutationOperation| {
                matches!(
                    op.state,
                    MutationState::RemoteVerified | MutationState::Finalized
                )
            };
            let status = match op.state {
                MutationState::Prepared => DataWriteStatus::Prepared,
                MutationState::Finalized | MutationState::RemoteVerified => {
                    DataWriteStatus::Committed
                }
                MutationState::Rejected => DataWriteStatus::Rejected,
                _ if children.iter().any(|(child, last)| *last && proven(child)) => {
                    DataWriteStatus::Committed
                }
                _ => DataWriteStatus::SubmissionUnknown,
            };
            (
                status,
                status == DataWriteStatus::SubmissionUnknown
                    && children.iter().any(|(child, _)| proven(child)),
            )
        }
    };
    Ok(DataWriteOutcome {
        submission_id: entry.handle.to_string(),
        status,
        partial,
        node_id: entry.node_id.as_ref().map(|id| hex(id)),
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
        path_components(destination)?;
        if path_components(destination)?.len() > 64 {
            return Err(Error::InvalidKvPath("adapter path depth exceeds limit"));
        }
    }
    Ok(())
}
