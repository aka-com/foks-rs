//! Native FOKS v0.1.9 host, identity, recovery, team, and KV client.
//!
//! This is deliberately the smallest useful client slice: WebPKI TLS, the
//! public and authenticated RPC, full host/Merkle/chain verification, atomic
//! SQLite hard-state advancement, backup-key account recovery, and verified
//! read/write KV soft projections.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use foks_client_db::{
    Acceptance, HardStateStore, KvDirectoryProjection, MutationKind, MutationOperation,
    MutationState, StoredHostSnapshot,
};
use foks_crypto::{
    derive_device_public, derive_shared_verify_key, derive_subkey_id, make_software_eldest_link,
    make_software_provision_link, make_software_puk_rotation_link, make_software_revoke_link,
    open_puk_parcel_for_role, open_puk_parcel_with_for_role, open_puk_seed_chain, prefixed_hash,
    seal_initial_puk_box, seal_puk_seed_chain_box, seal_software_puk_boxes, DevicePublicMaterial,
    InitialPukBoxRandomness, PukBoxRandomness, PukRotation, SoftwareEldestInput,
    SoftwareEldestMaterial, SoftwareProvisionInput, SoftwarePukBoxInput, UserMutationBase,
    YubiDevice,
};
use foks_proto::{
    ClientVersionExt, DeviceLabel, DeviceLabelNameAndCommitmentKey, DeviceNagInfo, DeviceType,
    EntityId, HostchainTail, InviteCode, PassphraseUpdateArgument, PermissionToken,
    ProvisionDeviceArgument, PukParcel, RegServerConfig, RevokeDeviceArgument, Role, SecretSeed,
    ServerClientVersionInfo, ServiceType, SharedKeyBoxSet, SoftwareSignupArgument, TreeRoot,
    UsernameReservation, ENTITY_PUK_VERIFY, ENTITY_USER,
};
use foks_rpc::{
    encode_check_invite_code_request, encode_check_name_exists_request,
    encode_clear_device_nag_request, encode_get_client_cert_chain_request_at,
    encode_get_client_version_info_request, encode_get_current_merkle_root_hash_request,
    encode_get_current_merkle_root_signed_request, encode_get_device_nag_request,
    encode_get_historical_merkle_roots_request, encode_get_puk_for_role_request,
    encode_load_user_chain_as_local_team_request, encode_load_user_chain_open_host_request,
    encode_load_user_chain_request_from, encode_merkle_check_key_exists_request,
    encode_merkle_lookup_request, encode_merkle_multi_lookup_request,
    encode_merkle_select_vhost_request, encode_probe_key_exists_request,
    encode_provision_device_request, encode_registration_select_vhost_request,
    encode_registration_server_config_request, encode_reserve_username_request_at,
    encode_resolve_username_request, encode_revoke_device_request, encode_signup_request_at,
    encode_user_ping_request,
};
use foks_snowpack::{decode, Value};
use foks_verify::{
    authenticate_historical_roots_from_latest, merkle_history_requirements, normalize_device_name,
    normalize_username, restore_merkle_anchor, restore_public_host_identity, restore_verified_team,
    restore_verified_user, user_chain_root_epochs, verify_non_self_user_chain,
    verify_non_self_user_chain_increment, verify_public_host, verify_signed_merkle_advance,
    verify_user_chain, verify_user_chain_increment, AuthenticatedMerkleRoots, HostService,
    VerifiedMerkleAdvance, VerifiedPublicHost, VerifiedTeamState, VerifiedUserState,
};
use rustls::pki_types::CertificateDer;
use thiserror::Error;
use zeroize::Zeroizing;

pub const DEFAULT_PROBE_PORT: u16 = 4430;
const ADHOC_TEAM_OPERATION_ID_TYPE_ID: u64 = 0x556b_51c0_b659_d1c2;
const ADHOC_TEAM_REQUEST_HASH_TYPE_ID: u64 = 0xc041_ba64_4d2a_161f;
pub(crate) const TEAM_MUTATION_OPERATION_ID_TYPE_ID: u64 = 0x11ad_72e6_d590_82f1;
pub(crate) const TEAM_MUTATION_REQUEST_HASH_TYPE_ID: u64 = 0x4d1f_b849_724a_f9c4;

mod account;
mod auth;
mod device;
mod discovery;
mod error;
mod federation;
mod host;
mod kex;
mod kv;
mod mutation;
mod passphrase;
mod protected_store;
mod recovery;
mod scheduler;
mod team;
mod transport;
mod yubi_account;
mod yubi_management;

