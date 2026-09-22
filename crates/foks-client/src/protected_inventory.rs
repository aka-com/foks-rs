//! One key vocabulary for protected writers, reconciliation, and state export.
//! Logical keys and typed owners are private application data, never agent DTOs.
use crate::{EncryptedFileMutationStore, ProtectedStoreError};
use foks_client_db::{
    HardStateMetadata, HardStateStore, MutationKind, ProtectedOwnerCursor, ProtectedRecordOwner,
    SsoFlowState,
};
use std::collections::BTreeMap;
use std::time::Instant;

/// Adding a writer requires adding its family here and its owner below.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedRecordFamily {
    Mutation,
    Chat,
    Sso(u8),
    TeamRekey,
    TeamRotation,
    TeamMetadata,
    RemoteAddition,
    InvitationAck,
}
/// A key constructor carries all scope required by its family, so callers cannot
/// accidentally derive a chat key from an operation ID alone.
pub enum ProtectedRecordKey<'a> {
    Mutation(&'a [u8; 16]),
    Chat(&'a foks_client_db::ChatOperation),
    Sso(&'a [u8; 16], u8),
    TeamRekey(&'a [u8; 16]),
    TeamRotation(&'a [u8; 16]),
    TeamMetadata(&'a [u8; 16]),
    RemoteAddition(&'a [u8; 16]),
    InvitationAck(&'a [u8; 16]),
}
impl ProtectedRecordKey<'_> {
    pub fn encoded(self) -> Vec<u8> {
        match self {
            Self::Mutation(id) => id.to_vec(),
            Self::Sso(id, stage) => [b"foks-client-sso-v1".as_slice(), id, &[stage]].concat(),
            Self::TeamRekey(id) => [b"team-member-key-refresh-request-v1:".as_slice(), id].concat(),
            Self::TeamRotation(id) => [b"team-rotation-request-v1:".as_slice(), id].concat(),
            Self::TeamMetadata(id) => [b"team-index-range:".as_slice(), id].concat(),
            Self::RemoteAddition(id) => {
                [b"federation-remote-team-addition-v1:".as_slice(), id].concat()
            }
            Self::InvitationAck(id) => [id.as_slice(), b"/invitation-ack"].concat(),
            Self::Chat(op) => [
                b"chat-operation-v1".as_slice(),
                &op.scope.host,
                &op.scope.uid,
                &op.scope.team,
                &op.scope.channel,
                &op.id,
            ]
            .concat(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedPresence {
    Required,
    Optional,
    TerminalCleanup,
}

#[derive(Clone, Debug)]
pub struct ProtectedRecordDescriptor {
    pub key: Vec<u8>,
    pub family: ProtectedRecordFamily,
    pub presence: ProtectedPresence,
    pub owner: ProtectedRecordOwner,
}

impl ProtectedRecordDescriptor {
    /// Exportability is an owner-state rule, independent of material presence.
    /// Existing terminal bytes still require typed validation before transfer.
    pub fn exportable(&self) -> bool {
        match &self.owner {
            ProtectedRecordOwner::Mutation(op) => op.state.is_terminal(),
            ProtectedRecordOwner::Chat(op) => op.state.is_terminal(),
            ProtectedRecordOwner::Sso(flow) => matches!(
                flow.state,
                SsoFlowState::Complete
                    | SsoFlowState::Cancelled
                    | SsoFlowState::Expired
                    | SsoFlowState::Rejected
                    | SsoFlowState::Denied
            ),
            ProtectedRecordOwner::Team(op) => matches!(
                op.state,
                foks_client_db::TeamMutationState::Verified
                    | foks_client_db::TeamMutationState::Rejected
                    | foks_client_db::TeamMutationState::Superseded
            ),
        }
    }

    /// Fingerprint and typed scope validation lives with the owning workflow.
    /// AEAD authentication is additionally performed by the encrypted store.
    pub fn validate_payload(&self, bytes: &[u8]) -> crate::Result<()> {
        match (&self.owner, self.family) {
            (ProtectedRecordOwner::Mutation(op), ProtectedRecordFamily::Mutation) => {
                crate::mutation::validate_protected_request(op, bytes)
            }
            (ProtectedRecordOwner::Mutation(_), ProtectedRecordFamily::InvitationAck) => {
                if !bytes.is_empty() {
                    foks_proto::TeamRsvp::decode(bytes)?;
                }
                Ok(())
            }
            (ProtectedRecordOwner::Chat(op), ProtectedRecordFamily::Chat) => {
                crate::realtime::validate_inventory_request(op, bytes).map(|_| ())
            }
            (ProtectedRecordOwner::Sso(flow), ProtectedRecordFamily::Sso(stage)) => {
                crate::sso::validate_inventory_stage(flow, stage, bytes)
            }
            (
                ProtectedRecordOwner::Team(op),
                ProtectedRecordFamily::TeamRekey
                | ProtectedRecordFamily::TeamRotation
                | ProtectedRecordFamily::TeamMetadata
                | ProtectedRecordFamily::RemoteAddition,
            ) => crate::team::validate_inventory_request(op, bytes),
            _ => Err(crate::Error::OperationBinding(
                "protected descriptor family differs from owner",
            )),
        }
    }
}

/// Complete authority is available only after all four owner tables are exhausted.
/// Saturation fails closed rather than treating a truncated set as authoritative.
#[derive(Default)]
pub struct ProtectedRecordInventory {
    revision: Option<HardStateMetadata>,
    cursor: ProtectedOwnerCursor,
    records: BTreeMap<String, ProtectedRecordDescriptor>,
    bytes: usize,
    complete: bool,
    saturated: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ProtectedInventoryProgress {
    pub examined: u64,
    pub complete: bool,
    pub saturated: bool,
    pub restarted: bool,
}
impl ProtectedRecordInventory {
    pub fn advance(
        &mut self,
        hard: &HardStateStore,
        deadline: Instant,
    ) -> crate::Result<ProtectedInventoryProgress> {
        let revision = hard.metadata()?;
        let restarted = self.revision.is_some_and(|old| old != revision);
        if self.revision != Some(revision) {
            *self = Self {
                revision: Some(revision),
                ..Self::default()
            };
        }
        let mut progress = ProtectedInventoryProgress {
            restarted,
            ..Default::default()
        };
        while !self.complete
            && !self.saturated
            && progress.examined < 256
            && Instant::now() < deadline
        {
            let Some(owner) = hard.next_protected_owner(&mut self.cursor)? else {
                self.complete = true;
                break;
            };
            progress.examined += 1;
            for record in descriptors(owner) {
                // Descriptor fields are schema-bounded. Charge a conservative 2 KiB
                // per owner copy as well as the logical key and filename storage.
                let bytes = 2048 + record.key.len() + 68;
                if self.records.len() >= 2048 || self.bytes + bytes > 4 * 1024 * 1024 {
                    self.saturated = true;
                    break;
                }
                let name = EncryptedFileMutationStore::record_filename(&record.key)
                    .map_err(|e| crate::Error::ProtectedStore(e.to_string()))?;
                if self.records.insert(name, record).is_some() {
                    self.saturated = true;
                    return Err(crate::Error::OperationBinding(
                        "duplicate protected record ownership",
                    ));
                }
                self.bytes += bytes;
            }
        }
        // The caller holds writer exclusion, but do not trust that assumption alone.
        if hard.metadata()? != revision {
            *self = Self::default();
            return Err(crate::Error::OperationBinding(
                "protected inventory revision changed",
            ));
        }
        progress.complete = self.complete;
        progress.saturated = self.saturated;
        Ok(progress)
    }

    pub fn records(
        &self,
        revision: HardStateMetadata,
    ) -> Result<&BTreeMap<String, ProtectedRecordDescriptor>, ProtectedStoreError> {
        if !self.complete || self.saturated || self.revision != Some(revision) {
            return Err(ProtectedStoreError::Backend(
                "protected inventory is incomplete or stale".into(),
            ));
        }
        Ok(&self.records)
    }
}

fn descriptors(owner: ProtectedRecordOwner) -> Vec<ProtectedRecordDescriptor> {
    use ProtectedPresence::*;
    use ProtectedRecordFamily as F;
    use ProtectedRecordKey as K;
    let mut keys = Vec::with_capacity(4);
    match &owner {
        ProtectedRecordOwner::Mutation(op) => {
            let presence = if op.state.is_terminal() {
                TerminalCleanup
            } else {
                Required
            };
            keys.push((op.material_ref.clone(), F::Mutation, presence));
            if op.kind == MutationKind::Invitation {
                keys.push((
                    K::InvitationAck(&op.operation_id).encoded(),
                    F::InvitationAck,
                    if op.state.is_terminal() {
                        TerminalCleanup
                    } else {
                        Optional
                    },
                ));
            }
        }
        ProtectedRecordOwner::Chat(op) => keys.push((
            K::Chat(op).encoded(),
            F::Chat,
            if op.state.is_terminal() {
                TerminalCleanup
            } else {
                Required
            },
        )),
        ProtectedRecordOwner::Sso(flow) => {
            let required = match flow.state {
                SsoFlowState::Prepared => Some(0),
                SsoFlowState::AwaitingBrowser => Some(1),
                SsoFlowState::Ready => Some(2),
                SsoFlowState::Binding => Some(3),
                _ => None,
            };
            for stage in 0..4 {
                let presence = if required.is_some_and(|last| stage <= last) {
                    Required
                } else if required.is_some() || flow.state == SsoFlowState::Unknown {
                    Optional
                } else {
                    TerminalCleanup
                };
                keys.push((K::Sso(&flow.id, stage).encoded(), F::Sso(stage), presence));
            }
        }
        ProtectedRecordOwner::Team(op) => {
            // Older team APIs optionally accept a protected backend and do not
            // persist its selection. Preserve every legal variant conservatively.
            for (family, key) in [
                (F::TeamRekey, K::TeamRekey(&op.operation_id)),
                (F::TeamRotation, K::TeamRotation(&op.operation_id)),
                (F::TeamMetadata, K::TeamMetadata(&op.operation_id)),
                (F::RemoteAddition, K::RemoteAddition(&op.operation_id)),
            ] {
                keys.push((key.encoded(), family, Optional));
            }
        }
    }
    keys.into_iter()
        .map(|(key, family, presence)| ProtectedRecordDescriptor {
            key,
            family,
            presence,
            owner: owner.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProtectedMutationStore;
    use rusqlite::params;
    use std::time::Duration;
    use zeroize::Zeroizing;

    fn fixture() -> (tempfile::TempDir, HardStateStore, Vec<u8>) {
        let dir = tempfile::tempdir().unwrap();
        let mut hard = HardStateStore::open(&dir.path().join("hard")).unwrap();
        let snapshot = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap()
        .snapshot;
        hard.accept_verified_host(&snapshot).unwrap();
        (dir, hard, snapshot.host_id().to_vec())
    }
    fn seed_mutations(dir: &std::path::Path, host: &[u8], start: u128, count: u128) {
        let mut conn = rusqlite::Connection::open(dir.join("hard")).unwrap();
        let tx = conn.transaction().unwrap();
        for n in start..start + count {
            let id = n.to_be_bytes();
            tx.execute(
                "INSERT INTO mutation_operations VALUES (?1,11,?2,?3,X'',NULL,?4,?1,?4,5,0,1,1)",
                params![id, host, vec![1u8; 33], [0u8; 32]],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    fn complete(inventory: &mut ProtectedRecordInventory, hard: &HardStateStore) {
        loop {
            let p = inventory
                .advance(hard, Instant::now() + Duration::from_secs(1))
                .unwrap();
            assert!(p.examined <= 256);
            assert!(!p.saturated);
            if p.complete {
                break;
            }
        }
    }

    #[test]
    fn complete_mixed_inventory_preserves_every_family_and_removes_only_orphans() {
        let (dir, hard, host) = fixture();
        seed_mutations(dir.path(), &host, 1, 1);
        let conn = rusqlite::Connection::open(dir.path().join("hard")).unwrap();
        conn.execute(
            "UPDATE mutation_operations SET state=3 WHERE operation_id=?1",
            [1u128.to_be_bytes()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_operations (operation_id,host_id,uid,team_id,channel_id,kind,state,request_hash,scan_cursor,receipt,rejection_code) VALUES (?1,?2,?3,?4,?5,1,1,?6,0,NULL,NULL)",
            params![
                [2u8; 16],
                host,
                vec![1u8; 33],
                vec![2u8; 33],
                [3u8; 16],
                [0u8; 32]
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sso_flows(operation_id,host_id,uid,device_id,purpose,state,material_hash,config_hash,expires_at,final_operation,commitment) VALUES (?1,?2,?3,?4,1,7,?5,?5,100,NULL,?5)",
            params![[3u8; 16], host, vec![1u8; 33], vec![2u8; 33], [0u8; 32]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO team_mutation_operations VALUES (?1,3,?2,?3,?3,?4,1,?5,3,1,1)",
            params![[4u8; 16], host, vec![1u8; 33], vec![2u8; 33], [0u8; 32]],
        )
        .unwrap();
        let mut inventory = ProtectedRecordInventory::default();
        complete(&mut inventory, &hard);
        let records = inventory.records(hard.metadata().unwrap()).unwrap();
        assert_eq!(records.len(), 11); // generic + ack + chat + four SSO + four team variants
        assert!(records
            .values()
            .filter(|r| matches!(r.family, ProtectedRecordFamily::Sso(_)))
            .all(|r| r.presence == ProtectedPresence::Optional));
        let mut protected = EncryptedFileMutationStore::open(
            dir.path().join("mutations"),
            Zeroizing::new([7u8; 32]),
        )
        .unwrap();
        for r in records.values() {
            protected
                .put_if_absent(&r.key, b"live exact request")
                .unwrap();
        }
        // Simulate process death after installation, before journal insertion.
        protected
            .put_if_absent(b"installed without journal", b"orphan")
            .unwrap();
        let mut scan = protected.temporary_scan().unwrap();
        let report = protected
            .reconcile_unowned_until(
                &mut scan,
                &inventory,
                hard.metadata().unwrap(),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        assert!(report.complete);
        assert_eq!(report.final_removed, 1);
        for r in records.values() {
            assert_eq!(&*protected.get(&r.key).unwrap(), b"live exact request");
        }
        assert!(matches!(
            protected.get(b"installed without journal"),
            Err(ProtectedStoreError::Missing)
        ));
        let report = protected
            .reconcile_unowned_until(
                &mut protected.temporary_scan().unwrap(),
                &inventory,
                hard.metadata().unwrap(),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(report.removed, 0);
    }

    #[test]
    fn partial_stale_and_oversized_inventories_never_authorize_deletion() {
        let (dir, hard, host) = fixture();
        seed_mutations(dir.path(), &host, 1, 600);
        let mut inventory = ProtectedRecordInventory::default();
        let first = inventory
            .advance(&hard, Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert_eq!(first.examined, 256);
        assert!(!first.complete);
        assert!(inventory.records(hard.metadata().unwrap()).is_err());
        // A supported writer changes the revision between reacquired passes.
        seed_mutations(dir.path(), &host, 601, 1);
        let second = inventory
            .advance(&hard, Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert!(second.restarted);
        assert!(!second.complete);
        complete(&mut inventory, &hard);
        assert_eq!(
            inventory.records(hard.metadata().unwrap()).unwrap().len(),
            1202
        );
        seed_mutations(dir.path(), &host, 602, 600);
        assert!(inventory.records(hard.metadata().unwrap()).is_err());
        loop {
            let p = inventory
                .advance(&hard, Instant::now() + Duration::from_secs(1))
                .unwrap();
            assert!(p.examined <= 256);
            assert!(!p.complete);
            if p.saturated {
                break;
            }
        }
        assert!(inventory.records(hard.metadata().unwrap()).is_err());
        assert!(inventory.bytes <= 4 * 1024 * 1024);
    }

    #[cfg(unix)]
    #[test]
    fn replacing_directory_invalidates_reacquired_scan() {
        let (dir, hard, _) = fixture();
        let path = dir.path().join("mutations");
        let mut protected =
            EncryptedFileMutationStore::open(&path, Zeroizing::new([7u8; 32])).unwrap();
        protected.put_if_absent(b"orphan", b"material").unwrap();
        let mut scan = protected.temporary_scan().unwrap();
        let mut inventory = ProtectedRecordInventory::default();
        complete(&mut inventory, &hard);
        std::fs::rename(&path, dir.path().join("old")).unwrap();
        let mut replacement =
            EncryptedFileMutationStore::open(&path, Zeroizing::new([7u8; 32])).unwrap();
        replacement
            .put_if_absent(b"orphan", b"replacement")
            .unwrap();
        assert!(replacement
            .reconcile_unowned_until(
                &mut scan,
                &inventory,
                hard.metadata().unwrap(),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err());
        assert_eq!(&*replacement.get(b"orphan").unwrap(), b"replacement");
    }
}
