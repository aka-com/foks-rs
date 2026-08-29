//! Crash-safe mutation coordination across public SQLite state and protected
//! retry material.

use std::path::Path;

use foks_client_db::{HardStateStore, MutationKind, MutationOperation, MutationState};
use foks_crypto::prefixed_hash;
use zeroize::Zeroizing;

use crate::{now_microseconds, Error, Result};

const MATERIAL_HASH_TYPE_ID: u64 = 0x730c_cae1_c80d_4ec2;

/// Failure returned by a protected credential/material store.
#[derive(Debug, thiserror::Error)]
pub enum ProtectedStoreError {
    #[error("protected material already exists with different bytes")]
    Conflict,
    #[error("protected material is absent")]
    Missing,
    #[error("protected material store failed: {0}")]
    Backend(String),
}

/// Boundary to an encrypted credential store, platform keychain, or hardware
/// backed secret store. Implementations must durably commit `put_if_absent`
/// before returning and must never silently replace different bytes at a key.
pub trait ProtectedMutationStore {
    fn put_if_absent(
        &mut self,
        key: &[u8],
        material: &[u8],
    ) -> std::result::Result<(), ProtectedStoreError>;
    fn get(&mut self, key: &[u8]) -> std::result::Result<Zeroizing<Vec<u8>>, ProtectedStoreError>;
    fn remove(&mut self, key: &[u8]) -> std::result::Result<(), ProtectedStoreError>;
}

/// Public binding fields for a new mutation. The coordinator computes and
/// persists the protected-material fingerprint and initial timestamps.
pub struct MutationDraft {
    pub operation_id: [u8; 16],
    pub kind: MutationKind,
    pub host_id: Vec<u8>,
    pub scope_id: Vec<u8>,
    pub subject_id: Vec<u8>,
    pub expected_version: Option<u64>,
    pub request_hash: [u8; 32],
}

/// Coordinates the required ordering between two durability domains:
/// protected material first, then SQLite WAL. Remote verification remains
/// nonterminal until the application acknowledges its own durable commit;
/// terminal SQLite state is committed before protected material is erased.
pub struct MutationCoordinator<'a, S: ProtectedMutationStore + ?Sized> {
    hard_database: &'a Path,
    protected: &'a mut S,
}

impl<'a, S: ProtectedMutationStore + ?Sized> MutationCoordinator<'a, S> {
    pub fn new(hard_database: &'a Path, protected: &'a mut S) -> Self {
        Self {
            hard_database,
            protected,
        }
    }

    pub fn prepare(
        &mut self,
        draft: MutationDraft,
        material: Zeroizing<Vec<u8>>,
    ) -> Result<MutationOperation> {
        let material_ref = draft.operation_id.to_vec();
        self.protected
            .put_if_absent(&material_ref, &material)
            .map_err(material_error)?;
        let now = now_microseconds()?;
        let operation = MutationOperation {
            operation_id: draft.operation_id,
            kind: draft.kind,
            host_id: draft.host_id,
            scope_id: draft.scope_id,
            subject_id: draft.subject_id,
            expected_version: draft.expected_version,
            request_hash: draft.request_hash,
            material_ref,
            material_hash: prefixed_hash(MATERIAL_HASH_TYPE_ID, &material),
            state: MutationState::Prepared,
            attempt_count: 0,
            created_at: now,
            updated_at: now,
        };
        HardStateStore::open(self.hard_database)?.record_mutation(&operation)?;
        Ok(operation)
    }

    pub fn begin_submission(&mut self, operation_id: &[u8; 16]) -> Result<()> {
        // Prove the protected record is still present and bound before the WAL
        // crosses the no-replay boundary.
        let operation = self.operation(operation_id)?;
        self.load_bound_material(&operation)?;
        HardStateStore::open(self.hard_database)?
            .begin_mutation_submission(operation_id, now_microseconds()?)?;
        Ok(())
    }

    pub fn submission_unknown(&mut self, operation_id: &[u8; 16]) -> Result<()> {
        HardStateStore::open(self.hard_database)?.advance_mutation(
            operation_id,
            MutationState::SubmissionUnknown,
            now_microseconds()?,
        )?;
        Ok(())
    }

