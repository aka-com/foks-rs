//! The resolved-path cache a chunked read consults, against a live server.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use foks_client_app::{
    derive_vault_key, AccountVault, AuthCacheKey, AuthenticatedUserCache, ClientCredentials,
    CredentialBackend, KvMutationPrecondition, KvNodeCache, KvNodeCacheEntry, KvNodeCacheKey,
    KvRoleSummary, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

/// A cache that records what the read path asked of it. It is otherwise the
/// bounded store the agent keeps: one entry per key, replaced on `put`.
#[derive(Default)]
struct RecordingCache {
    entries: Mutex<Vec<(KvNodeCacheKey, KvNodeCacheEntry)>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
    invalidations: AtomicUsize,
}

impl KvNodeCache for RecordingCache {
    fn get(&self, key: &KvNodeCacheKey) -> Option<KvNodeCacheEntry> {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, entry)| entry.clone());
        if found.is_some() {
            self.hits.fetch_add(1, Ordering::AcqRel);
        } else {
            self.misses.fetch_add(1, Ordering::AcqRel);
        }
        found
    }

    fn put(&self, key: KvNodeCacheKey, entry: KvNodeCacheEntry) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|(stored, _)| *stored != key);
        entries.push((key, entry));
    }

    fn invalidate(&self, key: &KvNodeCacheKey) {
        self.invalidations.fetch_add(1, Ordering::AcqRel);
        self.entries
            .lock()
            .unwrap()
            .retain(|(stored, _)| stored != key);
    }

    fn invalidate_profile(&self, _state_root: &std::path::Path, _profile: &str) {
        self.entries.lock().unwrap().clear();
    }
}

/// The authenticated-user cache the agent also attaches. Without it every
/// read re-authenticates, which would swamp the request counts this test
/// compares.
#[derive(Default)]
struct HoldingAuth {
    entries: Mutex<Vec<(AuthCacheKey, Arc<foks_client::AuthenticatedUserOutcome>)>>,
}

impl AuthenticatedUserCache for HoldingAuth {
    fn get(&self, key: &AuthCacheKey) -> Option<Arc<foks_client::AuthenticatedUserOutcome>> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, outcome)| Arc::clone(outcome))
    }

    fn put(&self, key: AuthCacheKey, outcome: Arc<foks_client::AuthenticatedUserOutcome>) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|(stored, _)| *stored != key);
        entries.push((key, outcome));
    }

    fn invalidate_profile(&self, _state_root: &std::path::Path, _profile: &str) {
        self.entries.lock().unwrap().clear();
    }
}

