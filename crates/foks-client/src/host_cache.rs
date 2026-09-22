//! One verified host projection per host database, discovery name and exact
//! hard-state revision.
//!
//! Restoring a pinned host replays the stored evidence from scratch: the
//! hostchain is decoded and verified, then verified again inside the Merkle
//! anchor, then once more at every level of the anchor's skip-path nest, and
//! each level copies the whole authenticated root set. The nest gains a level
//! for every Merkle epoch the client pins, so the replay grows with the
//! profile's age while the bytes it reads do not change between writes.
//!
//! The key is exact rather than heuristic. Every byte the restore reads comes
//! from a table in `foks_client_db`'s revision set, and the triggers installed
//! with that schema raise `hard_state_revision` and reroll `write_token` on
//! every insert, update and delete to those tables, so an unchanged triple
//! means byte-identical inputs. The restore itself is a pure function of those
//! bytes: it reads no clock, no environment and no file, and it never consults
//! the client. The scheme deliberately over-invalidates, because any hard-state
//! write at all drops the entry, which is the safe direction.
//!
//! Two inputs are not in that triple and are part of the key for that reason.
//! The discovery name is the caller's argument rather than a stored column, and
//! one database holds a row per name, so a key without it would answer a
//! request for one host with another host's identity and endpoints. The
//! database path is the caller's spelling rather than the canonical path, and
//! [`crate::pinning::span`] keys the advance-and-accept exclusion on that exact
//! spelling, so a key without it would let two spellings of one file share an
//! entry and silently defeat that exclusion.
//!
//! Entries hold no secrets: a pinned host is public identity, service
//! endpoints and TLS roots.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use foks_client_db::HardStateMetadata;
use foks_verify::VerifiedMerkleRoot;

use crate::PinnedHost;

/// One entry per host database a process realistically drives at once. A new
/// revision replaces its predecessor rather than accumulating beside it, so
/// this bounds distinct databases and names, not writes. Each entry holds one
/// Merkle anchor, whose stored evidence grows with the number of epochs its
/// profile has pinned, so this is also the bound on how much of that a process
/// keeps resident.
const MAXIMUM_CACHED_HOSTS: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheKey {
    database_path: PathBuf,
    lookup_name: String,
    metadata: HardStateMetadata,
}

impl CacheKey {
    pub(crate) fn new(
        database_path: &Path,
        lookup_name: &str,
        metadata: HardStateMetadata,
    ) -> Self {
        Self {
            database_path: database_path.to_owned(),
            lookup_name: lookup_name.to_owned(),
            metadata,
        }
    }

    /// Whether both keys name the same host in the same database, whatever
    /// revision each was taken at.
    fn names_same_host(&self, other: &Self) -> bool {
        self.database_path == other.database_path && self.lookup_name == other.lookup_name
    }
}

/// A bounded map from an exact key to a restored value, holding one entry per
/// named host and evicting the oldest when full.
struct BoundedCache<V> {
    entries: VecDeque<(CacheKey, V)>,
}

impl<V: Clone> BoundedCache<V> {
    const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    fn get(&self, key: &CacheKey) -> Option<V> {
        self.entries
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, value)| value.clone())
    }

    fn put(&mut self, key: CacheKey, value: V) {
        self.entries
            .retain(|(stored, _)| !stored.names_same_host(&key));
        while self.entries.len() >= MAXIMUM_CACHED_HOSTS {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Everything one restore of a host's stored evidence produces: the
/// authenticated capability, the Merkle anchor a chain replay is checked
/// against, and the host chain those replays cite.
pub(crate) struct RestoredHost {
    pub(crate) host: PinnedHost,
    pub(crate) anchor: VerifiedMerkleRoot,
    pub(crate) chain_bytes: Vec<u8>,
    pub(crate) chain_seqno: u64,
    pub(crate) chain_tail_hash: [u8; 32],
}

fn cache() -> &'static Mutex<BoundedCache<Arc<RestoredHost>>> {
    static CACHE: OnceLock<Mutex<BoundedCache<Arc<RestoredHost>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BoundedCache::new()))
}

