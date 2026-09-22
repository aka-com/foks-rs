use std::collections::BTreeMap;

use foks_client_db::{
    Acceptance, HardStateStore, TeamMutationKind, TeamMutationOperation, TeamMutationState,
};
use foks_crypto::derive_shared_public;
use foks_proto::{Role, SecretSeed, UserLink, ENTITY_PTK_VERIFY};
use foks_verify::verify_public_host;
use zeroize::Zeroizing;

use super::{
    frame_protected_team_edit_with_bearer, refresh_operation_id, required_rotation_roles,
    rotation_operation_id, team_member_key_refresh_request_key, validate_rotation_change,
    RefreshBinding, RefreshChange, RotationBinding,
};
use crate::{DeviceCredential, FoksClient, ProtectedMutationStore, ProtectedStoreError};

const DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap()
}

#[derive(Default)]
struct MemoryProtectedStore(BTreeMap<Vec<u8>, Vec<u8>>);

impl ProtectedMutationStore for MemoryProtectedStore {
    fn put_if_absent(
        &mut self,
        key: &[u8],
        value: &[u8],
    ) -> std::result::Result<(), ProtectedStoreError> {
        match self.0.get(key) {
            Some(existing) if existing != value => Err(ProtectedStoreError::Conflict),
            Some(_) => Ok(()),
            None => {
                self.0.insert(key.to_vec(), value.to_vec());
                Ok(())
            }
        }
    }