/// Three properties of the cache, in the order a download meets them: the
/// first chunk walks the path and remembers it, a later chunk reads under the
/// remembered node for fewer server requests, and an entry replaced by a peer
/// fails the read rather than being served from the node the walk found.
#[test]
fn a_chunk_read_remembers_its_path_and_refuses_a_replaced_entry() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let root = state.join("root.der");
    environment.write_probe_root(&root).unwrap();
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            label: None,
            probe: format!(
                "localhost:{}",
                environment.addresses().unwrap().probe.port()
            ),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let cache = Arc::new(RecordingCache::default());
    let session = ProfileSession::open(&registry, "local")
        .unwrap()
        .with_authenticated_user_cache(Arc::new(HoldingAuth::default()))
        .with_kv_node_cache(cache.clone());
    let credentials = ClientCredentials::open(&state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            session.probe_and_pin()?;
            let master = credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "cacheowner",
                "device",
                "cache@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            let data = vec![0x61; 300 * 1024];
            session.put_kv_file_checked(
                "owner",
                "/nested/large",
                &mut data.as_slice(),
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                true,
                &mut vault,
                &master,
            )?;
            // A second write so the entry the cache remembers is at a version
            // a re-created dirent cannot reach: a re-create restarts at 1.
            let written = session.put_kv_file_checked(
                "owner",
                "/nested/large",
                &mut data.as_slice(),
                KvMutationPrecondition::ExactVersion(1),
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                false,
                &mut vault,
                &master,
            )?;
            let selected = session
                .list_kv_metadata("owner", &mut vault)?
                .entries
                .into_iter()
                .find(|e| e.path == "/nested/large")
                .unwrap();
            let dirent_id = selected.dirent_id;
            let version = written.version;
            assert_eq!(version, 2);

            let before = server.metrics().requests_started;
            let first = session.read_kv_chunk(
                "owner",
                "/nested/large",
                dirent_id,
                version,
                0,
                64 * 1024,
                &mut vault,
            )?;
            let walked = server.metrics().requests_started - before;
            assert_eq!(first.content, data[..64 * 1024]);
            assert_eq!(cache.misses.load(Ordering::Acquire), 1);
            assert_eq!(cache.entries.lock().unwrap().len(), 1);

            let before = server.metrics().requests_started;
            let second = session.read_kv_chunk(
                "owner",
                "/nested/large",
                dirent_id,
                version,
                64 * 1024,
                64 * 1024,
                &mut vault,
            )?;
            let remembered = server.metrics().requests_started - before;
            assert_eq!(second.content, data[64 * 1024..128 * 1024]);
            assert_eq!(cache.hits.load(Ordering::Acquire), 1);
            assert_eq!(cache.invalidations.load(Ordering::Acquire), 0);
            println!("read_kv_chunk requests: walked={walked} remembered={remembered}");
            assert!(
                remembered * 2 <= walked,
                "walked={walked} remembered={remembered}"
            );

            // A peer unlinks the entry and writes another file at the same
            // name. The new dirent is a fresh identifier at version 1, so the
            // cache still holds an entry for the version asked for here.
            session.remove_kv("owner", "/nested/large", false, &mut vault, &master)?;
            session.put_kv_file_checked(
                "owner",
                "/nested/large",
                &mut data.as_slice(),
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                false,
                &mut vault,
                &master,
            )?;
            assert_eq!(cache.entries.lock().unwrap().len(), 1);
            let refused = session.read_kv_chunk(
                "owner",
                "/nested/large",
                dirent_id,
                version,
                64 * 1024,
                64 * 1024,
                &mut vault,
            );
            assert!(
                refused.is_err(),
                "a replaced entry was served from the remembered node"
            );
            // The refusal came from the version vector, not from the key: the
            // cache was consulted, found stale, and dropped.
            assert_eq!(cache.hits.load(Ordering::Acquire), 2);
            assert_eq!(cache.invalidations.load(Ordering::Acquire), 1);
            assert!(cache.entries.lock().unwrap().is_empty());
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
}