    pub fn remote_verified(&mut self, operation_id: &[u8; 16]) -> Result<()> {
        let operation = self.operation(operation_id)?;
        self.load_bound_material(&operation)?;
        HardStateStore::open(self.hard_database)?.advance_mutation(
            operation_id,
            MutationState::RemoteVerified,
            now_microseconds()?,
        )?;
        Ok(())
    }

    /// Acknowledges that the consumer of a remotely verified mutation has
    /// durably committed its application-owned state. This is the only
    /// successful path that erases protected mutation material.
    pub fn finalize(&mut self, operation_id: &[u8; 16]) -> Result<()> {
        let operation = self.operation(operation_id)?;
        HardStateStore::open(self.hard_database)?.advance_mutation(
            operation_id,
            MutationState::Finalized,
            now_microseconds()?,
        )?;
        // A crash or backend failure here leaves only an orphaned protected
        // record. The authoritative journal is already terminal.
        remove_terminal_material(self.protected, &operation.material_ref)
    }

    pub fn remote_verified_and_finalize(&mut self, operation_id: &[u8; 16]) -> Result<()> {
        self.remote_verified(operation_id)?;
        self.finalize(operation_id)
    }

    pub fn rejected(&mut self, operation_id: &[u8; 16]) -> Result<()> {
        let operation = self.operation(operation_id)?;
        HardStateStore::open(self.hard_database)?.advance_mutation(
            operation_id,
            MutationState::Rejected,
            now_microseconds()?,
        )?;
        remove_terminal_material(self.protected, &operation.material_ref)
    }

    pub fn pending(&mut self, host_id: &[u8]) -> Result<Vec<MutationOperation>> {
        HardStateStore::open(self.hard_database)?
            .pending_mutations(host_id)
            .map_err(Error::from)
    }

    pub fn load_bound_material(
        &mut self,
        operation: &MutationOperation,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let material = self
            .protected
            .get(&operation.material_ref)
            .map_err(material_error)?;
        if prefixed_hash(MATERIAL_HASH_TYPE_ID, &material) != operation.material_hash {
            return Err(Error::OperationBinding(
                "protected mutation material fingerprint changed",
            ));
        }
        Ok(material)
    }

    fn operation(&self, operation_id: &[u8; 16]) -> Result<MutationOperation> {
        HardStateStore::open(self.hard_database)?
            .mutation(operation_id)?
            .ok_or(Error::OperationBinding(
                "mutation operation is not recorded",
            ))
    }
}

fn material_error(error: ProtectedStoreError) -> Error {
    Error::ProtectedMaterial(error.to_string())
}