    fn get(&mut self, key: &[u8]) -> std::result::Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
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

fn initialized_host() -> (tempfile::TempDir, FoksClient, crate::PinnedHost) {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("hard.sqlite3");
    let snapshot = verify_public_host(
        "foks.app",
        include_bytes!(
            "../../../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
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
    let client = FoksClient::default();
    let host = client.pinned_host("foks.app", &database).unwrap();
    (temporary, client, host)
}

fn team_operation(
    host: &crate::PinnedHost,
    operation_id: [u8; 16],
) -> (
    TeamMutationOperation,
    foks_proto::EntityId,
    foks_proto::EntityId,
) {
    let mut actor = vec![0x21; 33];
    actor[0] = foks_proto::ENTITY_USER;
    let actor = foks_proto::EntityId::from_bytes(actor).unwrap();
    let mut device = vec![0x31; 33];
    device[0] = foks_proto::ENTITY_DEVICE;
    let mut team = vec![0x41; 33];
    team[0] = foks_proto::ENTITY_NAMED_TEAM;
    let team = foks_proto::EntityId::from_bytes(team).unwrap();
    (
        TeamMutationOperation {
            operation_id,
            kind: TeamMutationKind::PtkRotation,
            host_id: host.host_id().as_bytes().to_vec(),
            actor_id: actor.as_bytes().to_vec(),
            device_id: device,
            team_id: team.as_bytes().to_vec(),
            expected_seqno: 2,
            request_hash: [0x51; 32],
            state: TeamMutationState::Prepared,
            created_at: 100,
            updated_at: 100,
        },
        actor,
        team,
    )
}

#[test]
fn reconciliation_is_bound_to_the_exact_official_rotation() {
    let link = UserLink::decode(&fixture("remove-member-link.snowp")).unwrap();
    let change = link.decode_team_group_change().unwrap();
    let rotations = [
        "remove-member-ptk-member-min-seed.bin",
        "remove-member-ptk-member-seed.bin",
    ];
    let introduced = change
        .shared_keys
        .iter()
        .zip(rotations)
        .map(|(key, name)| {
            let seed = SecretSeed::new(fixture(name).try_into().unwrap());
            (
                key.role,
                key.generation,
                derive_shared_public(&seed, ENTITY_PTK_VERIFY)
                    .unwrap()
                    .verify_key,
            )
        })
        .collect();
    let binding = RotationBinding {
        target: change.changes[0].party.clone(),
        target_host: None,
        target_source_role: Role::OWNER,
        removal_key_commitment: [7; 32],
        destination_role: Role::NONE,
        replacement: None,
        expected_seqno: change.seqno,
        introduced,
    };
    validate_rotation_change(&change, &binding).unwrap();
    let operation =
        rotation_operation_id(&change.signer_owner.party, &change.team, &binding).unwrap();

    let mut wrong = binding;
    wrong.removal_key_commitment[0] ^= 1;
    assert_ne!(
        operation,
        rotation_operation_id(&change.signer_owner.party, &change.team, &wrong).unwrap()
    );
    wrong.introduced.pop();
    assert!(validate_rotation_change(&change, &wrong).is_err());
}

#[test]
fn team_member_key_refresh_operation_identity_is_bound_to_the_actor_not_the_transport_device() {
    let mut actor = vec![0x22; 33];
    actor[0] = foks_proto::ENTITY_USER;
    let actor = foks_proto::EntityId::from_bytes(actor).unwrap();
    let mut team = vec![0x32; 33];
    team[0] = foks_proto::ENTITY_NAMED_TEAM;
    let team = foks_proto::EntityId::from_bytes(team).unwrap();
    let mut target = vec![0x42; 33];
    target[0] = foks_proto::ENTITY_USER;
    let target = foks_proto::EntityId::from_bytes(target).unwrap();
    let mut verify = vec![0x52; 33];
    verify[0] = foks_proto::ENTITY_PUK_VERIFY;
    let verify = foks_proto::EntityId::from_bytes(verify).unwrap();
    let mut ptk_verify = vec![0x62; 33];
    ptk_verify[0] = foks_proto::ENTITY_PTK_VERIFY;
    let ptk_verify = foks_proto::EntityId::from_bytes(ptk_verify).unwrap();
    let binding = RefreshBinding {
        changes: vec![RefreshChange {
            party: target,
            host: None,
            source_role: Role::OWNER,
            destination_role: Role::OWNER,
            generation: 2,
            verify_key: verify,
            hepk_fingerprint: [0x72; 32],
        }],
        expected_seqno: 3,
        introduced: vec![(Role::OWNER, 2, ptk_verify, [0x82; 32])],
    };

    // Both public credential wrappers feed this same UID-based identity
    // function; neither a software seed nor a Yubi parent ID is an input.
    let software_path = refresh_operation_id(&actor, &team, &binding).unwrap();
    let yubi_path = refresh_operation_id(&actor, &team, &binding).unwrap();
    assert_eq!(software_path, yubi_path);

    let mut other_actor = actor.into_bytes();
    other_actor[32] ^= 1;
    let other_actor = foks_proto::EntityId::from_bytes(other_actor).unwrap();
    assert_ne!(
        software_path,
        refresh_operation_id(&other_actor, &team, &binding).unwrap()
    );

    let mut team_actor = vec![0x92; 33];
    team_actor[0] = foks_proto::ENTITY_NAMED_TEAM;
    let team_actor = foks_proto::EntityId::from_bytes(team_actor).unwrap();
    let team_actor_path = refresh_operation_id(&team_actor, &team, &binding).unwrap();
    assert_ne!(software_path, team_actor_path);
    assert_eq!(
        team_actor_path,
        refresh_operation_id(&team_actor, &team, &binding).unwrap()
    );
}

#[test]
fn yubi_team_member_key_refresh_api_covers_prepare_submit_replay_resume_and_discard() {
    let _ = FoksClient::refresh_team_member_keys_operation_id_yubi;
    let _ = FoksClient::refresh_team_member_keys_and_rotate_ptks_yubi;
    let _ = FoksClient::replay_recorded_team_rekey_yubi;
    let _ = FoksClient::resume_refresh_team_member_keys_and_rotate_ptks_yubi;
    let _ = FoksClient::discard_unrecorded_team_rekey_yubi;
}

#[test]
fn local_team_actor_api_covers_software_and_yubi_crash_recovery() {
    let _ = FoksClient::refresh_team_member_keys_operation_id_for_actor;
    let _ = FoksClient::refresh_team_member_keys_and_rotate_ptks_as_local_team;
    let _ = FoksClient::refresh_team_member_keys_and_rotate_ptks_as_local_team_yubi;
    let _ = FoksClient::resume_refresh_team_member_keys_and_rotate_ptks_as_local_team;
    let _ = FoksClient::resume_refresh_team_member_keys_and_rotate_ptks_as_local_team_yubi;
    let _ = FoksClient::replay_recorded_team_rekey_as_local_team;
    let _ = FoksClient::replay_recorded_team_rekey_as_local_team_yubi;
    let _ = FoksClient::discard_unrecorded_team_rekey_for_local_team_actor;
}

#[test]
fn membership_changes_rotate_the_required_role_keys() {
    let roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    assert_eq!(
        required_rotation_roles(roles, Role::OWNER, 1, Role::NONE, None),
        roles
    );
    assert_eq!(
        required_rotation_roles(roles, Role::OWNER, 1, Role::ADMIN, Some(1)),
        [Role::OWNER]
    );
    assert_eq!(
        required_rotation_roles(roles, Role::member(0), 3, Role::member(0), Some(4)),
        [Role::member(-0x4000), Role::member(0)]
    );
}

#[test]
fn protected_only_team_member_key_refresh_frame_can_be_rebuilt_but_a_journaled_frame_cannot_be_discarded(
) {
    let (_temporary, client, host) = initialized_host();
    let operation_id = [0x61; 16];
    let key = team_member_key_refresh_request_key(&operation_id);
    let mut protected = MemoryProtectedStore::default();
    protected
        .put_if_absent(&key, b"stale head-bound frame")
        .unwrap();

    client
        .discard_unjournaled_team_rekey_request(&host, &operation_id, &mut protected)
        .unwrap();
    assert!(matches!(
        protected.get(&key),
        Err(ProtectedStoreError::Missing)
    ));
    protected
        .put_if_absent(&key, b"rebuilt latest-head frame")
        .unwrap();

    let (operation, _, _) = team_operation(&host, operation_id);
    HardStateStore::open(&host.database_path)
        .unwrap()
        .record_team_mutation(&operation)
        .unwrap();
    assert!(client
        .discard_unjournaled_team_rekey_request(&host, &operation_id, &mut protected)
        .is_err());
    assert_eq!(
        protected.get(&key).unwrap().as_slice(),
        b"rebuilt latest-head frame"
    );
}

#[test]
fn superseded_member_edit_releases_its_journal_and_protected_request() {
    let (_temporary, client, host) = initialized_host();
    let operation_id = [0x69; 16];
    let (operation, actor, team) = team_operation(&host, operation_id);
    HardStateStore::open(&host.database_path)
        .unwrap()
        .record_team_mutation(&operation)
        .unwrap();
    let key = super::team_member_change_request_key(&operation_id);
    let mut protected = MemoryProtectedStore::default();
    protected.put_if_absent(&key, b"lost-race frame").unwrap();
    let credential = DeviceCredential {
        key_kind: crate::SoftwareKeyKind::Device,
        uid: actor,
        seed: SecretSeed::new([0x71; 32]),
        certificate_chain: Vec::new(),
    };

    client
        .supersede_recorded_team_member_change(
            &host,
            &credential,
            &team,
            operation.expected_seqno,
            &operation_id,
            &mut protected,
        )
        .unwrap();

    assert_eq!(
        HardStateStore::open(&host.database_path)
            .unwrap()
            .team_mutation(&operation_id)
            .unwrap()
            .unwrap()
            .state,
        TeamMutationState::Superseded
    );
    assert!(matches!(
        protected.get(&key),
        Err(ProtectedStoreError::Missing)
    ));
}

#[test]
fn recorded_team_member_key_refresh_cleanup_rejects_active_submission_without_a_witness() {
    let (_temporary, client, host) = initialized_host();
    let operation_id = [0x71; 16];
    let (operation, actor, team) = team_operation(&host, operation_id);
    let key = team_member_key_refresh_request_key(&operation_id);
    let mut protected = MemoryProtectedStore::default();
    protected
        .put_if_absent(&key, b"exact submitted frame")
        .unwrap();
    HardStateStore::open(&host.database_path)
        .unwrap()
        .record_and_begin_team_mutation(&operation, 101)
        .unwrap();

    let mut wrong_actor = actor.as_bytes().to_vec();
    wrong_actor[32] ^= 1;
    let wrong_actor = foks_proto::EntityId::from_bytes(wrong_actor).unwrap();
    assert!(client
        .reject_recorded_team_rekey(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &wrong_actor,
            &mut protected,
        )
        .is_err());
    assert_eq!(
        HardStateStore::open(&host.database_path)
            .unwrap()
            .team_mutation(&operation_id)
            .unwrap()
            .unwrap()
            .state,
        TeamMutationState::Submitting
    );
    assert_eq!(
        protected.get(&key).unwrap().as_slice(),
        b"exact submitted frame"
    );

    assert!(client
        .reject_recorded_team_rekey(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &actor,
            &mut protected,
        )
        .is_err());
    assert_eq!(
        HardStateStore::open(&host.database_path)
            .unwrap()
            .team_mutation(&operation_id)
            .unwrap()
            .unwrap()
            .state,
        TeamMutationState::Submitting
    );
    assert_eq!(
        protected.get(&key).unwrap().as_slice(),
        b"exact submitted frame"
    );

    HardStateStore::open(&host.database_path)
        .unwrap()
        .advance_team_mutation(&operation_id, TeamMutationState::Rejected, 102)
        .unwrap();

    client
        .reject_recorded_team_rekey(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &actor,
            &mut protected,
        )
        .unwrap();
    assert!(matches!(
        protected.get(&key),
        Err(ProtectedStoreError::Missing)
    ));

    client
        .reject_recorded_team_rekey(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &actor,
            &mut protected,
        )
        .unwrap();
}

#[test]
fn verified_team_member_key_refresh_cleanup_tolerates_request_already_removed() {
    let (_temporary, client, host) = initialized_host();
    let operation_id = [0x81; 16];
    let (operation, actor, team) = team_operation(&host, operation_id);
    let mut hard_store = HardStateStore::open(&host.database_path).unwrap();
    hard_store.record_team_mutation(&operation).unwrap();
    let mut protected = MemoryProtectedStore::default();
    let key = team_member_key_refresh_request_key(&operation_id);
    protected.put_if_absent(&key, b"verified frame").unwrap();

    assert!(client
        .cleanup_verified_team_rekey_request(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &actor,
            &mut protected,
        )
        .is_err());
    assert_eq!(protected.get(&key).unwrap().as_slice(), b"verified frame");

    hard_store
        .advance_team_mutation(&operation_id, TeamMutationState::Submitting, 101)
        .unwrap();
    hard_store
        .advance_team_mutation(&operation_id, TeamMutationState::Verified, 102)
        .unwrap();
    let mut wrong_actor = actor.as_bytes().to_vec();
    wrong_actor[32] ^= 1;
    let wrong_actor = foks_proto::EntityId::from_bytes(wrong_actor).unwrap();
    assert!(client
        .cleanup_verified_team_rekey_request(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &wrong_actor,
            &mut protected,
        )
        .is_err());
    assert_eq!(protected.get(&key).unwrap().as_slice(), b"verified frame");

    client
        .cleanup_verified_team_rekey_request(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &actor,
            &mut protected,
        )
        .unwrap();
    client
        .cleanup_verified_team_rekey_request(
            &host,
            &team,
            operation.expected_seqno,
            &operation_id,
            &actor,
            &mut protected,
        )
        .unwrap();
    assert!(matches!(
        protected.get(&key),
        Err(ProtectedStoreError::Missing)
    ));
    assert_eq!(
        hard_store
            .team_mutation(&operation_id)
            .unwrap()
            .unwrap()
            .state,
        TeamMutationState::Verified
    );
}

#[test]
fn nested_team_member_key_refresh_bearer_is_attached_only_to_the_submission_frame() {
    let link = UserLink::decode(&fixture("remove-member-link.snowp")).unwrap();
    let boxes = foks_proto::SharedKeyBoxSet::new([0x90; 16], Vec::new(), None).unwrap();
    let request =
        foks_rpc::encode_remove_team_member_request(&foks_proto::RemoveTeamMemberArgument {
            link: &link,
            next_tree_location: [0x91; 32],
            ptk_boxes: &boxes,
            seed_chain: &[],
            removals: &[],
            hepks: &[],
            new_key_on_rotate: None,
            team_bearer_token: None,
        })
        .unwrap();
    let bearer = [0xa1; 16];
    let framed = frame_protected_team_edit_with_bearer(&request, Some(&bearer)).unwrap();
    let protected = super::decode_protected_team_edit_request(&request).unwrap();
    let submitted = super::decode_protected_team_edit_request(&framed).unwrap();

    assert!(protected.team_bearer_token.is_none());
    assert_eq!(submitted.team_bearer_token, Some(bearer));
    assert_eq!(submitted.link.encoded().unwrap(), link.encoded().unwrap());
    assert_eq!(submitted.next_tree_location, [0x91; 32]);
    assert_eq!(
        frame_protected_team_edit_with_bearer(&request, None).unwrap(),
        request
    );
}
