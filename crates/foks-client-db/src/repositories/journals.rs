use rusqlite::{params, OptionalExtension};

use crate::*;

impl HardStateStore {
    /// Durably records a mutation before any network submission. Protected
    /// material identified by `material_ref` must already be committed by the
    /// caller; an orphaned material record is safe, while a journal row with
    /// missing material is not recoverable.
    pub fn record_mutation(&mut self, operation: &MutationOperation) -> Result<()> {
        validate_mutation_operation(operation)?;
        if operation.state != MutationState::Prepared
            || operation.attempt_count != 0
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidMutationOperation(
                "new operation must be unattempted and prepared",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO mutation_operations (
                operation_id, operation_kind, host_id, scope_id, subject_id,
                expected_version, request_hash, material_ref, material_hash,
                state, attempt_count, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                operation.operation_id.as_slice(),
                operation.kind as u8,
                operation.host_id,
                operation.scope_id,
                operation.subject_id,
                operation
                    .expected_version
                    .map(|value| sqlite_integer("mutation expected version", value))
                    .transpose()?,
                operation.request_hash.as_slice(),
                operation.material_ref,
                operation.material_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("mutation attempt count", operation.attempt_count)?,
                sqlite_integer("mutation created time", operation.created_at)?,
                sqlite_integer("mutation updated time", operation.updated_at)?,
            ],
        )?;
        Ok(())
    }

    /// Atomically marks the one and only initial submission attempt. Once this
    /// commits, a crash is treated as an ambiguous response and recovery must
    /// inspect authenticated server state rather than reposting the request.
    pub fn begin_mutation_submission(
        &mut self,
        operation_id: &[u8; 16],
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self.write_transaction()?;
        let (state, created_at, previous_updated_at, attempts) = transaction
            .query_row(
                "SELECT state, created_at, updated_at, attempt_count
                 FROM mutation_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidMutationOperation("operation is not recorded"))?;
        let created_at = stored_unsigned("mutation created time", created_at)?;
        let previous_updated_at = stored_unsigned("mutation updated time", previous_updated_at)?;
        if MutationState::from_sql(state)? != MutationState::Prepared || attempts != 0 {
            return Err(Error::InvalidMutationOperation(
                "mutation cannot be submitted more than once",
            ));
        }
        // Clamp the local journal timestamp monotonically so a backward wall-clock
        // step cannot block the one-time submission. The no-replay boundary is the
        // state/attempt guard above, not the timestamp.
        let stamped = monotonic_timestamp(created_at, previous_updated_at, updated_at);
        transaction.execute(
            "UPDATE mutation_operations
             SET state = ?2, attempt_count = 1, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                MutationState::Submitting as u8,
                sqlite_integer("mutation updated time", stamped)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn advance_mutation(
        &mut self,
        operation_id: &[u8; 16],
        state: MutationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self.write_transaction()?;
        let (current, created_at, previous_updated_at) = transaction
            .query_row(
                "SELECT state, created_at, updated_at
                 FROM mutation_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidMutationOperation("operation is not recorded"))?;
        let current = MutationState::from_sql(current)?;
        let created_at = stored_unsigned("mutation created time", created_at)?;
        let previous_updated_at = stored_unsigned("mutation updated time", previous_updated_at)?;
        if !current.can_transition_to(state) {
            return Err(Error::InvalidMutationOperation(
                "operation state transition is invalid",
            ));
        }
        // Clamp monotonically so a backward wall-clock step cannot block
        // finalization/verification. Ordering (state machine) is enforced by
        // can_transition_to above, not by the timestamp.
        let stamped = monotonic_timestamp(created_at, previous_updated_at, updated_at);
        transaction.execute(
            "UPDATE mutation_operations SET state = ?2, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("mutation updated time", stamped)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn mutation(&self, operation_id: &[u8; 16]) -> Result<Option<MutationOperation>> {
        self.connection
            .query_row(
                "SELECT operation_kind, host_id, scope_id, subject_id,
                        expected_version, request_hash, material_ref, material_hash, state,
                        attempt_count, created_at, updated_at
                 FROM mutation_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| mutation_operation_from_row(*operation_id, row),
            )
            .optional()?
            .map(Ok)
            .transpose()
    }

    /// Returns all nonterminal operations in deterministic creation order for
    /// startup reconciliation.
    pub fn pending_mutations(&self, host_id: &[u8]) -> Result<Vec<MutationOperation>> {
        let mut statement = self.connection.prepare(
            "SELECT operation_id, operation_kind, host_id, scope_id, subject_id,
                    expected_version, request_hash, material_ref, material_hash, state,
                    attempt_count, created_at, updated_at
             FROM mutation_operations
             WHERE host_id = ?1 AND state IN (1, 2, 3, 4)
             ORDER BY created_at, operation_id",
        )?;
        let rows = statement.query_map([host_id], |row| {
            let operation_id = row.get::<_, Vec<u8>>(0)?;
            let operation_id: [u8; 16] = operation_id.try_into().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    16,
                    rusqlite::types::Type::Blob,
                    "invalid mutation operation ID".into(),
                )
            })?;
            mutation_operation_from_offset(operation_id, row, 1)
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// Finds the newest operation for an application-owned pending record,
    /// including final state. Applications use `RemoteVerified` to close the
    /// durability boundary between core verification and committing their
    /// final credential record, and may revisit `Finalized` to clean an orphan.
    pub fn latest_mutation_for_binding(
        &self,
        host_id: &[u8],
        kind: MutationKind,
        scope_id: &[u8],
        subject_id: &[u8],
    ) -> Result<Option<MutationOperation>> {
        self.connection
            .query_row(
                "SELECT operation_id, operation_kind, host_id, scope_id, subject_id,
                        expected_version, request_hash, material_ref, material_hash, state,
                        attempt_count, created_at, updated_at
                 FROM mutation_operations
                 WHERE host_id = ?1 AND operation_kind = ?2
                   AND scope_id = ?3 AND subject_id = ?4
                 ORDER BY created_at DESC, operation_id DESC LIMIT 1",
                params![host_id, kind as u8, scope_id, subject_id],
                |row| {
                    let operation_id = row.get::<_, Vec<u8>>(0)?;
                    let operation_id: [u8; 16] = operation_id.try_into().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            16,
                            rusqlite::types::Type::Blob,
                            "invalid mutation operation ID".into(),
                        )
                    })?;
                    mutation_operation_from_offset(operation_id, row, 1)
                },
            )
            .optional()?
            .map(Ok)
            .transpose()
    }

