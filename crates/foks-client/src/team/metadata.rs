//! Signed, crash-recoverable team metadata transitions.

use std::time::Duration;

use foks_client_db::{HardStateStore, TeamMutationKind, TeamMutationOperation, TeamMutationState};
use foks_crypto::{make_team_index_range_link, prefixed_hash, TeamMetadataInput};
use foks_proto::{
    ChangeMetadata, EntityId, Rational, RationalRange, RoleType, TeamMetadataEditArgument,
    TreeRoot, ENTITY_NAMED_TEAM, MERKLE_ROOT_TYPE_ID, TREE_LOCATION_TYPE_ID,
};
use foks_rpc::{decode_team_edit_result, encode_team_metadata_edit_request, STATUS_TX_RETRY_ERROR};
use foks_snowpack::{encode, Value};

use super::{membership::authorized_actor_member, AuthenticatedTeamOutcome};
use crate::{
    current_owner_puk, now_microseconds, now_milliseconds, random_bytes, user_key_for_seed,
    AuthenticatedUserOutcome, DeviceCredential, Error, FoksClient, PinnedHost,
    ProtectedMutationStore, ProtectedStoreError, Result, TEAM_MUTATION_OPERATION_ID_TYPE_ID,
    TEAM_MUTATION_REQUEST_HASH_TYPE_ID,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TeamIndexRangeDirection {
    Lower,
    Raise,
}

pub struct TeamIndexRangeMutationOutcome {
    pub operation_id: [u8; 16],
    pub range: RationalRange,
    pub authenticated: AuthenticatedTeamOutcome,
}

impl FoksClient {
    /// Authenticates a team's current range using the same owner credential that
    /// can subsequently sign a metadata transition.
    pub(crate) fn authenticated_team_for_index_range(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        let user = self.authenticate_and_pin(host, credential)?;
        self.load_and_pin_team(host, credential, &user.verified, &user.puks, team)
    }

    pub fn lower_team_index_range_durable(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<TeamIndexRangeMutationOutcome> {
        self.narrow_team_index_range_durable(
            host,
            crate::FederationCredential::Software(credential),
            team,
            TeamIndexRangeDirection::Lower,
            protected_store,
        )
    }

    pub fn raise_team_index_range_durable(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &EntityId,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<TeamIndexRangeMutationOutcome> {
        self.narrow_team_index_range_durable(
            host,
            crate::FederationCredential::Software(credential),
            team,
            TeamIndexRangeDirection::Raise,
            protected_store,
        )
    }

    pub fn narrow_team_index_range_durable(
        &self,
        host: &PinnedHost,
        credential: crate::FederationCredential<'_, '_>,
        team: &EntityId,
        direction: TeamIndexRangeDirection,
        protected_store: &mut dyn ProtectedMutationStore,
    ) -> Result<TeamIndexRangeMutationOutcome> {
        team.clone().require_type(ENTITY_NAMED_TEAM)?;
        let user = self.authenticate_credential_and_pin(host, credential)?;
        let owner = current_owner_puk(&user)?;
        let device_id = credential.device_id()?;
        let authenticated = self.load_and_pin_team_with_credential(
            host,
            credential,
            &user.verified,
            &user.puks,
            team,
        )?;
        let actor_public = user_key_for_seed(&user.verified, &owner.seed)?;
        let actor =
            authorized_actor_member(&authenticated.verified, credential.uid(), actor_public)?;
        if !matches!(actor.role.kind(), RoleType::Admin | RoleType::Owner) {
            return Err(Error::TeamRequest(
                "team index range can be changed only by an administrator",
            ));
        }

        let mut hard_store = HardStateStore::open(&host.database_path)?;
        if let Some(operation) = hard_store.latest_team_mutation(
            host.host_id().as_bytes(),
            team.as_bytes(),
            TeamMutationKind::MetadataChange,
        )? {
            let key = team_index_range_request_key(&operation.operation_id);
            match protected_store.get(&key) {
                Ok(exact_request) => {
                    return self.resume_team_index_range_mutation(
                        host,
                        credential,
                        team,
                        &user,
                        authenticated,
                        &device_id,
                        operation,
                        &exact_request,
                        protected_store,
                        &mut hard_store,
                    );
                }
                Err(ProtectedStoreError::Missing)
                    if operation.state == TeamMutationState::Verified => {}
                Err(ProtectedStoreError::Missing) => {
                    return Err(Error::OperationBinding(
                        "pending team metadata mutation lost its protected request",
                    ));
                }
                Err(error) => return Err(Error::ProtectedStore(error.to_string())),
            }
        }

        let current = authenticated.verified.index_range();
        let next = match direction {
            TeamIndexRangeDirection::Lower => lower_index_range(current)?,
            TeamIndexRangeDirection::Raise => raise_index_range(current)?,
        };
        require_strict_narrowing(current, &next)?;
        let expected_seqno = authenticated
            .verified
            .chain_seqno()
            .checked_add(1)
            .ok_or(Error::TeamRequest("team sequence overflow"))?;
        let (_, merkle) = self.advance_merkle_root(host)?;
        let root = TreeRoot {
            epoch: merkle.root().epoch,
            hash: foks_crypto::prefixed_hash_signable(
                MERKLE_ROOT_TYPE_ID,
                &merkle.root().encoded()?,
            )?,
        };
        let link_output = make_team_index_range_link(
            &TeamMetadataInput {
                actor: credential.uid(),
                actor_source_role: actor_public.role,
                team,
                host: host.host_id(),
                sequence: expected_seqno,
                previous: authenticated.verified.chain_tail_hash(),
                root: &root,
                time: now_milliseconds()?,
                next_tree_location: random_bytes()?,
                index_range: &next,
            },
            &owner.seed,
        )?;
        let exact_request = encode_team_metadata_edit_request(&TeamMetadataEditArgument {
            link: &link_output.link,
            next_tree_location: link_output.next_tree_location,
        })?;
        let operation_id = team_index_range_operation_id(
            host.host_id(),
            credential.uid(),
            team,
            expected_seqno,
            &next,
        )?;
        let request_key = team_index_range_request_key(&operation_id);
        match protected_store.put_if_absent(&request_key, &exact_request) {
            Ok(()) | Err(ProtectedStoreError::Conflict) => {}
            Err(error) => return Err(Error::ProtectedStore(error.to_string())),
        }
        let retained = protected_store
            .get(&request_key)
            .map_err(|error| Error::ProtectedStore(error.to_string()))?;
        let retained_target =
            validate_protected_team_index_range_request(&retained, team, expected_seqno)?;
        if !rational_range_equal(&retained_target, &next)? {
            return Err(Error::OperationBinding(
                "protected team metadata request changed its allocated range",
            ));
        }
        let created_at = now_microseconds()?;
        let operation = TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::MetadataChange,
            host_id: host.host_id().as_bytes().to_vec(),
            actor_id: credential.uid().as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: team.as_bytes().to_vec(),
            expected_seqno,
            request_hash: prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, &retained),
            state: TeamMutationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        hard_store.record_and_begin_team_mutation(&operation, now_microseconds()?)?;
        self.submit_and_reconcile_team_index_range(
            host,
            credential,
            team,
            &user,
            operation,
            &retained,
            protected_store,
            &mut hard_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn resume_team_index_range_mutation(
        &self,
        host: &PinnedHost,
        credential: crate::FederationCredential<'_, '_>,
        team: &EntityId,
        user: &AuthenticatedUserOutcome,
        authenticated: AuthenticatedTeamOutcome,
        device_id: &EntityId,
        operation: TeamMutationOperation,
        exact_request: &[u8],
        protected_store: &mut dyn ProtectedMutationStore,
        hard_store: &mut HardStateStore,
    ) -> Result<TeamIndexRangeMutationOutcome> {
        validate_team_index_range_operation(&operation, host, credential.uid(), device_id, team)?;
        if prefixed_hash(TEAM_MUTATION_REQUEST_HASH_TYPE_ID, exact_request)
            != operation.request_hash
        {
            return Err(Error::OperationBinding(
                "protected team metadata request changed",
            ));
        }
        let target = validate_protected_team_index_range_request(
            exact_request,
            team,
            operation.expected_seqno,
        )?;
        if authenticated.verified.chain_seqno() >= operation.expected_seqno {
            validate_team_index_range_transition(
                &authenticated.verified,
                operation.expected_seqno,
                &target,
            )?;
            super::membership::finish_team_mutation_journal(hard_store, &operation.operation_id)?;
            remove_team_index_range_request(protected_store, &operation.operation_id)?;
            return Ok(TeamIndexRangeMutationOutcome {
                operation_id: operation.operation_id,
                range: target,
                authenticated,
            });
        }
        if authenticated.verified.chain_seqno().checked_add(1) != Some(operation.expected_seqno) {
            return Err(Error::OperationBinding(
                "pending team metadata sequence is not the authenticated next position",
            ));
        }
        if operation.state == TeamMutationState::Prepared {
            hard_store.advance_team_mutation(
                &operation.operation_id,
                TeamMutationState::Submitting,
                now_microseconds()?,
            )?;
        }
        self.submit_and_reconcile_team_index_range(
            host,
            credential,
            team,
            user,
            operation,
            exact_request,
            protected_store,
            hard_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_and_reconcile_team_index_range(
        &self,
        host: &PinnedHost,
        credential: crate::FederationCredential<'_, '_>,
        team: &EntityId,
        user: &AuthenticatedUserOutcome,
        operation: TeamMutationOperation,
        exact_request: &[u8],
        protected_store: &mut dyn ProtectedMutationStore,
        hard_store: &mut HardStateStore,
    ) -> Result<TeamIndexRangeMutationOutcome> {
        let target = validate_protected_team_index_range_request(
            exact_request,
            team,
            operation.expected_seqno,
        )?;
        let post = || {
            let response = self.call_with_material(
                host,
                &host.user,
                exact_request,
                credential.transport().0,
                credential.transport().1,
            )?;
            decode_team_edit_result(&response)?;
            Ok(())
        };
        let post_error = match post() {
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: STATUS_TX_RETRY_ERROR,
                ..
            })) => post().err(),
            Err(error) => Some(error),
            Ok(()) => None,
        };
        hard_store.advance_team_mutation(
            &operation.operation_id,
            if post_error.is_none() {
                TeamMutationState::Submitted
            } else {
                TeamMutationState::SubmissionUnknown
            },
            now_microseconds()?,
        )?;
        let observed = self.wait_for_team_index_range_transition(
            host,
            credential,
            team,
            user,
            operation.expected_seqno,
            &target,
        );
        let authenticated = match observed {
            Ok(value) => value,
            Err(error @ Error::OperationBinding(_)) => {
                hard_store.advance_team_mutation(
                    &operation.operation_id,
                    TeamMutationState::Superseded,
                    now_microseconds()?,
                )?;
                remove_team_index_range_request(protected_store, &operation.operation_id)?;
                return Err(error);
            }
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        super::membership::finish_team_mutation_journal(hard_store, &operation.operation_id)?;
        remove_team_index_range_request(protected_store, &operation.operation_id)?;
        Ok(TeamIndexRangeMutationOutcome {
            operation_id: operation.operation_id,
            range: target,
            authenticated,
        })
    }

    fn wait_for_team_index_range_transition(
        &self,
        host: &PinnedHost,
        credential: crate::FederationCredential<'_, '_>,
        team: &EntityId,
        user: &AuthenticatedUserOutcome,
        expected_seqno: u64,
        target: &RationalRange,
    ) -> Result<AuthenticatedTeamOutcome> {
        let mut last_error = None;
        for attempt in 0..40 {
            match self.load_and_pin_team_with_credential(
                host,
                credential,
                &user.verified,
                &user.puks,
                team,
            ) {
                Ok(authenticated) if authenticated.verified.chain_seqno() >= expected_seqno => {
                    validate_team_index_range_transition(
                        &authenticated.verified,
                        expected_seqno,
                        target,
                    )?;
                    return Ok(authenticated);
                }
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved(
                        "team chain has not reached the prepared metadata transition",
                    ));
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Err(last_error.expect("team transition loop executes at least once"))
    }
}

pub fn lower_index_range(current: &RationalRange) -> Result<RationalRange> {
    foks_verify::validate_rational_range(current)?;
    let next = if rational_range_equal(current, &default_index_range())? {
        RationalRange {
            low: current.low.clone(),
            high: finite_rational(vec![0x20], 0),
        }
    } else {
        RationalRange {
            low: current.low.clone(),
            high: rational_right_shift(&current.high)?,
        }
    };
    require_strict_narrowing(current, &next)?;
    Ok(next)
}

pub fn raise_index_range(current: &RationalRange) -> Result<RationalRange> {
    foks_verify::validate_rational_range(current)?;
    let next = if rational_range_equal(current, &default_index_range())? {
        RationalRange {
            low: finite_rational(vec![0x80], 0),
            high: current.high.clone(),
        }
    } else {
        RationalRange {
            low: rational_left_shift(&current.low)?,
            high: current.high.clone(),
        }
    };
    require_strict_narrowing(current, &next)?;
    Ok(next)
}

pub fn default_index_range() -> RationalRange {
    foks_verify::canonical_default_team_index_range()
}

fn finite_rational(base: Vec<u8>, exponent: i64) -> Rational {
    Rational {
        infinity: false,
        base,
        exponent,
    }
}

fn rational_right_shift(value: &Rational) -> Result<Rational> {
    if value.infinity {
        return Err(Error::TeamRequest(
            "infinite team range bound cannot be lowered",
        ));
    }
    let mut base = Vec::with_capacity(value.base.len().saturating_add(1));
    let mut carry = false;
    for byte in &value.base {
        let next_carry = byte & 1 != 0;
        base.push((byte >> 1) | if carry { 0x80 } else { 0 });
        carry = next_carry;
    }
    let mut exponent = value.exponent;
    if carry {
        base.push(0x80);
        exponent = exponent
            .checked_sub(1)
            .ok_or(Error::TeamRequest("team range exponent underflow"))?;
    }
    if base.first() == Some(&0) {
        base.remove(0);
    }
    Ok(finite_rational(base, exponent))
}

fn rational_left_shift(value: &Rational) -> Result<Rational> {
    if value.infinity {
        return Err(Error::TeamRequest(
            "infinite team range bound cannot be raised",
        ));
    }
    if value.base.is_empty() {
        return Err(Error::TeamRequest("zero team range bound cannot be raised"));
    }
    let mut base = vec![0; value.base.len()];
    let mut carry = false;
    for index in (0..value.base.len()).rev() {
        let byte = value.base[index];
        let next_carry = byte & 0x80 != 0;
        base[index] = (byte << 1) | u8::from(carry);
        carry = next_carry;
    }
    if carry {
        base.insert(0, 1);
    }
    let mut exponent = value.exponent;
    if base.last() == Some(&0) {
        base.pop();
        exponent = exponent
            .checked_add(1)
            .ok_or(Error::TeamRequest("team range exponent overflow"))?;
    }
    Ok(finite_rational(base, exponent))
}

fn require_strict_narrowing(current: &RationalRange, next: &RationalRange) -> Result<()> {
    foks_verify::validate_rational_range(next)?;
    if rational_range_equal(current, next)? || !foks_verify::rational_range_includes(current, next)?
    {
        return Err(Error::TeamRequest(
            "team index range transition is not a strict narrowing",
        ));
    }
    Ok(())
}

fn rational_range_equal(left: &RationalRange, right: &RationalRange) -> Result<bool> {
    Ok(
        foks_verify::compare_rationals(&left.low, &right.low)? == std::cmp::Ordering::Equal
            && foks_verify::compare_rationals(&left.high, &right.high)?
                == std::cmp::Ordering::Equal,
    )
}

fn validate_team_index_range_transition(
    team: &foks_verify::VerifiedTeamState,
    sequence: u64,
    target: &RationalRange,
) -> Result<()> {
    let change = team.group_change_at(sequence)?;
    let [ChangeMetadata::TeamIndexRange(observed)] = change.metadata.as_slice() else {
        return Err(Error::OperationBinding(
            "prepared metadata sequence contains another transition",
        ));
    };
    if !change.changes.is_empty()
        || !change.shared_keys.is_empty()
        || !rational_range_equal(observed, target)?
    {
        return Err(Error::OperationBinding(
            "observed team transition does not match the prepared index range",
        ));
    }
    Ok(())
}

fn validate_protected_team_index_range_request(
    request: &[u8],
    team: &EntityId,
    expected_seqno: u64,
) -> Result<RationalRange> {
    let mut framed = std::io::Cursor::new(request);
    let call = foks_rpc::read_call(&mut framed, foks_rpc::DEFAULT_MAX_FRAME_LENGTH)
        .map_err(|_| Error::OperationBinding("protected metadata request is malformed"))?;
    if usize::try_from(framed.position()).ok() != Some(request.len())
        || call.protocol_id() != foks_rpc::TEAM_ADMIN_PROTOCOL_ID
        || call.method_position() != foks_rpc::TEAM_EDIT_METHOD_POSITION
    {
        return Err(Error::OperationBinding(
            "protected metadata request targets another route",
        ));
    }
    let decoded = foks_proto::DecodedTeamEditArgument::decode(call.argument())?;
    let change = decoded.link.decode_team_group_change()?;
    validate_next_tree_location_commitment(
        decoded.next_tree_location,
        change.next_location_commitment,
    )?;
    let [ChangeMetadata::TeamIndexRange(target)] = change.metadata.as_slice() else {
        return Err(Error::OperationBinding(
            "protected metadata request changed its target",
        ));
    };
    if change.team != *team
        || change.seqno != expected_seqno
        || !change.changes.is_empty()
        || !change.shared_keys.is_empty()
        || !decoded.ptk_boxes.boxes.is_empty()
        || !decoded.seed_chain.is_empty()
        || !decoded.removal_keys.is_empty()
        || !decoded.removals.is_empty()
        || !decoded.hepks.is_empty()
        || decoded.new_key_on_rotate.is_some()
        || decoded.team_bearer_token.is_some()
        || !decoded.remote_member_view_tokens.is_empty()
        || !decoded.local_permissions_for.is_empty()
    {
        return Err(Error::OperationBinding(
            "protected metadata request changed its authority or payload",
        ));
    }
    foks_verify::validate_rational_range(target)?;
    Ok(target.clone())
}

fn validate_next_tree_location_commitment(
    next_tree_location: [u8; 32],
    expected_commitment: [u8; 32],
) -> Result<()> {
    let location_wire = encode(&Value::Binary(next_tree_location.to_vec()))?;
    if prefixed_hash(TREE_LOCATION_TYPE_ID, &location_wire) != expected_commitment {
        return Err(Error::OperationBinding(
            "protected metadata request changed its next tree location",
        ));
    }
    Ok(())
}

fn validate_team_index_range_operation(
    operation: &TeamMutationOperation,
    host: &PinnedHost,
    actor: &EntityId,
    device: &EntityId,
    team: &EntityId,
) -> Result<()> {
    if operation.kind != TeamMutationKind::MetadataChange
        || operation.host_id != host.host_id().as_bytes()
        || operation.actor_id != actor.as_bytes()
        || operation.device_id != device.as_bytes()
        || operation.team_id != team.as_bytes()
    {
        return Err(Error::OperationBinding(
            "team metadata journal does not match supplied identities",
        ));
    }
    Ok(())
}

fn team_index_range_operation_id(
    host: &EntityId,
    actor: &EntityId,
    team: &EntityId,
    expected_seqno: u64,
    target: &RationalRange,
) -> Result<[u8; 16]> {
    let binding = encode(&Value::Array(vec![
        Value::Binary(host.as_bytes().to_vec()),
        Value::Binary(actor.as_bytes().to_vec()),
        Value::Binary(team.as_bytes().to_vec()),
        Value::Unsigned(expected_seqno),
        rational_range_value(target),
    ]))?;
    let hash = prefixed_hash(TEAM_MUTATION_OPERATION_ID_TYPE_ID, &binding);
    Ok(hash[..16].try_into().expect("hash prefix has fixed length"))
}

fn rational_range_value(range: &RationalRange) -> Value {
    Value::Array(vec![
        rational_value(&range.low),
        rational_value(&range.high),
    ])
}

fn rational_value(value: &Rational) -> Value {
    Value::Array(vec![
        Value::Bool(value.infinity),
        if value.base.is_empty() {
            Value::Null
        } else {
            Value::Binary(value.base.clone())
        },
        if value.exponent < 0 {
            Value::Negative(value.exponent)
        } else {
            Value::Unsigned(value.exponent as u64)
        },
    ])
}

fn team_index_range_request_key(operation_id: &[u8; 16]) -> Vec<u8> {
    crate::ProtectedRecordKey::TeamMetadata(operation_id).encoded()
}

fn remove_team_index_range_request(
    store: &mut dyn ProtectedMutationStore,
    operation_id: &[u8; 16],
) -> Result<()> {
    match store.remove(&team_index_range_request_key(operation_id)) {
        Ok(()) | Err(ProtectedStoreError::Missing) => Ok(()),
        Err(error) => Err(Error::ProtectedStore(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_default_and_shift_allocations_match() {
        let default = default_index_range();
        assert_eq!(
            lower_index_range(&default).unwrap().high,
            finite_rational(vec![0x20], 0)
        );
        assert_eq!(
            raise_index_range(&default).unwrap().low,
            finite_rational(vec![0x80], 0)
        );

        let lowered = lower_index_range(&lower_index_range(&default).unwrap()).unwrap();
        assert_eq!(lowered.high, finite_rational(vec![0x10], 0));
        let raised = raise_index_range(&raise_index_range(&default).unwrap()).unwrap();
        assert_eq!(raised.low, finite_rational(vec![1], 1));
    }

    #[test]
    fn shifts_preserve_upstream_base_256_carry_rules() {
        assert_eq!(
            rational_right_shift(&finite_rational(vec![1], 0)).unwrap(),
            finite_rational(vec![0x80], -1)
        );
        assert_eq!(
            rational_left_shift(&finite_rational(vec![0x80], 0)).unwrap(),
            finite_rational(vec![1], 1)
        );
    }

    #[test]
    fn protected_metadata_location_must_match_the_signed_commitment() {
        let location = [7; 32];
        let wire = encode(&Value::Binary(location.to_vec())).unwrap();
        let commitment = prefixed_hash(TREE_LOCATION_TYPE_ID, &wire);
        validate_next_tree_location_commitment(location, commitment).unwrap();

        let mut changed = location;
        changed[0] ^= 1;
        assert!(matches!(
            validate_next_tree_location_commitment(changed, commitment),
            Err(Error::OperationBinding(
                "protected metadata request changed its next tree location"
            ))
        ));
    }
}