pub use account::*;
pub use auth::*;
pub use device::*;
pub use discovery::*;
pub use error::*;
pub use federation::*;
pub use host::*;
pub use kex::*;
pub use kv::*;
pub use mutation::*;
pub use passphrase::*;
pub use protected_store::*;
pub use recovery::*;
pub use scheduler::*;
pub use team::*;
pub use transport::*;
pub use yubi_account::*;
pub use yubi_management::*;

fn fix_device_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(['—', '–'], "-")
        .replace(['‘', '’'], "'")
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| Error::KvResponse("OS randomness unavailable"))?;
    Ok(bytes)
}

fn now_microseconds() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::KvResponse("system clock precedes Unix epoch"))?;
    u64::try_from(elapsed.as_micros())
        .map_err(|_| Error::KvResponse("system clock timestamp overflow"))
}

fn now_milliseconds() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::KvResponse("system clock precedes Unix epoch"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| Error::KvResponse("system clock timestamp overflow"))
}

fn user_key_for_seed<'a>(
    user: &'a VerifiedUserState,
    seed: &SecretSeed,
) -> Result<&'a foks_verify::VerifiedSharedKey> {
    let verify_key = derive_shared_verify_key(seed, ENTITY_PUK_VERIFY)?;
    let mut matches = user
        .shared_keys()
        .iter()
        .filter(|key| key.verify_key == verify_key);
    let key = matches
        .next()
        .ok_or(Error::KeyBinding("seed has no matching verified PUK"))?;
    if matches.next().is_some() {
        return Err(Error::KeyBinding("seed matches more than one verified PUK"));
    }
    Ok(key)
}

fn user_key_history_for_seed<'a>(
    user: &'a VerifiedUserState,
    seed: &SecretSeed,
) -> Result<&'a foks_verify::VerifiedSharedKey> {
    let verify_key = derive_shared_verify_key(seed, ENTITY_PUK_VERIFY)?;
    let mut matches = user
        .shared_key_history()
        .iter()
        .filter(|key| key.verify_key == verify_key);
    let key = matches.next().ok_or(Error::KeyBinding(
        "seed has no matching authenticated PUK history",
    ))?;
    if matches.next().is_some() {
        return Err(Error::KeyBinding(
            "seed matches more than one authenticated PUK generation",
        ));
    }
    Ok(key)
}

fn current_owner_puk(user: &AuthenticatedUserOutcome) -> Result<&UserPrivateKey> {
    let matching = user
        .puks
        .iter()
        .filter(|key| key.role == Role::OWNER)
        .filter(|key| {
            user_key_for_seed(&user.verified, &key.seed)
                .is_ok_and(|public| public.role == key.role && public.generation == key.generation)
        })
        .collect::<Vec<_>>();
    let [owner] = matching.as_slice() else {
        return Err(Error::KeyBinding("expected exactly one current owner PUK"));
    };
    Ok(owner)
}

