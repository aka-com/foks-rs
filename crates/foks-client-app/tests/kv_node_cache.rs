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
            let version = written.version;
            assert_eq!(version, 2);

            let before = server.metrics().requests_started;
            let first = session.read_kv_chunk(
                "owner",
                "/nested/large",
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