fn remove_terminal_material<S: ProtectedMutationStore + ?Sized>(
    store: &mut S,
    key: &[u8],
) -> Result<()> {
    match store.remove(key) {
        Ok(()) | Err(ProtectedStoreError::Missing) => Ok(()),
        Err(error) => Err(material_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use foks_client_db::{Acceptance, MutationKind, MutationState};

    use super::*;
    use foks_verify::verify_public_host;

    #[derive(Default)]
    struct MemoryProtectedStore(BTreeMap<Vec<u8>, Vec<u8>>);

    impl ProtectedMutationStore for MemoryProtectedStore {
        fn put_if_absent(
            &mut self,
            key: &[u8],
            material: &[u8],
        ) -> std::result::Result<(), ProtectedStoreError> {
            match self.0.get(key) {
                Some(existing) if existing != material => Err(ProtectedStoreError::Conflict),
                Some(_) => Ok(()),
                None => {
                    self.0.insert(key.to_vec(), material.to_vec());
                    Ok(())
                }
            }
        }

        fn get(
            &mut self,
            key: &[u8],
        ) -> std::result::Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
            self.0
                .get(key)
                .cloned()
                .map(Zeroizing::new)
                .ok_or(ProtectedStoreError::Missing)
        }

        fn remove(&mut self, key: &[u8]) -> std::result::Result<(), ProtectedStoreError> {
            self.0
                .remove(key)
                .map(|_| ())
                .ok_or(ProtectedStoreError::Missing)
        }
    }

    struct FailRemoveOnceStore {
        inner: MemoryProtectedStore,
        fail_remove: bool,
    }

    impl ProtectedMutationStore for FailRemoveOnceStore {
        fn put_if_absent(
            &mut self,
            key: &[u8],
            material: &[u8],
        ) -> std::result::Result<(), ProtectedStoreError> {
            self.inner.put_if_absent(key, material)
        }

        fn get(
            &mut self,
            key: &[u8],
        ) -> std::result::Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
            self.inner.get(key)
        }

        fn remove(&mut self, key: &[u8]) -> std::result::Result<(), ProtectedStoreError> {
            if self.fail_remove {
                self.fail_remove = false;
                return Err(ProtectedStoreError::Backend(
                    "injected cleanup interruption".to_owned(),
                ));
            }
            self.inner.remove(key)
        }
    }

    #[derive(Default)]
    struct AuthoritativeMutationEndpoint {
        accepted: BTreeMap<[u8; 16], [u8; 32]>,
        submissions: usize,
    }

    impl AuthoritativeMutationEndpoint {
        fn accept(&mut self, operation_id: [u8; 16], request_hash: [u8; 32]) {
            self.submissions += 1;
            match self.accepted.get(&operation_id) {
                Some(existing) => assert_eq!(*existing, request_hash),
                None => {
                    self.accepted.insert(operation_id, request_hash);
                }
            }
        }

        fn observed(&self, operation: &MutationOperation) -> bool {
            self.accepted.get(&operation.operation_id) == Some(&operation.request_hash)
        }
    }

    /// Deterministically models the transport's hardest ambiguity: the
    /// authoritative endpoint commits the request, then the response is lost.
    struct LostResponseProxy<'a> {
        endpoint: &'a mut AuthoritativeMutationEndpoint,
    }

    impl LostResponseProxy<'_> {
        fn submit(
            &mut self,
            operation_id: [u8; 16],
            request_hash: [u8; 32],
        ) -> std::result::Result<(), &'static str> {
            self.endpoint.accept(operation_id, request_hash);
            Err("response lost after authoritative acceptance")
        }
    }

    fn initialized_database() -> (tempfile::TempDir, PathBuf, Vec<u8>) {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("hard.sqlite3");
        let snapshot = verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap()
        .snapshot;
        assert_eq!(
            HardStateStore::open(&database)
                .unwrap()
                .accept_verified_host(&snapshot)
                .unwrap(),
            Acceptance::Inserted
        );
        (temporary, database, snapshot.host_id().to_vec())
    }

    fn draft(operation_id: [u8; 16], host_id: &[u8]) -> MutationDraft {
        MutationDraft {
            operation_id,
            kind: MutationKind::DeviceProvision,
            host_id: host_id.to_vec(),
            scope_id: vec![2; 33],
            subject_id: vec![3; 33],
            expected_version: Some(4),
            request_hash: [5; 32],
        }
    }

    fn reconcile_without_replay(
        database: &Path,
        protected: &mut MemoryProtectedStore,
        endpoint: &AuthoritativeMutationEndpoint,
        operation_id: [u8; 16],
    ) -> bool {
        let operation = HardStateStore::open(database)
            .unwrap()
            .mutation(&operation_id)
            .unwrap()
            .unwrap();
        assert!(matches!(
            operation.state,
            MutationState::Submitting | MutationState::SubmissionUnknown
        ));
        if endpoint.observed(&operation) {
            MutationCoordinator::new(database, protected)
                .remote_verified(&operation_id)
                .unwrap();
            true
        } else {
            false
        }
    }

    #[test]
    fn protected_material_precedes_wal_and_survives_unknown_response() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("hard.sqlite3");
        let snapshot = verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap()
        .snapshot;
        assert_eq!(
            HardStateStore::open(&database)
                .unwrap()
                .accept_verified_host(&snapshot)
                .unwrap(),
            Acceptance::Inserted
        );
        let host_id = snapshot.host_id().to_vec();
        let mut protected = MemoryProtectedStore::default();
        let operation = {
            let mut coordinator = MutationCoordinator::new(&database, &mut protected);
            let operation = coordinator
                .prepare(
                    MutationDraft {
                        operation_id: [7; 16],
                        kind: MutationKind::DeviceRevoke,
                        host_id: host_id.clone(),
                        scope_id: vec![2; 33],
                        subject_id: vec![3; 33],
                        expected_version: Some(4),
                        request_hash: [5; 32],
                    },
                    Zeroizing::new(b"encrypted retry material".to_vec()),
                )
                .unwrap();
            coordinator
                .begin_submission(&operation.operation_id)
                .unwrap();
            coordinator
                .submission_unknown(&operation.operation_id)
                .unwrap();
            operation
        };
        let mut restarted = MutationCoordinator::new(&database, &mut protected);
        let pending = restarted.pending(&host_id).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].state, MutationState::SubmissionUnknown);
        assert_eq!(
            restarted
                .load_bound_material(&pending[0])
                .unwrap()
                .as_slice(),
            b"encrypted retry material"
        );
        assert!(HardStateStore::open(&database)
            .unwrap()
            .begin_mutation_submission(&operation.operation_id, u64::MAX / 2)
            .is_err());
        restarted.remote_verified(&operation.operation_id).unwrap();
        restarted.remote_verified(&operation.operation_id).unwrap();
        let awaiting = restarted.pending(&host_id).unwrap();
        assert_eq!(awaiting.len(), 1);
        assert_eq!(awaiting[0].state, MutationState::RemoteVerified);
        assert_eq!(
            restarted
                .load_bound_material(&awaiting[0])
                .unwrap()
                .as_slice(),
            b"encrypted retry material"
        );
        restarted.finalize(&operation.operation_id).unwrap();
        restarted.finalize(&operation.operation_id).unwrap();
        assert!(restarted.pending(&host_id).unwrap().is_empty());
    }

    #[test]
    fn protected_material_tampering_is_detected_before_submission() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("hard.sqlite3");
        let snapshot = verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap()
        .snapshot;
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&snapshot)
            .unwrap();
        let mut protected = MemoryProtectedStore::default();
        let operation = {
            let mut coordinator = MutationCoordinator::new(&database, &mut protected);
            coordinator
                .prepare(
                    MutationDraft {
                        operation_id: [8; 16],
                        kind: MutationKind::PukRotation,
                        host_id: snapshot.host_id().to_vec(),
                        scope_id: vec![2; 33],
                        subject_id: Vec::new(),
                        expected_version: Some(8),
                        request_hash: [9; 32],
                    },
                    Zeroizing::new(b"original".to_vec()),
                )
                .unwrap()
        };
        protected
            .0
            .insert(operation.material_ref.clone(), b"tampered".to_vec());
        let mut coordinator = MutationCoordinator::new(&database, &mut protected);
        assert!(matches!(
            coordinator.begin_submission(&operation.operation_id),
            Err(Error::OperationBinding(_))
        ));
        assert_eq!(
            HardStateStore::open(&database)
                .unwrap()
                .mutation(&operation.operation_id)
                .unwrap()
                .unwrap()
                .state,
            MutationState::Prepared
        );
    }

    #[test]
    fn lost_response_and_restart_crash_matrix_never_replays_ambiguous_mutations() {
        // Crash after protected material but before the SQLite WAL: the only
        // residue is an unreachable protected-store record, never a request
        // eligible for submission.
        {
            let (_temporary, database, _host_id) = initialized_database();
            let mut protected = MemoryProtectedStore::default();
            protected
                .put_if_absent(&[10; 16], b"orphaned material")
                .unwrap();
            assert!(HardStateStore::open(&database)
                .unwrap()
                .mutation(&[10; 16])
                .unwrap()
                .is_none());
        }

        // Crash after Prepared: this is the sole durable state that permits
        // one submission. The proxy commits it and loses the response; the
        // restarted coordinator reconciles rather than replaying it.
        {
            let (_temporary, database, host_id) = initialized_database();
            let mut protected = MemoryProtectedStore::default();
            let operation = MutationCoordinator::new(&database, &mut protected)
                .prepare(
                    draft([11; 16], &host_id),
                    Zeroizing::new(b"prepared request".to_vec()),
                )
                .unwrap();
            let mut endpoint = AuthoritativeMutationEndpoint::default();
            MutationCoordinator::new(&database, &mut protected)
                .begin_submission(&operation.operation_id)
                .unwrap();
            assert!(LostResponseProxy {
                endpoint: &mut endpoint,
            }
            .submit(operation.operation_id, operation.request_hash)
            .is_err());
            MutationCoordinator::new(&database, &mut protected)
                .submission_unknown(&operation.operation_id)
                .unwrap();
            assert!(reconcile_without_replay(
                &database,
                &mut protected,
                &endpoint,
                operation.operation_id,
            ));
            assert_eq!(endpoint.submissions, 1);
        }

        // Crash after crossing Prepared -> Submitting but before the send: a
        // restart cannot distinguish this from a lost response. It therefore
        // does not replay, and remains pending until authoritative evidence is
        // available or an operator rejects it.
        {
            let (_temporary, database, host_id) = initialized_database();
            let mut protected = MemoryProtectedStore::default();
            let operation = MutationCoordinator::new(&database, &mut protected)
                .prepare(
                    draft([12; 16], &host_id),
                    Zeroizing::new(b"not sent".to_vec()),
                )
                .unwrap();
            MutationCoordinator::new(&database, &mut protected)
                .begin_submission(&operation.operation_id)
                .unwrap();
            let endpoint = AuthoritativeMutationEndpoint::default();
            assert!(!reconcile_without_replay(
                &database,
                &mut protected,
                &endpoint,
                operation.operation_id,
            ));
            assert_eq!(endpoint.submissions, 0);
            assert!(HardStateStore::open(&database)
                .unwrap()
                .begin_mutation_submission(&operation.operation_id, u64::MAX / 2)
                .is_err());
        }

        // Crash after server acceptance but before either response handling or
        // SubmissionUnknown persistence leaves Submitting. Authenticated
        // observation still completes it without a second send.
        {
            let (_temporary, database, host_id) = initialized_database();
            let mut protected = MemoryProtectedStore::default();
            let operation = MutationCoordinator::new(&database, &mut protected)
                .prepare(
                    draft([13; 16], &host_id),
                    Zeroizing::new(b"accepted request".to_vec()),
                )
                .unwrap();
            MutationCoordinator::new(&database, &mut protected)
                .begin_submission(&operation.operation_id)
                .unwrap();
            let mut endpoint = AuthoritativeMutationEndpoint::default();
            endpoint.accept(operation.operation_id, operation.request_hash);
            assert!(reconcile_without_replay(
                &database,
                &mut protected,
                &endpoint,
                operation.operation_id,
            ));
            assert_eq!(endpoint.submissions, 1);
        }

        // Remote verification retains the protected record. A crash or
        // key-store failure after the application-acknowledged terminal
        // SQLite commit leaves an orphan that repeating finalization cleans.
        {
            let (_temporary, database, host_id) = initialized_database();
            let mut protected = FailRemoveOnceStore {
                inner: MemoryProtectedStore::default(),
                fail_remove: true,
            };
            let operation = MutationCoordinator::new(&database, &mut protected)
                .prepare(
                    draft([14; 16], &host_id),
                    Zeroizing::new(b"terminal request".to_vec()),
                )
                .unwrap();
            MutationCoordinator::new(&database, &mut protected)
                .begin_submission(&operation.operation_id)
                .unwrap();
            MutationCoordinator::new(&database, &mut protected)
                .remote_verified(&operation.operation_id)
                .unwrap();
            assert!(!protected.inner.0.is_empty());
            assert!(MutationCoordinator::new(&database, &mut protected)
                .finalize(&operation.operation_id)
                .is_err());
            assert_eq!(
                HardStateStore::open(&database)
                    .unwrap()
                    .mutation(&operation.operation_id)
                    .unwrap()
                    .unwrap()
                    .state,
                MutationState::Finalized
            );
            MutationCoordinator::new(&database, &mut protected)
                .finalize(&operation.operation_id)
                .unwrap();
            assert!(protected.inner.0.is_empty());
        }
    }
}
