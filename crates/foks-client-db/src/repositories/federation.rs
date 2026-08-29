use rusqlite::{params, OptionalExtension as _};

use super::journals::team_mutation_from_connection;
use crate::*;

impl HardStateStore {
    /// Atomically reserves the local team-chain position and checkpoints the
    /// federation workflow that owns it. Exact retries are idempotent; any binding
    /// conflict rolls back both halves of the preparation.
    pub fn prepare_federation_local_mutation(
        &mut self,
        saga_id: &[u8; 16],
        operation: &TeamMutationOperation,
        updated_at: u64,
    ) -> Result<()> {
        validate_team_mutation(operation)?;
        if operation.kind != TeamMutationKind::MembershipChange
            || operation.state != TeamMutationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidFederationSaga(
                "local mutation must be a newly prepared membership change",
            ));
        }
        let transaction = self.write_transaction()?;
        let saga = federation_saga_from_connection(&transaction, saga_id)?
            .ok_or(Error::InvalidFederationSaga("saga is not recorded"))?;
        if !matches!(
            saga.state,
            FederationSagaState::RemoteVerified | FederationSagaState::LocalPrepared
        ) || operation.host_id != saga.local_host_id
            || operation.actor_id != saga.actor_id
            || operation.team_id != saga.local_team_id
            || updated_at < saga.created_at
            || updated_at < saga.updated_at
        {
            return Err(Error::InvalidFederationSaga(
                "local mutation does not match the remote-verified saga",
            ));
        }
        let checkpoint = (operation.expected_seqno, operation.operation_id);
        match (saga.expected_local_seqno, saga.local_mutation_id) {
            (None, None) if saga.state == FederationSagaState::RemoteVerified => {}
            (Some(sequence), Some(mutation))
                if saga.state == FederationSagaState::LocalPrepared
                    && (sequence, mutation) == checkpoint => {}
            _ => {
                return Err(Error::InvalidFederationSaga(
                    "local mutation checkpoint changed",
                ));
            }
        }

        transaction.execute(
            "INSERT INTO team_mutation_operations (
                operation_id, operation_kind, host_id, actor_id, device_id,
                team_id, expected_seqno, request_hash, state, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(operation_id) DO NOTHING",
            params![
                operation.operation_id.as_slice(),
                operation.kind as u8,
                operation.host_id,
                operation.actor_id,
                operation.device_id,
                operation.team_id,
                sqlite_integer("team mutation sequence", operation.expected_seqno)?,
                operation.request_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("team mutation created time", operation.created_at)?,
                sqlite_integer("team mutation updated time", operation.updated_at)?,
            ],
        )?;
        let stored = team_mutation_from_connection(&transaction, &operation.operation_id)?.ok_or(
            Error::InvalidFederationSaga("prepared local mutation disappeared"),
        )?;
        if stored.kind != operation.kind
            || stored.host_id != operation.host_id
            || stored.actor_id != operation.actor_id
            || stored.device_id != operation.device_id
            || stored.team_id != operation.team_id
            || stored.expected_seqno != operation.expected_seqno
            || stored.request_hash != operation.request_hash
            || matches!(
                stored.state,
                TeamMutationState::Rejected | TeamMutationState::Superseded
            )
        {
            return Err(Error::InvalidFederationSaga(
                "local mutation ID was reused for another binding",
            ));
        }
        transaction.execute(
            "UPDATE federation_saga_operations
             SET state = ?2, expected_local_seqno = ?3, local_mutation_id = ?4,
                 updated_at = ?5
             WHERE operation_id = ?1",
            params![
                saga_id.as_slice(),
                FederationSagaState::LocalPrepared as u8,
                sqlite_integer("federation local sequence", operation.expected_seqno)?,
                operation.operation_id.as_slice(),
                sqlite_integer("federation saga updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Records the first durable cross-host checkpoint. Repeating the exact
    /// operation is idempotent; reusing its ID for another binding fails.
    pub fn record_federation_saga(&mut self, operation: &FederationSagaOperation) -> Result<()> {
        validate_federation_saga(operation)?;
        if operation.state != FederationSagaState::PermissionGranted
            || operation.expected_local_seqno.is_some()
            || operation.local_mutation_id.is_some()
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidFederationSaga(
                "new saga must start at the permission-granted checkpoint",
            ));
        }
        let transaction = self.write_transaction()?;
        let host_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.local_host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        transaction.execute(
            "INSERT INTO federation_saga_operations (
                 operation_id, local_host_id, remote_host_id, actor_id,
                 local_team_id, remote_party_id, permission_hash,
                 destination_role_type, destination_visibility,
                 removal_key_commitment, state, expected_local_seqno,
                 local_mutation_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                       NULL, NULL, ?12, ?12)
             ON CONFLICT(operation_id) DO NOTHING",
            params![
                operation.operation_id.as_slice(),
                operation.local_host_id,
                operation.remote_host_id,
                operation.actor_id,
                operation.local_team_id,
                operation.remote_party_id,
                operation.permission_hash.as_slice(),
                sqlite_integer(
                    "federation destination role",
                    operation.destination_role_type
                )?,
                operation.destination_visibility,
                operation.removal_key_commitment.as_slice(),
                operation.state as u8,
                sqlite_integer("federation saga created time", operation.created_at)?,
            ],
        )?;
        let stored = federation_saga_from_connection(&transaction, &operation.operation_id)?
            .ok_or(Error::InvalidFederationSaga("recorded saga disappeared"))?;
        if !same_federation_binding(&stored, operation) {
            return Err(Error::InvalidFederationSaga(
                "operation ID was reused for another saga",
            ));
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn advance_federation_saga(
        &mut self,
        operation_id: &[u8; 16],
        next: FederationSagaState,
        updated_at: u64,
    ) -> Result<()> {
        if next == FederationSagaState::LocalPrepared {
            return Err(Error::InvalidFederationSaga(
                "local preparation must atomically reserve its team mutation",
            ));
        }
        let transaction = self.write_transaction()?;
        let current = federation_saga_from_connection(&transaction, operation_id)?
            .ok_or(Error::InvalidFederationSaga("saga is not recorded"))?;
        if !current.state.can_transition_to(next)
            || updated_at < current.updated_at
            || updated_at < current.created_at
        {
            return Err(Error::InvalidFederationSaga(
                "saga state transition is invalid",
            ));
        }
        let checkpoint = match (current.expected_local_seqno, current.local_mutation_id) {
            (Some(sequence), Some(mutation)) => Some((sequence, mutation)),
            (None, None) => None,
            _ => {
                return Err(Error::InvalidFederationSaga(
                    "stored local checkpoint is incomplete",
                ));
            }
        };
        let (sequence, mutation) = if let Some((sequence, mutation)) = checkpoint {
            (
                Some(sqlite_integer("federation local sequence", sequence)?),
                Some(mutation),
            )
        } else {
            (None, None)
        };
        transaction.execute(
            "UPDATE federation_saga_operations
             SET state = ?2, expected_local_seqno = ?3, local_mutation_id = ?4,
                 updated_at = ?5
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                next as u8,
                sequence,
                mutation.map(|value| value.to_vec()),
                sqlite_integer("federation saga updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn federation_saga(
        &self,
        operation_id: &[u8; 16],
    ) -> Result<Option<FederationSagaOperation>> {
        federation_saga_from_connection(&self.connection, operation_id)
    }

    pub fn pending_federation_sagas(
        &self,
        local_host_id: &[u8],
    ) -> Result<Vec<FederationSagaOperation>> {
        let mut statement = self.connection.prepare(
            "SELECT operation_id FROM federation_saga_operations
             WHERE local_host_id = ?1 AND state BETWEEN 1 AND 4
             ORDER BY created_at, operation_id",
        )?;
        let ids = statement
            .query_map([local_host_id], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                let id: [u8; 16] = id.try_into().map_err(|_| {
                    Error::InvalidFederationSaga("stored operation ID has the wrong length")
                })?;
                federation_saga_from_connection(&self.connection, &id)?.ok_or(
                    Error::InvalidFederationSaga("pending federation saga record not found"),
                )
            })
            .collect()
    }
}

fn federation_saga_from_connection(
    connection: &rusqlite::Connection,
    operation_id: &[u8; 16],
) -> Result<Option<FederationSagaOperation>> {
    connection
        .query_row(
            "SELECT local_host_id, remote_host_id, actor_id, local_team_id,
                    remote_party_id, permission_hash, destination_role_type,
                    destination_visibility, removal_key_commitment, state,
                    expected_local_seqno, local_mutation_id, created_at, updated_at
             FROM federation_saga_operations WHERE operation_id = ?1",
            [operation_id.as_slice()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<Vec<u8>>>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, i64>(13)?,
                ))
            },
        )
        .optional()?
        .map(|row| {
            Ok(FederationSagaOperation {
                operation_id: *operation_id,
                local_host_id: row.0,
                remote_host_id: row.1,
                actor_id: row.2,
                local_team_id: row.3,
                remote_party_id: row.4,
                permission_hash: row.5.try_into().map_err(|_| {
                    Error::InvalidFederationSaga("stored permission hash has the wrong length")
                })?,
                destination_role_type: stored_unsigned("federation destination role", row.6)?,
                destination_visibility: row.7,
                removal_key_commitment: row.8.try_into().map_err(|_| {
                    Error::InvalidFederationSaga(
                        "stored removal-key commitment has the wrong length",
                    )
                })?,
                state: FederationSagaState::from_sql(row.9)?,
                expected_local_seqno: row
                    .10
                    .map(|value| stored_unsigned("federation local sequence", value))
                    .transpose()?,
                local_mutation_id: row
                    .11
                    .map(|value| {
                        value.try_into().map_err(|_| {
                            Error::InvalidFederationSaga(
                                "stored local mutation ID has the wrong length",
                            )
                        })
                    })
                    .transpose()?,
                created_at: stored_unsigned("federation saga created time", row.12)?,
                updated_at: stored_unsigned("federation saga updated time", row.13)?,
            })
        })
        .transpose()
}

