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
    make_software_puk_rotation_link, make_software_revoke_link, open_puk_parcel_with_for_role,
    open_puk_seed_chain, prefixed_hash, seal_initial_puk_box, seal_puk_seed_chain_box,
    seal_software_puk_boxes, DevicePublicMaterial, InitialPukBoxRandomness, PukBoxRandomness,
    PukRotation, SoftwareEldestInput, SoftwareEldestMaterial, SoftwareProvisionInput,
    SoftwarePukBoxInput, UserMutationBase, YubiDevice,
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
    normalize_username, restore_local_merkle_checkpoint, restore_public_host_identity,
    restore_verified_team, restore_verified_user, user_chain_root_epochs,
    verify_non_self_user_chain, verify_non_self_user_chain_increment, verify_public_host,
    verify_signed_merkle_advance, verify_user_chain, verify_user_chain_increment,
    AuthenticatedMerkleRoots, HostService, VerifiedMerkleAdvance, VerifiedPublicHost,
    VerifiedTeamState, VerifiedUserState,
};
use rustls::pki_types::CertificateDer;
use thiserror::Error;
use zeroize::Zeroizing;

pub const DEFAULT_PROBE_PORT: u16 = 4430;
const ADHOC_TEAM_OPERATION_ID_TYPE_ID: u64 = 0x556b_51c0_b659_d1c2;
const ADHOC_TEAM_REQUEST_HASH_TYPE_ID: u64 = 0xc041_ba64_4d2a_161f;
pub(crate) const TEAM_MUTATION_OPERATION_ID_TYPE_ID: u64 = 0x11ad_72e6_d590_82f1;
pub(crate) const TEAM_MUTATION_REQUEST_HASH_TYPE_ID: u64 = 0x4d1f_b849_724a_f9c4;

mod account_conveniences;
mod bot_token;
mod change_marker;
mod web_admin;
pub use account_conveniences::*;
pub use web_admin::{AdminDestination, WebAdminError, WebAdminHandoff};
mod account;
mod sso;
pub use sso::{SsoIntent, SsoProgress, SsoSigningKey, SsoSignupAuthorization};
mod auth;
mod device;
mod discovery;
mod error;
mod federation;
mod host;
mod host_cache;
pub use host_cache::{clear_cached_hosts, host_replay_count};
mod kex;
mod kv;
mod mutation;
mod passphrase;
mod pinning;
mod protected_inventory;
mod protected_store;
pub use protected_inventory::*;
mod realtime;
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
pub use realtime::*;
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
    getrandom::fill(&mut bytes).map_err(|_| Error::Crypto(foks_crypto::Error::Entropy))?;
    Ok(bytes)
}

fn now_microseconds() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Transport("system clock precedes Unix epoch"))?;
    u64::try_from(elapsed.as_micros())
        .map_err(|_| Error::Transport("system clock timestamp overflow"))
}