#[test]
fn selected_identity_survives_user_and_team_cache_fallbacks() {
    let environment = TestEnvironment::new().unwrap();
    let _server = environment.start_server().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let root = state.join("root.der");
    environment.write_probe_root(&root).unwrap();
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            label: None,
            probe: format!(
                "localhost:{}",
                environment.addresses().unwrap().probe.port()
            ),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let cache = Arc::new(RecordingCache::default());
    let session = ProfileSession::open(&registry, "local")
        .unwrap()
        .with_authenticated_user_cache(Arc::new(HoldingAuth::default()))
        .with_kv_node_cache(cache.clone());
    let credentials = ClientCredentials::open(&state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            session.probe_and_pin()?;
            let master = credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "boundowner",
                "device",
                "cache@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;

            let team_id = session
                .create_named_team("owner", "shared", "boundteam", &mut vault, &master)?
                .team_id_hex;
            for team in [None, Some(team_id.as_str())] {
                let put =
                    |path: &str, mut bytes: &[u8], precondition, vault: &mut AccountVault<'_>| {
                        match team {
                            None => session.put_kv_file_checked(
                                "owner",
                                path,
                                &mut bytes,
                                precondition,
                                KvRoleSummary::Owner,
                                KvRoleSummary::Owner,
                                false,
                                vault,
                                &master,
                            ),
                            Some(id) => session.put_team_kv_file_checked(
                                "owner",
                                "shared",
                                id,
                                path,
                                &mut bytes,
                                precondition,
                                KvRoleSummary::Owner,
                                KvRoleSummary::Owner,
                                false,
                                vault,
                                &master,
                            ),
                        }
                    };
                let selected = |path: &str, vault: &mut AccountVault<'_>| {
                    let catalog = match team {
                        None => session.list_kv_metadata("owner", vault),
                        Some(id) => session.list_team_kv_metadata("owner", "shared", id, vault),
                    }?;
                    Ok::<_, foks_client_app::Error>(
                        catalog
                            .entries
                            .into_iter()
                            .find(|entry| entry.path == path)
                            .unwrap(),
                    )
                };
                let read = |entry: &foks_client_app::KvCatalogEntry,
                            vault: &mut AccountVault<'_>| {
                    match team {
                        None => session.read_kv_entry(
                            "owner",
                            &entry.path,
                            entry.dirent_id,
                            entry.version,
                            vault,
                        ),
                        Some(id) => session.read_team_kv_entry(
                            "owner",
                            "shared",
                            id,
                            &entry.path,
                            entry.dirent_id,
                            entry.version,
                            vault,
                        ),
                    }
                };
                let chunk = |entry: &foks_client_app::KvCatalogEntry,
                             offset,
                             vault: &mut AccountVault<'_>| {
                    match team {
                        None => session.read_kv_chunk(
                            "owner",
                            &entry.path,
                            entry.dirent_id,
                            entry.version,
                            offset,
                            64 * 1024,
                            vault,
                        ),
                        Some(id) => session.read_team_kv_chunk(
                            "owner",
                            "shared",
                            id,
                            &entry.path,
                            entry.dirent_id,
                            entry.version,
                            offset,
                            64 * 1024,
                            vault,
                        ),
                    }
                };
                let old = vec![0x61; 128 * 1024];
                let new = vec![0x62; old.len()];
                for mode in ["miss", "warm", "expired"] {
                    for before_initial in [false, true] {
                        let path = format!("/{mode}-{before_initial}");
                        put(&path, &old, KvMutationPrecondition::Create, &mut vault)?;
                        let original = selected(&path, &mut vault)?;
                        assert_eq!(original.version, 1);
                        if !before_initial {
                            assert_eq!(read(&original, &mut vault)?.dirent_id, original.dirent_id);
                        }
                        if mode != "miss" {
                            let first = chunk(&original, 0, &mut vault)?;
                            assert_eq!(first.content, old[..64 * 1024]);
                            assert_eq!(first.dirent_id, original.dirent_id);
                            // A warm unchanged read must remain bound and complete.
                            let last = chunk(&original, 64 * 1024, &mut vault)?;
                            assert_eq!(last.content, old[64 * 1024..]);
                            assert!(last.eof);
                        }
                        match team {
                            None => {
                                session.remove_kv("owner", &path, false, &mut vault, &master)?
                            }
                            Some(id) => session.remove_team_kv_checked(
                                "owner",
                                "shared",
                                id,
                                &path,
                                false,
                                original.version,
                                &mut vault,
                                &master,
                            )?,
                        };
                        put(&path, &new, KvMutationPrecondition::Create, &mut vault)?;
                        let replacement = selected(&path, &mut vault)?;
                        assert_eq!(replacement.version, original.version);
                        assert_ne!(replacement.dirent_id, original.dirent_id);
                        if mode == "expired" {
                            // Deterministic expiration: the cache returns no retained entry.
                            cache.entries.lock().unwrap().clear();
                        }
                        assert!(
                            matches!(
                                read(&original, &mut vault),
                                Err(foks_client_app::Error::KvConflict)
                            ),
                            "initial selected read adopted a replacement: {mode}"
                        );
                        let hits = cache.hits.load(Ordering::Acquire);
                        let misses = cache.misses.load(Ordering::Acquire);
                        let invalidations = cache.invalidations.load(Ordering::Acquire);
                        assert!(
                            matches!(
                                chunk(&original, 64 * 1024, &mut vault),
                                Err(foks_client_app::Error::KvConflict)
                            ),
                            "spanning read adopted a replacement: {mode}"
                        );
                        if mode == "warm" {
                            assert_eq!(cache.hits.load(Ordering::Acquire), hits + 1);
                            assert_eq!(
                                cache.invalidations.load(Ordering::Acquire),
                                invalidations + 1
                            );
                        } else {
                            assert_eq!(cache.misses.load(Ordering::Acquire), misses + 1);
                        }
                        let current = chunk(&replacement, 0, &mut vault)?;
                        assert_eq!(current.content, new[..64 * 1024]);
                        assert_eq!(current.dirent_id, replacement.dirent_id);
                        // Same-entry overwrite must still fail by version, not identity.
                        put(
                            &path,
                            &old,
                            KvMutationPrecondition::ExactVersion(1),
                            &mut vault,
                        )?;
                        let overwritten = selected(&path, &mut vault)?;
                        assert_eq!(overwritten.dirent_id, replacement.dirent_id);
                        assert_eq!(overwritten.version, 2);
                        assert!(matches!(
                            chunk(&replacement, 64 * 1024, &mut vault),
                            Err(foks_client_app::Error::KvConflict)
                        ));
                        assert_eq!(
                            chunk(&overwritten, 0, &mut vault)?.content,
                            old[..64 * 1024]
                        );
                    }
                }
            }
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
}