fn require_nonstale_shared_key(user: &VerifiedUserState, role: foks_proto::Role) -> Result<()> {
    if user.stale_shared_key_roles().contains(&role) {
        return Err(Error::KeyBinding(
            "shared key is still readable by a revoked device",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use crate::kv::{read_kv_upload_chunk, read_kv_upload_chunk_with_carry, KvRequest};
    use foks_client_db::SoftStateStore;
    use foks_proto::{KvListResponse, KvParty};
    use foks_rpc::KvAuth;
    use foks_verify::verify_merkle_advance;

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );
    const USER_ROOT: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-root-998.snowp");
    const USER_HISTORY: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-historical-response.snowp"
    );
    const SIGNUP_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/signup";
    const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

    fn signup_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{SIGNUP_DIR}/{name}")).unwrap()
    }

    fn mutation_fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{MUTATION_DIR}/{name}")).unwrap()
    }

    #[test]
    fn adhoc_team_id_is_predictable_before_submission() {
        let secrets = AdHocTeamSecrets {
            member_min: SecretSeed::new(
                mutation_fixture("adhoc-ptk-member-min-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
            member: SecretSeed::new(
                mutation_fixture("adhoc-ptk-member-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
            admin: SecretSeed::new(
                mutation_fixture("adhoc-ptk-admin-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
            owner: SecretSeed::new(
                mutation_fixture("adhoc-ptk-owner-seed.bin")
                    .try_into()
                    .unwrap(),
            ),
        };
        let expected = match decode(&mutation_fixture("adhoc-team-id.snowp")).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            other => panic!("expected TeamID, got {other:?}"),
        };
        assert_eq!(secrets.team_id().unwrap(), expected);
        assert_ne!(secrets.operation_id().unwrap(), [0; 16]);
    }

    #[test]
    fn software_account_preparation_uses_verified_root_and_exact_name_rules() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &HostchainTail {
                seqno: public.snapshot.chain_seqno(),
                hash: public.snapshot.chain_tail_hash(),
            },
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("hard.sqlite3");
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&public.snapshot)
            .unwrap();
        let client = FoksClient::webpki();
        let host = client.pinned_host("foks.app", &database).unwrap();
        let secrets = SoftwareAccountSecrets::new(
            SecretSeed::new(signup_fixture("device-seed.bin").try_into().unwrap()),
            SecretSeed::new(signup_fixture("puk-seed.bin").try_into().unwrap()),
            signup_fixture("self-token.bin").try_into().unwrap(),
        );
        let prepared = client
            .prepare_software_account(
                &host,
                &advance,
                &SoftwareAccountRequest {
                    username_utf8: "SignupFixture".to_owned(),
                    device_name: "  Signup  Device—One  ".to_owned(),
                    invite_code: InviteCode::Empty,
                    email: "fixture@example.com".to_owned(),
                    passphrase: None,
                },
                UsernameReservation::decode(&signup_fixture("reservation.snowp")).unwrap(),
                &secrets,
            )
            .unwrap();
        assert_eq!(prepared.normalized_username, b"signupfixture");
        assert_eq!(prepared.device_name.display_name, b"Signup Device-One");
        assert_eq!(
            prepared.device_name.label.normalized_name,
            b"signup device-one"
        );
        assert_eq!(prepared.eldest.link.signatures().len(), 2);
        assert_eq!(prepared.eldest.uid.entity_type(), ENTITY_USER);
    }

    #[test]
    fn target_normalizes_name_and_defaults_port() {
        let target = ProbeTarget::parse("FOKS.APP.").unwrap();
        assert_eq!(target.hostname(), "foks.app");
        assert_eq!(target.port(), DEFAULT_PROBE_PORT);
        assert_eq!(target.address(), "foks.app:4430");
    }

    #[test]
    fn target_accepts_explicit_port() {
        let target = ProbeTarget::parse("localhost:9443").unwrap();
        assert_eq!(target.hostname(), "localhost");
        assert_eq!(target.port(), 9443);
    }

    #[test]
    fn target_accepts_ipv6_literals() {
        let target = ProbeTarget::parse("[2001:0db8::1]:9443").unwrap();
        assert_eq!(target.hostname(), "2001:db8::1");
        assert_eq!(target.port(), 9443);
        assert_eq!(target.address(), "[2001:db8::1]:9443");

        let defaulted = ProbeTarget::parse("::1").unwrap();
        assert_eq!(defaulted.hostname(), "::1");
        assert_eq!(defaulted.port(), DEFAULT_PROBE_PORT);
        assert_eq!(defaulted.address(), "[::1]:4430");
    }

    #[test]
    fn target_rejects_ambiguous_or_invalid_names() {
        for target in [
            "",
            ".",
            "a..b",
            "-bad.test",
            "bad-.test",
            "bad name",
            "[::1",
            "[::1]junk",
        ] {
            assert!(ProbeTarget::parse(target).is_err(), "accepted {target:?}");
        }
    }

    #[test]
    fn pinned_capability_uses_only_authenticated_service_endpoints() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("hard.sqlite3");
        let verified = verify_public_host("foks.app", PROBE).unwrap();
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();

        let client = FoksClient::webpki();
        let pinned = client.pinned_host("foks.app", &database).unwrap();
        assert_eq!(pinned.host_id.as_bytes(), verified.snapshot.host_id());
        assert_eq!(
            pinned.registration.address(),
            verified.public_zone.services.registration
        );
        assert_eq!(pinned.user.address(), verified.public_zone.services.user);
        assert_eq!(
            pinned.merkle_query.address(),
            verified.public_zone.services.merkle_query
        );
        assert_eq!(
            pinned.kv_store.address(),
            verified.public_zone.services.kv_store
        );
        assert!(!pinned.tls_ca_certificates.is_empty());
        assert_eq!(
            authenticated_tls_roots(&pinned).unwrap().len(),
            pinned.tls_ca_certificates.len()
        );
    }

    #[test]
    fn modified_service_projection_cannot_create_a_pinned_capability() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("hard.sqlite3");
        let verified = verify_public_host("foks.app", PROBE).unwrap();
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();

        let connection = rusqlite::Connection::open(&database).unwrap();
        let endpoint =
            foks_snowpack::encode(&Value::Text(b"attacker.example:4430".to_vec())).unwrap();
        connection
            .execute(
                "UPDATE host_services SET endpoint_bytes = ?1 WHERE service_type = ?2",
                rusqlite::params![
                    endpoint,
                    i64::try_from(ServiceType::User.protocol_value()).unwrap()
                ],
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            FoksClient::webpki().pinned_host("foks.app", &database),
            Err(Error::HostBinding(
                "stored host projection does not match authenticated evidence"
            ))
        ));
    }

    #[test]
    fn modified_merkle_pin_is_reauthenticated_before_reuse() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("hard.sqlite3");
        let verified = verify_public_host("foks.app", PROBE).unwrap();
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();

        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute("UPDATE merkle_heads SET root_hash = zeroblob(32)", [])
            .unwrap();
        drop(connection);

        assert!(matches!(
            FoksClient::webpki().pinned_host("foks.app", &database),
            Err(Error::Verify(foks_verify::Error::PersistedMerkleEvidence))
        ));
    }

    #[test]
    fn unsigned_merkle_skip_evidence_cannot_become_durable_authority() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &HostchainTail {
                seqno: public.snapshot.chain_seqno(),
                hash: public.snapshot.chain_tail_hash(),
            },
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("hard.sqlite3");
        let mut database = HardStateStore::open(&database_path).unwrap();
        database.accept_verified_host(&public.snapshot).unwrap();
        assert!(matches!(
            database.accept_verified_merkle_root(public.snapshot.host_id(), advance.snapshot()),
            Err(foks_client_db::Error::InvalidSnapshot(
                "signed Merkle evidence is empty"
            ))
        ));
    }

    #[test]
    fn official_kv_transcript_projects_without_network_or_interactivity() {
        let fixture = |name: &str| {
            std::fs::read(format!(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/user/{name}"
            ))
            .unwrap()
        };
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let hard_path = directory.path().join("hard.sqlite3");
        HardStateStore::open(&hard_path)
            .unwrap()
            .accept_verified_host(&public.snapshot)
            .unwrap();
        let client = FoksClient::webpki();
        let host = client.pinned_host("foks.app", &hard_path).unwrap();
        let Value::Binary(team) = decode(&fixture("team-id.snowp")).unwrap() else {
            panic!("team fixture is not binary");
        };
        let party = EntityId::from_bytes(team).unwrap();
        let seed = SecretSeed::new(fixture("team-ptk-member-min-seed.bin").try_into().unwrap());
        let private_keys = [KvPrivateKeyRef {
            role: Role::member(-16_384),
            generation: 1,
            seed: &seed,
        }];
        let token: [u8; 16] = std::array::from_fn(|index| 0x40 + index as u8);
        let listing = KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
        let symlink_request =
            foks_rpc::encode_kv_get_node_request(KvAuth::Team(&token), listing.entries[1].value)
                .unwrap();
        let transcript = VecDeque::from([
            (
                fixture("kv-get-root-request.frame"),
                fixture("kv-root.snowp"),
            ),
            (
                fixture("kv-get-dir-request.frame"),
                fixture("kv-root-dir.snowp"),
            ),
            (fixture("kv-list-request.frame"), fixture("kv-list.snowp")),
            (symlink_request, fixture("kv-symlink-node.snowp")),
            (
                fixture("kv-get-large-node-request.frame"),
                fixture("kv-large-node.snowp"),
            ),
            (
                fixture("kv-get-large-chunk-request.frame"),
                fixture("kv-large-chunk.snowp"),
            ),
        ]);
        let mut transcript = transcript;
        let soft_path = directory.path().join("soft.sqlite3");
        let projections = client
            .sync_kv_with_fetch(
                &host,
                KvParty {
                    party: party.clone(),
                    host: host.host_id.clone(),
                },
                KvAuth::Team(&token),
                &private_keys,
                &soft_path,
                |auth, request| {
                    if matches!(request, KvRequest::CacheCheck(_)) {
                        return Ok(Vec::new());
                    }
                    let request = request.encode(auth, 1)?;
                    let (expected, response) = transcript
                        .pop_front()
                        .ok_or(Error::KvResponse("unexpected fixture request"))?;
                    if request != expected {
                        return Err(Error::KvResponse("fixture request mismatch"));
                    }
                    Ok(response)
                },
            )
            .unwrap();
        assert!(transcript.is_empty());
        assert_eq!(projections.len(), 1);
        assert_eq!(projections[0].entries.len(), 3);
        assert_eq!(
            projections[0].entries[0].large_file_size,
            Some(fixture("kv-large-plaintext.bin").len() as u64)
        );
        assert!(projections[0].entries[0].content.is_none());
        assert_eq!(
            projections[0].entries[2].content.as_deref(),
            Some(fixture("kv-small-plaintext.bin").as_slice())
        );
        let large_node = projections[0].entries[0].node_id;
        let store = SoftStateStore::open(&soft_path).unwrap();
        let stored = store
            .directory(
                host.host_id.as_bytes(),
                party.as_bytes(),
                &projections[0].directory_id,
            )
            .unwrap();
        assert_eq!(stored, Some(projections.into_iter().next().unwrap()));
        let mut streamed = Vec::new();
        let size = store
            .write_large_file(
                host.host_id.as_bytes(),
                party.as_bytes(),
                &large_node,
                &mut streamed,
            )
            .unwrap();
        assert_eq!(size, Some(streamed.len() as u64));
        assert_eq!(streamed, fixture("kv-large-plaintext.bin"));

        let listing = KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
        let symlink_request =
            foks_rpc::encode_kv_get_node_request(KvAuth::Team(&token), listing.entries[1].value)
                .unwrap();
        let mut cached_transcript = VecDeque::from([
            (
                fixture("kv-get-root-request.frame"),
                fixture("kv-root.snowp"),
            ),
            (
                fixture("kv-get-dir-request.frame"),
                fixture("kv-root-dir.snowp"),
            ),
            (fixture("kv-list-request.frame"), fixture("kv-list.snowp")),
            (symlink_request, fixture("kv-symlink-node.snowp")),
            (
                fixture("kv-get-large-node-request.frame"),
                fixture("kv-large-node.snowp"),
            ),
            (
                fixture("kv-get-large-chunk-request.frame"),
                fixture("kv-large-chunk.snowp"),
            ),
        ]);
        let mut cache_checks = 0;
        let cached = client
            .sync_kv_with_fetch(
                &host,
                KvParty {
                    party: party.clone(),
                    host: host.host_id.clone(),
                },
                KvAuth::Team(&token),
                &private_keys,
                &soft_path,
                |auth, request| {
                    if matches!(request, KvRequest::CacheCheck(_)) {
                        cache_checks += 1;
                        return Ok(Vec::new());
                    }
                    let request = request.encode(auth, 1)?;
                    let (expected, response) = cached_transcript
                        .pop_front()
                        .ok_or(Error::KvResponse("unexpected cached fixture request"))?;
                    if request != expected {
                        return Err(Error::KvResponse("cached fixture request mismatch"));
                    }
                    Ok(response)
                },
            )
            .unwrap();
        assert!(cached_transcript.is_empty());
        assert_eq!(cache_checks, 1);
        assert_eq!(cached, stored.into_iter().collect::<Vec<_>>());
    }

    #[test]
    fn upload_reader_keeps_only_one_chunk_and_one_byte_of_lookahead() {
        let exact_small_bytes = vec![7; KvWriteSession::SMALL_FILE_BYTES];
        let mut exact_small = exact_small_bytes.as_slice();
        let (small, carry, final_chunk) = read_kv_upload_chunk(&mut exact_small).unwrap();
        assert_eq!(small.len(), KvWriteSession::SMALL_FILE_BYTES);
        assert!(carry.is_empty());
        assert!(final_chunk);

        let input = vec![9; KvWriteSession::MAX_UPLOAD_CHUNK + 1];
        let mut reader = input.as_slice();
        let (first, carry, final_chunk) = read_kv_upload_chunk(&mut reader).unwrap();
        assert_eq!(first.len(), KvWriteSession::MAX_UPLOAD_CHUNK);
        assert_eq!(carry, [9]);
        assert!(!final_chunk);
        let (last, carry, final_chunk) =
            read_kv_upload_chunk_with_carry(&mut reader, carry).unwrap();
        assert_eq!(last, [9]);
        assert!(carry.is_empty());
        assert!(final_chunk);
    }
}

#[cfg(test)]
#[path = "../tests/authenticated_user.rs"]
mod authenticated_user_tests;