fn now_milliseconds() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Transport("system clock precedes Unix epoch"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| Error::Transport("system clock timestamp overflow"))
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

    use crate::kv::{
        read_kv_chunk_with_fetch, read_kv_node_with_fetch, read_kv_upload_chunk,
        read_kv_upload_chunk_with_carry, KvRequest,
    };
    use foks_client_db::SoftStateStore;
    use foks_proto::{KvListResponse, KvNode, KvParty};
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

        // Restore once before tampering, so the projection under test is the
        // cached one rather than a cold read.
        FoksClient::webpki()
            .pinned_host("foks.app", &database)
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

        // Restore once before tampering, so the anchor under test is the
        // cached one rather than a cold read.
        FoksClient::webpki()
            .pinned_host("foks.app", &database)
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

    /// Serializes the tests that clear the process-wide host cache, so one
    /// cannot drop an entry another has just asserted.
    fn cache_tests() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Accepts the fixture host into a fresh database and returns its path.
    fn pinned_fixture_host(directory: &std::path::Path) -> std::path::PathBuf {
        let database = directory.join("hard.sqlite3");
        let verified = verify_public_host("foks.app", PROBE).unwrap();
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();
        database
    }

    #[test]
    fn a_restored_host_is_cached_under_the_revision_it_was_read_at() {
        let _guard = cache_tests();
        let directory = tempfile::tempdir().unwrap();
        let database = pinned_fixture_host(directory.path());
        let metadata = HardStateStore::open(&database).unwrap().metadata().unwrap();
        let key = host_cache::CacheKey::new(&database, "foks.app", metadata);

        clear_cached_hosts();
        assert!(host_cache::get(&key).is_none());
        FoksClient::webpki()
            .pinned_host("foks.app", &database)
            .unwrap();
        assert!(host_cache::get(&key).is_some());
    }

    #[test]
    fn a_cached_host_equals_the_projection_a_fresh_replay_produces() {
        let _guard = cache_tests();
        let directory = tempfile::tempdir().unwrap();
        let database = pinned_fixture_host(directory.path());
        let client = FoksClient::webpki();

        let first = client.pinned_host("foks.app", &database).unwrap();
        let cached = client.pinned_host("foks.app", &database).unwrap();
        clear_cached_hosts();
        let replayed = client.pinned_host("foks.app", &database).unwrap();

        assert_eq!(first, cached);
        assert_eq!(first, replayed);
    }

    #[test]
    fn two_discovery_names_for_one_database_do_not_share_a_cached_host() {
        let directory = tempfile::tempdir().unwrap();
        let database = pinned_fixture_host(directory.path());
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute(
                "INSERT INTO host_lookups (lookup_name, host_id)
                 SELECT 'alias.test', host_id FROM host_lookups WHERE lookup_name = 'foks.app'",
                [],
            )
            .unwrap();
        drop(connection);
        let client = FoksClient::webpki();

        let probed = client.pinned_host("foks.app", &database).unwrap();
        let alias = client.pinned_host("alias.test", &database).unwrap();

        // One database, one host identity, two discovery names. A cache keyed
        // without the name would answer the second request with the first
        // name, and every later resolution would follow it.
        assert_eq!(probed.lookup_name(), "foks.app");
        assert_eq!(alias.lookup_name(), "alias.test");
        assert_eq!(probed.host_id(), alias.host_id());
    }

    #[test]
    fn two_spellings_of_one_database_do_not_share_a_cached_host() {
        let directory = tempfile::tempdir().unwrap();
        let database = pinned_fixture_host(directory.path());
        // Path equality folds away a `.` component but keeps `..`, so this is
        // a spelling that reaches the same file and compares unequal, which is
        // exactly what the advance-and-accept exclusion keys on.
        std::fs::create_dir(directory.path().join("sub")).unwrap();
        let indirect = directory.path().join("sub").join("..").join("hard.sqlite3");
        let client = FoksClient::webpki();

        let direct_host = client.pinned_host("foks.app", &database).unwrap();
        let indirect_host = client.pinned_host("foks.app", &indirect).unwrap();

        // The advance-and-accept exclusion is keyed on the caller's spelling,
        // so a capability must carry the path it was asked for.
        assert_eq!(direct_host.database_path, database);
        assert_eq!(indirect_host.database_path, indirect);
    }

    #[test]
    fn a_write_to_a_host_table_is_seen_through_a_cached_host() {
        let directory = tempfile::tempdir().unwrap();
        let database = pinned_fixture_host(directory.path());
        let client = FoksClient::webpki();
        client.pinned_host("foks.app", &database).unwrap();

        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute("UPDATE hosts SET canonical_name = 'attacker.example'", [])
            .unwrap();
        drop(connection);

        assert!(matches!(
            client.pinned_host("foks.app", &database),
            Err(Error::HostBinding(
                "stored host projection does not match authenticated evidence"
            ))
        ));
    }

    #[test]
    fn a_write_that_changes_nothing_still_drops_a_cached_host() {
        let directory = tempfile::tempdir().unwrap();
        let database = pinned_fixture_host(directory.path());
        let client = FoksClient::webpki();
        let before = client.pinned_host("foks.app", &database).unwrap();
        let revision = HardStateStore::open(&database).unwrap().metadata().unwrap();

        // An update that writes each row's existing bytes back. The trigger
        // fires on the statement, not on a difference, so the revision moves
        // and the cache drops its entry. That is the deliberate direction: the
        // key over-invalidates rather than reasoning about what a write meant.
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute("UPDATE merkle_roots SET root_bytes = root_bytes", [])
            .unwrap();
        drop(connection);
        assert_ne!(
            HardStateStore::open(&database).unwrap().metadata().unwrap(),
            revision
        );

        // The bytes did not change, so the restored projection is the same.
        assert_eq!(client.pinned_host("foks.app", &database).unwrap(), before);
    }

    #[test]
    fn re_accepting_an_unchanged_host_keeps_the_cached_projection() {
        let directory = tempfile::tempdir().unwrap();
        let database = pinned_fixture_host(directory.path());
        FoksClient::webpki()
            .pinned_host("foks.app", &database)
            .unwrap();
        let revision = HardStateStore::open(&database).unwrap().metadata().unwrap();

        let verified = verify_public_host("foks.app", PROBE).unwrap();
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();

        // Accepting a host whose chain and projection are unchanged writes
        // nothing, so no trigger fires and the cached projection survives.
        assert_eq!(
            HardStateStore::open(&database).unwrap().metadata().unwrap(),
            revision
        );
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
    fn only_a_missing_root_is_an_empty_namespace() {
        let fixture = |name: &str| {
            std::fs::read(format!(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/user/{name}"
            ))
            .unwrap()
        };
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let hard_path = temporary.path().join("hard.sqlite3");
        HardStateStore::open(&hard_path)
            .unwrap()
            .accept_verified_host(&public.snapshot)
            .unwrap();
        let client = FoksClient::webpki();
        let host = client.pinned_host("foks.app", &hard_path).unwrap();
        let Value::Binary(team) = decode(&fixture("team-id.snowp")).unwrap() else {
            panic!("invalid team fixture");
        };
        let party = KvParty {
            party: EntityId::from_bytes(team).unwrap(),
            host: host.host_id.clone(),
        };
        let seed = SecretSeed::new(fixture("team-ptk-member-min-seed.bin").try_into().unwrap());
        let keys = [KvPrivateKeyRef {
            role: Role::member(-16_384),
            generation: 1,
            seed: &seed,
        }];
        let remote = |status: foks_rpc::RpcStatus| {
            let frame = foks_rpc::encode_status_response_at(&status, 1).unwrap();
            Error::Rpc(
                foks_rpc::read_response(
                    &mut std::io::Cursor::new(frame),
                    foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
                    1,
                )
                .unwrap_err(),
            )
        };
        for metadata in [false, true] {
            for (name, code, missing_child) in [
                ("missing", 8016, false),
                ("denied", 8011, false),
                ("child", 8016, true),
            ] {
                let path = temporary.path().join(format!("{metadata}-{name}.sqlite3"));
                let mut calls = 0;
                let fetch = |_: KvAuth<'_>, request: &KvRequest| {
                    calls += 1;
                    if missing_child && matches!(request, KvRequest::Root) {
                        Ok(fixture("kv-root.snowp"))
                    } else if code == 8016 {
                        Err(remote(foks_rpc::RpcStatus::KvNoEnt))
                    } else {
                        Err(remote(foks_rpc::RpcStatus::KvPermission {
                            operation: 1,
                            resource: 1,
                        }))
                    }
                };
                let result = if metadata {
                    client.list_kv_metadata_with_fetch(
                        &host,
                        party.clone(),
                        KvAuth::User,
                        &keys,
                        &path,
                        fetch,
                    )
                } else {
                    client.sync_kv_with_fetch(
                        &host,
                        party.clone(),
                        KvAuth::User,
                        &keys,
                        &path,
                        fetch,
                    )
                };
                if code == 8016 && !missing_child {
                    assert!(result.unwrap().is_empty());
                    assert_eq!(calls, 1);
                } else {
                    assert!(
                        matches!(result, Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: actual, .. })) if actual == code)
                    );
                    assert_eq!(calls, if missing_child { 2 } else { 1 });
                }
                assert!(!SoftStateStore::open(&path)
                    .unwrap()
                    .has_kv_root(party.host.as_bytes(), party.party.as_bytes())
                    .unwrap());
            }
        }
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
        let wrong_owner_seed = SecretSeed::new([0x97; 32]);
        let private_keys = [
            KvPrivateKeyRef {
                role: Role::member(-16_384),
                generation: 1,
                seed: &seed,
            },
            KvPrivateKeyRef {
                role: Role::OWNER,
                generation: 1,
                seed: &wrong_owner_seed,
            },
        ];
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
        assert!(matches!(
            KvNode::decode(
                projections[0].entries[2]
                    .node_bytes
                    .as_deref()
                    .expect("content projection retains small-file metadata")
            )
            .unwrap(),
            KvNode::SmallFile(_)
        ));
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
    fn catalog_traversal_authenticates_but_does_not_stage_file_plaintext() {
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
        let root = foks_proto::KvRoot::decode(&fixture("kv-root.snowp")).unwrap();
        let metadata_list_request = KvRequest::List {
            directory: root.root,
            cursor: foks_rpc::KvListCursor::None,
            number: foks_proto::MAXIMUM_KV_LIST_PAGE_ENTRIES as u64,
            load_small_files: true,
        }
        .encode(KvAuth::Team(&token), 1)
        .unwrap();
        let listing = KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
        let symlink_request =
            foks_rpc::encode_kv_get_node_request(KvAuth::Team(&token), listing.entries[1].value)
                .unwrap();
        let mut transcript = VecDeque::from([
            (
                fixture("kv-get-root-request.frame"),
                fixture("kv-root.snowp"),
            ),
            (
                fixture("kv-get-dir-request.frame"),
                fixture("kv-root-dir.snowp"),
            ),
            (metadata_list_request.clone(), fixture("kv-list.snowp")),
            (symlink_request, fixture("kv-symlink-node.snowp")),
            (
                fixture("kv-get-large-node-request.frame"),
                fixture("kv-large-node.snowp"),
            ),
        ]);
        let catalog_soft_path = directory.path().join("catalog-soft.sqlite3");
        let metadata = client
            .list_kv_metadata_with_fetch(
                &host,
                KvParty {
                    party: party.clone(),
                    host: host.host_id.clone(),
                },
                KvAuth::Team(&token),
                &private_keys,
                &catalog_soft_path,
                |auth, request| {
                    if matches!(request, KvRequest::CacheCheck(_)) {
                        return Ok(Vec::new());
                    }
                    let request = request.encode(auth, 1)?;
                    let (expected, response) = transcript
                        .pop_front()
                        .ok_or(Error::KvResponse("unexpected catalog fixture request"))?;
                    if request != expected {
                        return Err(Error::KvResponse("catalog fixture request mismatch"));
                    }
                    Ok(response)
                },
            )
            .unwrap();
        assert!(
            transcript.is_empty(),
            "catalog fetched an unplanned payload"
        );
        assert_eq!(metadata.len(), 1);
        assert!(metadata[0].entries.iter().all(|entry| {
            entry.content.is_none()
                && entry.symlink.is_none()
                && entry.large_file_size.is_none()
                && entry.node_bytes.is_some()
        }));
        assert!(
            SoftStateStore::open(&catalog_soft_path)
                .unwrap()
                .version_vector(&metadata[0].host_id, &metadata[0].party_id)
                .unwrap()
                .is_some(),
            "metadata traversal did not durably pin its authenticated version vector"
        );

        let mut tampered = KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
        tampered.extended[0].small_file.key.role = Role::OWNER;
        let tampered =
            KvListResponse::new(tampered.entries, tampered.final_page, tampered.extended)
                .unwrap()
                .encoded()
                .to_vec();
        let mut tampered_transcript = VecDeque::from([
            (
                fixture("kv-get-root-request.frame"),
                fixture("kv-root.snowp"),
            ),
            (
                fixture("kv-get-dir-request.frame"),
                fixture("kv-root-dir.snowp"),
            ),
            (metadata_list_request, tampered),
        ]);
        let result = client.list_kv_metadata_with_fetch(
            &host,
            KvParty {
                party,
                host: host.host_id.clone(),
            },
            KvAuth::Team(&token),
            &private_keys,
            &directory.path().join("tampered-catalog-soft.sqlite3"),
            |auth, request| {
                let request = request.encode(auth, 1)?;
                let (expected, response) = tampered_transcript
                    .pop_front()
                    .ok_or(Error::KvResponse("unexpected tampered catalog request"))?;
                if request != expected {
                    return Err(Error::KvResponse(
                        "tampered catalog fixture request mismatch",
                    ));
                }
                Ok(response)
            },
        );
        assert!(result.is_err(), "tampered read-role header was accepted");
        assert!(tampered_transcript.is_empty());
    }

    #[test]
    fn single_node_reads_fetch_only_the_selected_content() {
        let fixture = |name: &str| {
            std::fs::read(format!(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/user/{name}"
            ))
            .unwrap()
        };
        let seed = SecretSeed::new(fixture("team-ptk-member-min-seed.bin").try_into().unwrap());
        let private_keys = [KvPrivateKeyRef {
            role: Role::member(-16_384),
            generation: 1,
            seed: &seed,
        }];
        let token: [u8; 16] = std::array::from_fn(|index| 0x40 + index as u8);
        let listing = KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
        let small = listing
            .extended
            .iter()
            .find(|extended| {
                listing.entries[extended.position as usize]
                    .value
                    .node_type()
                    .unwrap()
                    == foks_proto::KvNodeType::SmallFile
            })
            .unwrap();
        let small_id = listing.entries[small.position as usize].value;
        let small_node = KvNode::SmallFile(small.small_file.clone())
            .encoded()
            .unwrap();
        let mut requests = Vec::new();
        let fetched = read_kv_node_with_fetch(
            small_id,
            &private_keys,
            KvAuth::Team(&token),
            |_, request| {
                requests.push(request.clone());
                Ok(small_node.clone())
            },
        )
        .unwrap();
        assert_eq!(
            fetched,
            KvFetchedNode::SmallFile(fixture("kv-small-plaintext.bin"))
        );
        assert!(matches!(requests.as_slice(), [KvRequest::Node(id)] if *id == small_id));

        let large_id = listing
            .entries
            .iter()
            .find(|entry| entry.value.node_type().unwrap() == foks_proto::KvNodeType::File)
            .unwrap()
            .value;
        let large_node = fixture("kv-large-node.snowp");
        let large_chunk = fixture("kv-large-chunk.snowp");
        // Reading a large-file node costs exactly one request. It reports no
        // size, so it does not walk the chunk chain to measure one, which is
        // what made a read of an N-byte file cost N bytes before the caller's
        // own read had started.
        let mut inspect_requests =
            VecDeque::from([(KvRequest::Node(large_id), large_node.clone())]);
        let inspected = read_kv_node_with_fetch(
            large_id,
            &private_keys,
            KvAuth::Team(&token),
            |_, request| {
                let (expected, response) = inspect_requests.pop_front().unwrap();
                assert_eq!(
                    request.encode(KvAuth::Team(&token), 1).unwrap(),
                    expected.encode(KvAuth::Team(&token), 1).unwrap()
                );
                Ok(response)
            },
        )
        .unwrap();
        assert_eq!(inspected, KvFetchedNode::LargeFile { size: None });
        assert!(inspect_requests.is_empty());

        let mut requests = VecDeque::from([
            (KvRequest::Node(large_id), large_node),
            (
                KvRequest::Chunk {
                    file: large_id,
                    offset: 0,
                },
                large_chunk,
            ),
        ]);
        let fetched = read_kv_chunk_with_fetch(
            large_id,
            1,
            3,
            &private_keys,
            KvAuth::Team(&token),
            |_, request| {
                let (expected, response) = requests.pop_front().unwrap();
                assert_eq!(
                    request.encode(KvAuth::Team(&token), 1).unwrap(),
                    expected.encode(KvAuth::Team(&token), 1).unwrap()
                );
                Ok(response)
            },
        )
        .unwrap();
        assert_eq!(fetched.content, fixture("kv-large-plaintext.bin")[1..4]);
        assert!(!fetched.eof);
        assert!(requests.is_empty());
        // Go's range query returns the containing stored chunk (offset zero),
        // even when the requested offset lies inside it.
        let ranged = read_kv_chunk_with_fetch(
            large_id,
            4,
            3,
            &private_keys,
            KvAuth::Team(&token),
            |_, request| match request {
                KvRequest::Node(_) => Ok(fixture("kv-large-node.snowp")),
                KvRequest::Chunk { offset, .. } => {
                    assert_eq!(*offset, 4);
                    Ok(fixture("kv-large-chunk.snowp"))
                }
                _ => panic!("unexpected range request"),
            },
        )
        .unwrap();
        assert_eq!(ranged.content, fixture("kv-large-plaintext.bin")[4..7]);
        let future = read_kv_chunk_with_fetch(
            large_id,
            4,
            3,
            &private_keys,
            KvAuth::Team(&token),
            |_, request| {
                if matches!(request, KvRequest::Node(_)) {
                    return Ok(fixture("kv-large-node.snowp"));
                }
                let mut chunk =
                    foks_proto::KvEncryptedChunk::decode(&fixture("kv-large-chunk.snowp")).unwrap();
                chunk.offset = 5;
                Ok(chunk.encode().unwrap())
            },
        );
        assert!(
            future.is_err(),
            "a future chunk cannot answer an earlier range"
        );
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