fn validate_federation_saga(operation: &FederationSagaOperation) -> Result<()> {
    let destination_is_valid = match operation.destination_role_type {
        1 => i16::try_from(operation.destination_visibility).is_ok(),
        2 | 3 => operation.destination_visibility == 0,
        _ => false,
    };
    if !has_entity_type(&operation.local_host_id, &[foks_proto::ENTITY_HOST])
        || !has_entity_type(&operation.remote_host_id, &[foks_proto::ENTITY_HOST])
        || operation.local_host_id == operation.remote_host_id
        || !has_entity_type(&operation.actor_id, &[foks_proto::ENTITY_USER])
        || !has_entity_type(&operation.local_team_id, &[foks_proto::ENTITY_NAMED_TEAM])
        || !has_entity_type(
            &operation.remote_party_id,
            &[
                foks_proto::ENTITY_NAMED_TEAM,
                foks_proto::ENTITY_AD_HOC_TEAM,
            ],
        )
        || !destination_is_valid
        || operation.created_at > operation.updated_at
        || operation.expected_local_seqno.is_some() != operation.local_mutation_id.is_some()
    {
        return Err(Error::InvalidFederationSaga("saga binding is malformed"));
    }
    Ok(())
}

fn has_entity_type(value: &[u8], allowed: &[u8]) -> bool {
    value.len() == 33 && allowed.contains(&value[0])
}

fn same_federation_binding(
    left: &FederationSagaOperation,
    right: &FederationSagaOperation,
) -> bool {
    left.operation_id == right.operation_id
        && left.local_host_id == right.local_host_id
        && left.remote_host_id == right.remote_host_id
        && left.actor_id == right.actor_id
        && left.local_team_id == right.local_team_id
        && left.remote_party_id == right.remote_party_id
        && left.permission_hash == right.permission_hash
        && left.destination_role_type == right.destination_role_type
        && left.destination_visibility == right.destination_visibility
        && left.removal_key_commitment == right.removal_key_commitment
}