    /// Finds the newest mutation that has crossed the authenticated remote
    /// verification boundary and is therefore safe for application-owned
    /// protected-record cleanup. Pending operations remain visible through
    /// `latest_mutation_for_binding` for submission recovery.
    pub fn latest_finalizable_mutation_for_binding(
        &self,
        host_id: &[u8],
        kind: MutationKind,
        scope_id: &[u8],
        subject_id: &[u8],
    ) -> Result<Option<MutationOperation>> {
        self.connection
            .query_row(
                "SELECT operation_id, operation_kind, host_id, scope_id, subject_id,
                        expected_version, request_hash, material_ref, material_hash, state,
                        attempt_count, created_at, updated_at
                 FROM mutation_operations
                 WHERE host_id = ?1 AND operation_kind = ?2
                   AND scope_id = ?3 AND subject_id = ?4
                   AND state IN (?5, ?6)
                 ORDER BY created_at DESC, operation_id DESC LIMIT 1",
                params![
                    host_id,
                    kind as u8,
                    scope_id,
                    subject_id,
                    MutationState::RemoteVerified as u8,
                    MutationState::Finalized as u8
                ],
                |row| {
                    let operation_id = row.get::<_, Vec<u8>>(0)?;
                    let operation_id: [u8; 16] = operation_id.try_into().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            16,
                            rusqlite::types::Type::Blob,
                            "invalid mutation operation ID".into(),
                        )
                    })?;
                    mutation_operation_from_offset(operation_id, row, 1)
                },
            )
            .optional()?
            .map(Ok)
            .transpose()
    }
    /// Records only the public fingerprint of a prepared signup. The caller's
    /// encrypted credential store remains authoritative for retry material.
    pub fn record_signup_operation(&mut self, operation: &SignupOperation) -> Result<()> {
        validate_signup_operation(operation)?;
        if operation.state != SignupOperationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidSignupOperation(
                "new operation must be in the prepared state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO signup_operations (
                operation_id, host_id, normalized_username, uid, device_id,
                request_hash, state, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation.operation_id.as_slice(),
                operation.host_id,
                operation.normalized_username,
                operation.uid,
                operation.device_id,
                operation.request_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("signup created time", operation.created_at)?,
                sqlite_integer("signup updated time", operation.updated_at)?,
            ],
        )?;
        Ok(())
    }

    pub fn advance_signup_operation(
        &mut self,
        operation_id: &[u8; 16],
        state: SignupOperationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self.write_transaction()?;
        let current = transaction
            .query_row(
                "SELECT state, created_at, updated_at
                 FROM signup_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidSignupOperation("operation is not recorded"))?;
        let current_state = SignupOperationState::from_sql(current.0)?;
        let created_at = stored_unsigned("signup created time", current.1)?;
        let previous_updated_at = stored_unsigned("signup updated time", current.2)?;
        if updated_at < created_at
            || updated_at < previous_updated_at
            || (state as u8) < current_state as u8
            || (state as u8) > (current_state as u8).saturating_add(1)
        {
            return Err(Error::InvalidSignupOperation(
                "operation state transition is not monotonic",
            ));
        }
        transaction.execute(
            "UPDATE signup_operations SET state = ?2, updated_at = ?3 WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("signup updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn signup_operation(&self, operation_id: &[u8; 16]) -> Result<Option<SignupOperation>> {
        self.connection
            .query_row(
                "SELECT host_id, normalized_username, uid, device_id, request_hash,
                        state, created_at, updated_at
                 FROM signup_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(SignupOperation {
                    operation_id: *operation_id,
                    host_id: row.0,
                    normalized_username: row.1,
                    uid: row.2,
                    device_id: row.3,
                    request_hash: row.4.try_into().map_err(|_| {
                        Error::InvalidSignupOperation("stored request hash has the wrong length")
                    })?,
                    state: SignupOperationState::from_sql(row.5)?,
                    created_at: stored_unsigned("signup created time", row.6)?,
                    updated_at: stored_unsigned("signup updated time", row.7)?,
                })
            })
            .transpose()
    }

    /// Locates an interrupted signup from public identities that can be
    /// re-derived from caller-retained device and PUK seeds. Nonterminal rows
    /// are preferred so a process can recover even when its randomly generated
    /// operation ID was never returned to the caller.
    pub fn signup_operation_for_credential(
        &self,
        host_id: &[u8],
        uid: &[u8],
        device_id: &[u8],
    ) -> Result<Option<SignupOperation>> {
        let operation_id = self
            .connection
            .query_row(
                "SELECT operation_id FROM signup_operations
                 WHERE host_id = ?1 AND uid = ?2 AND device_id = ?3
                 ORDER BY CASE WHEN state = 3 THEN 1 ELSE 0 END,
                          updated_at DESC, operation_id DESC
                 LIMIT 1",
                rusqlite::params![host_id, uid, device_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|bytes| {
                bytes.try_into().map_err(|_| {
                    Error::InvalidSignupOperation("stored operation ID has the wrong length")
                })
            })
            .transpose()?;
        operation_id
            .as_ref()
            .map(|operation_id| self.signup_operation(operation_id))
            .transpose()
            .map(Option::flatten)
    }

    /// Records the public identity and request fingerprint for a prepared
    /// ad-hoc team creation. PTK seeds are deliberately caller-owned.
    pub fn record_adhoc_team_operation(&mut self, operation: &AdHocTeamOperation) -> Result<()> {
        validate_adhoc_team_operation(operation)?;
        if operation.state != AdHocTeamOperationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidAdHocTeamOperation(
                "new operation must be in the prepared state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO adhoc_team_operations (
                operation_id, host_id, uid, device_id, team_id, request_hash,
                state, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation.operation_id.as_slice(),
                operation.host_id,
                operation.uid,
                operation.device_id,
                operation.team_id,
                operation.request_hash.as_slice(),
                operation.state as u8,
                sqlite_integer("ad-hoc team created time", operation.created_at)?,
                sqlite_integer("ad-hoc team updated time", operation.updated_at)?,
            ],
        )?;
        Ok(())
    }

    pub fn advance_adhoc_team_operation(
        &mut self,
        operation_id: &[u8; 16],
        state: AdHocTeamOperationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self.write_transaction()?;
        let current = transaction
            .query_row(
                "SELECT state, created_at, updated_at
                 FROM adhoc_team_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidAdHocTeamOperation(
                "operation is not recorded",
            ))?;
        let current_state = AdHocTeamOperationState::from_sql(current.0)?;
        let created_at = stored_unsigned("ad-hoc team created time", current.1)?;
        let previous_updated_at = stored_unsigned("ad-hoc team updated time", current.2)?;
        if updated_at < created_at
            || updated_at < previous_updated_at
            || (state as u8) < current_state as u8
            || (state as u8) > (current_state as u8).saturating_add(1)
        {
            return Err(Error::InvalidAdHocTeamOperation(
                "operation state transition is not monotonic",
            ));
        }
        transaction.execute(
            "UPDATE adhoc_team_operations SET state = ?2, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("ad-hoc team updated time", updated_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn adhoc_team_operation(
        &self,
        operation_id: &[u8; 16],
    ) -> Result<Option<AdHocTeamOperation>> {
        self.connection
            .query_row(
                "SELECT host_id, uid, device_id, team_id, request_hash,
                        state, created_at, updated_at
                 FROM adhoc_team_operations WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(AdHocTeamOperation {
                    operation_id: *operation_id,
                    host_id: row.0,
                    uid: row.1,
                    device_id: row.2,
                    team_id: row.3,
                    request_hash: row.4.try_into().map_err(|_| {
                        Error::InvalidAdHocTeamOperation("stored request hash has the wrong length")
                    })?,
                    state: AdHocTeamOperationState::from_sql(row.5)?,
                    created_at: stored_unsigned("ad-hoc team created time", row.6)?,
                    updated_at: stored_unsigned("ad-hoc team updated time", row.7)?,
                })
            })
            .transpose()
    }

    pub fn record_team_mutation(&mut self, operation: &TeamMutationOperation) -> Result<()> {
        validate_team_mutation(operation)?;
        if operation.state != TeamMutationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidTeamMutation(
                "new operation must be in the prepared state",
            ));
        }
        let host_exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE host_id = ?1)",
            [&operation.host_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !host_exists {
            return Err(Error::UnknownHost);
        }
        self.connection.execute(
            "INSERT INTO team_mutation_operations (
                operation_id, operation_kind, host_id, actor_id, device_id,
                team_id, expected_seqno, request_hash, state, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
        Ok(())
    }

    /// Records a team mutation and marks it submitting in one transaction so a
    /// crash cannot leave the chain position reserved in `Prepared`.
    pub fn record_and_begin_team_mutation(
        &mut self,
        operation: &TeamMutationOperation,
        submitting_at: u64,
    ) -> Result<()> {
        validate_team_mutation(operation)?;
        if operation.state != TeamMutationState::Prepared
            || operation.created_at != operation.updated_at
        {
            return Err(Error::InvalidTeamMutation(
                "new operation must be in the prepared state",
            ));
        }
        let stamped =
            monotonic_timestamp(operation.created_at, operation.updated_at, submitting_at);
        let transaction = self.write_transaction()?;
        let occupant: Option<(Vec<u8>, i64)> = transaction
            .query_row(
                "SELECT operation_id, state FROM team_mutation_operations
                 WHERE host_id = ?1 AND team_id = ?2 AND expected_seqno = ?3
                   AND state IN (1, 2, 3, 4, 5)",
                params![
                    operation.host_id,
                    operation.team_id,
                    sqlite_integer("team mutation sequence", operation.expected_seqno)?
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id, state)) = occupant {
            if id.as_slice() != operation.operation_id {
                if TeamMutationState::from_sql(state)? == TeamMutationState::Prepared {
                    transaction.execute(
                        "UPDATE team_mutation_operations SET state = ?2, updated_at = ?3
                         WHERE operation_id = ?1",
                        params![
                            id,
                            TeamMutationState::Superseded as u8,
                            sqlite_integer("team mutation updated time", stamped)?,
                        ],
                    )?;
                } else {
                    return Err(Error::InvalidTeamMutation(
                        "team-chain position is already reserved",
                    ));
                }
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
                TeamMutationState::Submitting as u8,
                sqlite_integer("team mutation created time", operation.created_at)?,
                sqlite_integer("team mutation updated time", stamped)?,
            ],
        )?;
        let stored = team_mutation_from_connection(&transaction, &operation.operation_id)?.ok_or(
            Error::InvalidTeamMutation("submitted operation disappeared"),
        )?;
        if stored.kind != operation.kind
            || stored.host_id != operation.host_id
            || stored.actor_id != operation.actor_id
            || stored.device_id != operation.device_id
            || stored.team_id != operation.team_id
            || stored.expected_seqno != operation.expected_seqno
            || stored.request_hash != operation.request_hash
        {
            return Err(Error::InvalidTeamMutation(
                "operation ID was reused for another binding",
            ));
        }
        if stored.state == TeamMutationState::Prepared {
            transaction.execute(
                "UPDATE team_mutation_operations SET state = ?2, updated_at = ?3
                 WHERE operation_id = ?1",
                params![
                    operation.operation_id.as_slice(),
                    TeamMutationState::Submitting as u8,
                    sqlite_integer("team mutation updated time", stamped)?,
                ],
            )?;
        } else if stored.state != TeamMutationState::Submitting {
            return Err(Error::InvalidTeamMutation(
                "operation cannot be submitted more than once",
            ));
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn advance_team_mutation(
        &mut self,
        operation_id: &[u8; 16],
        state: TeamMutationState,
        updated_at: u64,
    ) -> Result<()> {
        let transaction = self.write_transaction()?;
        let current = transaction
            .query_row(
                "SELECT state, created_at, updated_at FROM team_mutation_operations
                 WHERE operation_id = ?1",
                [operation_id.as_slice()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(Error::InvalidTeamMutation("operation is not recorded"))?;
        let current_state = TeamMutationState::from_sql(current.0)?;
        let created_at = stored_unsigned("team mutation created time", current.1)?;
        let previous_updated_at = stored_unsigned("team mutation updated time", current.2)?;
        if !current_state.can_transition_to(state) {
            return Err(Error::InvalidTeamMutation(
                "operation state transition is invalid",
            ));
        }
        let stamped = monotonic_timestamp(created_at, previous_updated_at, updated_at);
        transaction.execute(
            "UPDATE team_mutation_operations SET state = ?2, updated_at = ?3
             WHERE operation_id = ?1",
            params![
                operation_id.as_slice(),
                state as u8,
                sqlite_integer("team mutation updated time", stamped)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn team_mutation(&self, operation_id: &[u8; 16]) -> Result<Option<TeamMutationOperation>> {
        team_mutation_from_connection(&self.connection, operation_id)
    }

    /// Finds the unique journal row occupying one authenticated team-chain
    /// position. This supports crash recovery when the secret-derived
    /// operation ID was not returned to the caller before interruption.
    pub fn team_mutation_at(
        &self,
        host_id: &[u8],
        team_id: &[u8],
        expected_seqno: u64,
    ) -> Result<Option<TeamMutationOperation>> {
        let sequence = sqlite_integer("team mutation sequence", expected_seqno)?;
        let operation_id = self
            .connection
            .query_row(
                "SELECT operation_id FROM team_mutation_operations
                 WHERE host_id = ?1 AND team_id = ?2 AND expected_seqno = ?3
                 ORDER BY CASE
                              WHEN state IN (1, 2, 3, 4) THEN 0
                              WHEN state = 5 THEN 1
                              ELSE 2
                          END,
                          updated_at DESC, operation_id DESC
                 LIMIT 1",
                rusqlite::params![host_id, team_id, sequence],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?
            .map(|bytes| {
                bytes.try_into().map_err(|_| {
                    Error::InvalidTeamMutation("stored operation ID has the wrong length")
                })
            })
            .transpose()?;
        operation_id
            .as_ref()
            .map(|operation_id| self.team_mutation(operation_id))
            .transpose()
            .map(Option::flatten)
    }
}

/// Clamps a local journal timestamp so it never moves backward: a backward
/// wall-clock adjustment cannot leave a row whose `updated_at` predates its
/// `created_at` or a prior update, which would otherwise block submission,
/// finalization, or verification. Used by both the team and generic journals.
fn monotonic_timestamp(created_at: u64, previous_updated_at: u64, requested: u64) -> u64 {
    requested
        .max(created_at)
        .max(previous_updated_at.saturating_add(1))
}

pub(crate) fn team_mutation_from_connection(
    connection: &rusqlite::Connection,
    operation_id: &[u8; 16],
) -> Result<Option<TeamMutationOperation>> {
    connection
        .query_row(
            "SELECT operation_kind, host_id, actor_id, device_id, team_id,
                    expected_seqno, request_hash, state, created_at, updated_at
             FROM team_mutation_operations WHERE operation_id = ?1",
            [operation_id.as_slice()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Vec<u8>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .optional()?
        .map(|row| {
            Ok(TeamMutationOperation {
                operation_id: *operation_id,
                kind: TeamMutationKind::from_sql(row.0)?,
                host_id: row.1,
                actor_id: row.2,
                device_id: row.3,
                team_id: row.4,
                expected_seqno: stored_unsigned("team mutation sequence", row.5)?,
                request_hash: row.6.try_into().map_err(|_| {
                    Error::InvalidTeamMutation("stored request hash has the wrong length")
                })?,
                state: TeamMutationState::from_sql(row.7)?,
                created_at: stored_unsigned("team mutation created time", row.8)?,
                updated_at: stored_unsigned("team mutation updated time", row.9)?,
            })
        })
        .transpose()
}