static REPLAYS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn get(key: &CacheKey) -> Option<Arc<RestoredHost>> {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
}

pub(crate) fn put(key: CacheKey, restored: &Arc<RestoredHost>) {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .put(key, Arc::clone(restored));
}

pub(crate) fn record_replay() {
    REPLAYS.fetch_add(1, Ordering::Relaxed);
}

/// Counts the host restores that replayed the stored evidence rather than
/// reusing a cached projection. The key is exact, so this reports work done,
/// not correctness.
pub fn host_replay_count() -> u64 {
    REPLAYS.load(Ordering::Relaxed)
}

/// Drops every cached host. The key already covers a relocated, reimported
/// or reset state directory, because each changes the path or the database
/// identity; this is for a caller that would rather not depend on that.
pub fn clear_cached_hosts() {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(revision: u64, token: u8) -> HardStateMetadata {
        HardStateMetadata {
            database_id: [7; 16],
            revision,
            write_token: [token; 16],
        }
    }

    fn key(path: &str, name: &str, revision: u64, token: u8) -> CacheKey {
        CacheKey::new(Path::new(path), name, metadata(revision, token))
    }

    #[test]
    fn a_cached_host_is_returned_only_for_its_own_name_path_and_revision() {
        let mut cache = BoundedCache::new();
        let stored = key("/a/hard.sqlite3", "one.test", 4, 1);
        cache.put(stored.clone(), "one");

        assert_eq!(cache.get(&stored), Some("one"));
        // A second discovery name in the same database at the same revision.
        // Without the name in the key this would answer with the first host's
        // identity and endpoints.
        assert_eq!(cache.get(&key("/a/hard.sqlite3", "two.test", 4, 1)), None);
        // A second spelling of the same file. Path equality folds away a `.`
        // component but keeps `..`, so this is a spelling the advance-and-accept
        // exclusion would also treat as a distinct key, and the cache must agree
        // with it rather than alias the two.
        assert_eq!(
            cache.get(&key("/a/sub/../hard.sqlite3", "one.test", 4, 1)),
            None
        );
        // The same revision number with a rerolled write token.
        assert_eq!(cache.get(&key("/a/hard.sqlite3", "one.test", 4, 2)), None);
        // A later revision.
        assert_eq!(cache.get(&key("/a/hard.sqlite3", "one.test", 5, 1)), None);
    }

    #[test]
    fn a_new_revision_replaces_its_predecessor_rather_than_accumulating() {
        let mut cache = BoundedCache::new();
        let first = key("/b/hard.sqlite3", "one.test", 1, 1);
        let second = key("/b/hard.sqlite3", "one.test", 2, 2);
        cache.put(first.clone(), "first");
        cache.put(second.clone(), "second");

        assert_eq!(cache.get(&first), None);
        assert_eq!(cache.get(&second), Some("second"));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_second_name_in_one_database_keeps_its_own_entry() {
        let mut cache = BoundedCache::new();
        let one = key("/b/hard.sqlite3", "one.test", 1, 1);
        let two = key("/b/hard.sqlite3", "two.test", 1, 1);
        cache.put(one.clone(), "one");
        cache.put(two.clone(), "two");

        assert_eq!(cache.get(&one), Some("one"));
        assert_eq!(cache.get(&two), Some("two"));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn the_cache_is_bounded_and_evicts_oldest_first() {
        let mut cache = BoundedCache::new();
        let mut keys = Vec::new();
        for index in 0..MAXIMUM_CACHED_HOSTS + 1 {
            let path = format!("/c/{index}/hard.sqlite3");
            let entry = key(&path, "one.test", 1, 1);
            cache.put(entry.clone(), index);
            keys.push(entry);
        }

        assert_eq!(cache.len(), MAXIMUM_CACHED_HOSTS);
        assert_eq!(cache.get(&keys[0]), None);
        assert_eq!(cache.get(&keys[1]), Some(1));
        assert_eq!(
            cache.get(keys.last().expect("a key was inserted")),
            Some(MAXIMUM_CACHED_HOSTS)
        );
    }
}
