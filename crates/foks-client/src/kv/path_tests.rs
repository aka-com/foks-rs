//! Path-scoped KV resolution measured against a synthetic store.
//!
//! The store is built with the same sealing primitives the writer uses, so a
//! resolution here performs every check it performs against a real server.
//! Each test counts the requests the resolver issues by kind and inspects the
//! version vector it asserts, which is the vector a mutation on that path
//! carries as its precondition.

use std::collections::{BTreeMap, BTreeSet};

use foks_client_db::KvDirectoryProjection;
use foks_crypto::{derive_kv_keys, seal_kv_chunk, seal_kv_dirent_name};
use foks_proto::{
    EntityId, KvDirectory, KvDirectoryPair, KvDirectoryStatus, KvDirent, KvEncryptedChunk,
    KvExtendedDirent, KvListResponse, KvNode, KvNodeId, KvNodeType, KvParty, KvPathVersionVector,
    KvRoot, KvSmallFilePlaintext, Role, RoleAndGeneration, SecretSeed,
};
use foks_rpc::KvAuth;

use super::support::{bound_dirent, kv_version_vector_from_tree};
use super::{KvPrivateKeyRef, KvRequest};
use crate::{
    read_kv_chunk_with_fetch, read_kv_node_with_fetch, verify_public_host, Error, FoksClient,
    HardStateStore, PinnedHost, Result,
};

const PROBE: &[u8] = include_bytes!(
    "../../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
);
const CHUNK_BYTES: usize = 4096;

/// Every KV request the resolver may issue, counted by kind.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RpcCounts {
    root: usize,
    directory: usize,
    list: usize,
    node: usize,
    chunk: usize,
    cache_check: usize,
}

#[derive(Default)]
struct Transcript {
    counts: RpcCounts,
    asserted: Vec<KvPathVersionVector>,
    stale_cache_checks: usize,
}

struct FakeDirectory {
    pair: KvDirectoryPair,
    seed: SecretSeed,
    entries: Vec<KvDirent>,
    extended: Vec<KvExtendedDirent>,
}

/// A server-shaped KV namespace whose ciphertexts are sealed under one key.
struct FakeStore {
    party: KvParty,
    seed: SecretSeed,
    key: RoleAndGeneration,
    root: KvRoot,
    root_directory: [u8; 16],
    directories: BTreeMap<[u8; 16], FakeDirectory>,
    nodes: BTreeMap<[u8; 17], Vec<u8>>,
    chunks: BTreeMap<([u8; 17], u64), Vec<u8>>,
    denied: BTreeSet<[u8; 16]>,
    next_id: u8,
}

impl FakeStore {
    fn new(party: KvParty) -> Self {
        let seed = SecretSeed::new([0x31; 32]);
        let key = RoleAndGeneration {
            role: Role::OWNER,
            generation: 1,
        };
        let root_directory = [1; 16];
        let binding_mac = derive_kv_keys(&seed)
            .unwrap()
            .bind_root(&party, root_directory, 1, key)
            .unwrap();
        let mut store = Self {
            party,
            seed,
            key,
            root: KvRoot::new(root_directory, 1, key, binding_mac).unwrap(),
            root_directory,
            directories: BTreeMap::new(),
            nodes: BTreeMap::new(),
            chunks: BTreeMap::new(),
            denied: BTreeSet::new(),
            next_id: 2,
        };
        store.insert_directory(root_directory);
        store
    }

    fn private_keys(&self) -> Vec<KvPrivateKeyRef<'_>> {
        vec![KvPrivateKeyRef {
            role: self.key.role,
            generation: self.key.generation,
            seed: &self.seed,
        }]
    }

    fn fresh_id(&mut self) -> [u8; 16] {
        let id = [self.next_id; 16];
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("the synthetic store stays small");
        id
    }

    fn insert_directory(&mut self, id: [u8; 16]) {
        let seed = SecretSeed::new([id[0]; 32]);
        let keys = derive_kv_keys(&self.seed).unwrap();
        let directory = KvDirectory {
            id,
            version: 1,
            key: self.key,
            seed_ciphertext: keys.seal_directory_seed(id, &seed).unwrap(),
            write_role: Role::OWNER,
            status: KvDirectoryStatus::Active,
        };
        self.directories.insert(
            id,
            FakeDirectory {
                pair: KvDirectoryPair::from_active(directory).unwrap(),
                seed,
                entries: Vec::new(),
                extended: Vec::new(),
            },
        );
    }

    fn link(&mut self, parent: [u8; 16], name: &str, node: KvNodeId) {
        let dirent_id = self.fresh_id();
        let directory = self
            .directories
            .get_mut(&parent)
            .expect("linking into a directory the store holds");
        let (name_mac, name_box) = seal_kv_dirent_name(
            &directory.seed,
            parent,
            1,
            name.as_bytes().to_vec(),
            [dirent_id[0]; 16],
        )
        .unwrap();
        let dirent = bound_dirent(
            &directory.seed,
            parent,
            dirent_id,
            node,
            1,
            1,
            Role::OWNER,
            name_mac,
            name_box,
            KvDirectoryStatus::Active,
            7,
        )
        .unwrap();
        directory.entries.push(dirent);
    }

    fn mkdir(&mut self, parent: [u8; 16], name: &str) -> [u8; 16] {
        let id = self.fresh_id();
        self.insert_directory(id);
        let mut node = [0; 17];
        node[0] = KvNodeType::Directory as u8;
        node[1..].copy_from_slice(&id);
        self.link(parent, name, KvNodeId(node));
        id
    }

    fn small_file(&mut self, parent: [u8; 16], name: &str, content: &[u8]) -> KvNodeId {
        let object = self.fresh_id();
        let mut node = [0; 17];
        node[0] = KvNodeType::SmallFile as u8;
        node[1..].copy_from_slice(&object);
        let node = KvNodeId(node);
        let boxed = derive_kv_keys(&self.seed)
            .unwrap()
            .seal_small_file(node, self.key, KvSmallFilePlaintext::File(content.to_vec()))
            .unwrap();
        self.nodes
            .insert(node.0, KvNode::SmallFile(boxed.clone()).encoded().unwrap());
        self.link(parent, name, node);
        let directory = self.directories.get_mut(&parent).expect("linked parent");
        let position = (directory.entries.len() - 1) as u64;
        directory.extended.push(KvExtendedDirent {
            position,
            small_file: boxed,
        });
        node
    }

    fn large_file(&mut self, parent: [u8; 16], name: &str, content: &[u8]) -> KvNodeId {
        let object = self.fresh_id();
        let mut node = [0; 17];
        node[0] = KvNodeType::File as u8;
        node[1..].copy_from_slice(&object);
        let node = KvNodeId(node);
        let file_seed = SecretSeed::new([object[0]; 32]);
        let metadata = derive_kv_keys(&self.seed)
            .unwrap()
            .seal_file_seed(node, self.key, 1, &file_seed, [object[0]; 16])
            .unwrap();
        self.nodes
            .insert(node.0, KvNode::File(metadata).encoded().unwrap());
        let mut offset = 0u64;
        let mut encrypted = 0u64;
        for chunk in content.chunks(CHUNK_BYTES) {
            let last = offset as usize + chunk.len() == content.len();
            let sealed = seal_kv_chunk(&file_seed, node, offset, last, chunk, encrypted).unwrap();
            encrypted += sealed.ciphertext.len() as u64;
            let served = KvEncryptedChunk {
                ciphertext: sealed.ciphertext,
                offset,
                final_chunk: last,
            };
            self.chunks
                .insert((node.0, offset), served.encode().unwrap());
            offset += chunk.len() as u64;
        }
        self.link(parent, name, node);
        node
    }

    fn deny(&mut self, directory: [u8; 16]) {
        self.denied.insert(directory);
    }

    fn set_root_version(&mut self, version: u64) {
        let binding_mac = derive_kv_keys(&self.seed)
            .unwrap()
            .bind_root(&self.party, self.root_directory, version, self.key)
            .unwrap();
        self.root = KvRoot::new(self.root_directory, version, self.key, binding_mac).unwrap();
    }

    fn status(status: foks_rpc::RpcStatus) -> Error {
        let frame = foks_rpc::encode_status_response_at(&status, 1).unwrap();
        Error::Rpc(
            foks_rpc::read_response(
                &mut std::io::Cursor::new(frame),
                foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
                1,
            )
            .unwrap_err(),
        )
    }

    fn denied_response() -> Error {
        Self::status(foks_rpc::RpcStatus::PermissionDenied(
            "restricted".to_owned(),
        ))
    }

    fn stale_cache() -> Error {
        Error::Rpc(foks_rpc::Error::KvStaleCache(KvPathVersionVector {
            root_version: 1,
            directories: Vec::new(),
        }))
    }

    fn serve(&self, transcript: &mut Transcript, request: &KvRequest) -> Result<Vec<u8>> {
        match request {
            KvRequest::Root => {
                transcript.counts.root += 1;
                Ok(self.root.encoded().to_vec())
            }
            KvRequest::Directory(id) => {
                transcript.counts.directory += 1;
                if self.denied.contains(id) {
                    return Err(Self::denied_response());
                }
                let directory = self
                    .directories
                    .get(id)
                    .ok_or(Error::KvResponse("unknown directory"))?;
                Ok(directory.pair.encoded().to_vec())
            }
            KvRequest::List { directory, .. } => {
                transcript.counts.list += 1;
                let directory = self
                    .directories
                    .get(directory)
                    .ok_or(Error::KvResponse("unknown directory"))?;
                Ok(
                    KvListResponse::new(
                        directory.entries.clone(),
                        true,
                        directory.extended.clone(),
                    )
                    .unwrap()
                    .encoded()
                    .to_vec(),
                )
            }
            KvRequest::Node(node) => {
                transcript.counts.node += 1;
                self.nodes
                    .get(&node.0)
                    .cloned()
                    .ok_or(Error::KvResponse("unknown node"))
            }
            KvRequest::Chunk { file, offset } => {
                transcript.counts.chunk += 1;
                self.chunks
                    .get(&(file.0, *offset))
                    .cloned()
                    .ok_or_else(|| Self::status(foks_rpc::RpcStatus::KvNoEnt))
            }
            KvRequest::CacheCheck(versions) => {
                transcript.counts.cache_check += 1;
                transcript.asserted.push(versions.clone());
                if transcript.stale_cache_checks > 0 {
                    transcript.stale_cache_checks -= 1;
                    return Err(Self::stale_cache());
                }
                Ok(Vec::new())
            }
            _ => Err(Error::KvResponse("unexpected request")),
        }
    }
}

fn pinned(directory: &tempfile::TempDir) -> (FoksClient, PinnedHost) {
    let public = verify_public_host("foks.app", PROBE).unwrap();
    let hard_path = directory.path().join("hard.sqlite3");
    HardStateStore::open(&hard_path)
        .unwrap()
        .accept_verified_host(&public.snapshot)
        .unwrap();
    let client = FoksClient::webpki();
    let host = client.pinned_host("foks.app", &hard_path).unwrap();
    (client, host)
}

/// A store with `width` directories directly under the root, a nested
/// directory inside the first of them and one small file at depth two.
struct Fixture {
    store: FakeStore,
    nested: [u8; 16],
    first: [u8; 16],
    file: KvNodeId,
}

fn wide_store(party: KvParty, width: usize) -> Fixture {
    let mut store = FakeStore::new(party);
    let root = store.root_directory;
    let mut first = None;
    let mut nested = None;
    for index in 0..width {
        let id = store.mkdir(root, &format!("dir-{index:03}"));
        if index == 0 {
            let inner = store.mkdir(id, "inner");
            nested = Some(inner);
            first = Some(id);
        }
    }
    let nested = nested.expect("the fixture always has a nested directory");
    let file = store.small_file(nested, "target.txt", b"path-scoped");
    Fixture {
        store,
        nested,
        first: first.expect("the fixture always has a first directory"),
        file,
    }
}

fn party_for(host: &PinnedHost) -> KvParty {
    let mut party = vec![0x30; 33];
    party[0] = foks_proto::ENTITY_USER;
    KvParty {
        party: EntityId::from_bytes(party).unwrap(),
        host: host.host_id.clone(),
    }
}

fn resolve(
    client: &FoksClient,
    host: &PinnedHost,
    store: &FakeStore,
    soft: &std::path::Path,
    components: &[&str],
) -> (Result<Vec<KvDirectoryProjection>>, Transcript) {
    let components = components
        .iter()
        .map(|component| component.as_bytes().to_vec())
        .collect::<Vec<_>>();
    let mut transcript = Transcript::default();
    let outcome = client.resolve_kv_path_with_fetch(
        host,
        store.party.clone(),
        KvAuth::User,
        &store.private_keys(),
        soft,
        &components,
        |_, request| store.serve(&mut transcript, request),
    );
    (outcome, transcript)
}

fn cited(vector: &KvPathVersionVector) -> BTreeSet<[u8; 16]> {
    vector
        .directories
        .iter()
        .map(|directory| directory.id)
        .collect()
}

#[test]
fn resolving_one_path_reads_only_that_paths_directories() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let fixture = wide_store(party_for(&host), 50);
    let soft = temporary.path().join("soft.sqlite3");

    let (resolved, transcript) =
        resolve(&client, &host, &fixture.store, &soft, &["dir-000", "inner"]);
    let resolved = resolved.unwrap();

    // Root, then exactly one directory per component: depth + 1 in a store of
    // fifty-two directories.
    assert_eq!(
        transcript.counts,
        RpcCounts {
            root: 1,
            directory: 3,
            list: 3,
            node: 0,
            chunk: 0,
            cache_check: 1,
        }
    );
    assert_eq!(resolved.len(), 3);
    assert_eq!(resolved[0].directory_id, fixture.store.root_directory);
    assert_eq!(resolved[1].directory_id, fixture.first);
    assert_eq!(resolved[2].directory_id, fixture.nested);

    // The closing assertion, and therefore a mutation's precondition, names
    // only the directories the walk used.
    let expected = BTreeSet::from([fixture.store.root_directory, fixture.first, fixture.nested]);
    assert_eq!(cited(&transcript.asserted[0]), expected);
    assert_eq!(transcript.asserted[0].root_version, 1);
    assert_eq!(
        cited(&kv_version_vector_from_tree(&resolved)),
        expected,
        "a write built from this path cites the same directories"
    );
    let root_citation = transcript.asserted[0]
        .directories
        .iter()
        .find(|directory| directory.id == fixture.store.root_directory)
        .expect("the root is cited");
    assert_eq!(
        root_citation.entries.len(),
        50,
        "every dirent of a cited directory is asserted unchanged"
    );
}

#[test]
fn a_complete_traversal_of_the_same_store_reads_every_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let fixture = wide_store(party_for(&host), 50);
    let soft = temporary.path().join("soft.sqlite3");
    let mut transcript = Transcript::default();

    let tree = client
        .list_kv_metadata_with_fetch(
            &host,
            fixture.store.party.clone(),
            KvAuth::User,
            &fixture.store.private_keys(),
            &soft,
            |_, request| fixture.store.serve(&mut transcript, request),
        )
        .unwrap();

    assert_eq!(tree.len(), 52);
    assert_eq!(transcript.counts.directory, 52);
    assert_eq!(transcript.counts.list, 52);
}

#[test]
fn a_node_read_and_a_chunk_read_fetch_no_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let mut fixture = wide_store(party_for(&host), 50);
    let large =
        fixture
            .store
            .large_file(fixture.nested, "archive.bin", &vec![0x5a; CHUNK_BYTES * 8]);
    let soft = temporary.path().join("soft.sqlite3");

    let (resolved, mut transcript) =
        resolve(&client, &host, &fixture.store, &soft, &["dir-000", "inner"]);
    let resolved = resolved.unwrap();
    assert!(resolved
        .last()
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.name == b"target.txt"));
    // One node fetch per large file in a directory the walk used, exactly as
    // the complete traversal authenticates it.
    let after_resolution = transcript.counts;
    assert_eq!(after_resolution.directory, 3);

    let keys = fixture.store.private_keys();
    let node = read_kv_node_with_fetch(fixture.file, &keys, KvAuth::User, |_, request| {
        fixture.store.serve(&mut transcript, request)
    })
    .unwrap();
    assert_eq!(
        node,
        super::KvFetchedNode::SmallFile(b"path-scoped".to_vec())
    );
    assert_eq!(transcript.counts.directory, after_resolution.directory);
    assert_eq!(transcript.counts.list, after_resolution.list);

    let before_chunks = transcript.counts;
    for index in 0..8u64 {
        let chunk = read_kv_chunk_with_fetch(
            large,
            index * CHUNK_BYTES as u64,
            CHUNK_BYTES,
            &keys,
            KvAuth::User,
            |_, request| fixture.store.serve(&mut transcript, request),
        )
        .unwrap();
        assert_eq!(chunk.content.len(), CHUNK_BYTES);
    }
    assert_eq!(transcript.counts.directory, before_chunks.directory);
    assert_eq!(transcript.counts.list, before_chunks.list);
    assert_eq!(transcript.counts.root, before_chunks.root);
}

#[test]
fn an_absent_component_stops_the_walk_where_a_complete_traversal_would() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let fixture = wide_store(party_for(&host), 8);
    let soft = temporary.path().join("soft.sqlite3");

    let (resolved, transcript) = resolve(
        &client,
        &host,
        &fixture.store,
        &soft,
        &["dir-000", "missing", "deeper"],
    );
    let resolved = resolved.unwrap();

    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[1].directory_id, fixture.first);
    assert_eq!(transcript.counts.directory, 2);
    assert_eq!(cited(&transcript.asserted[0]).len(), 2);
}

#[test]
fn a_refused_directory_on_the_path_is_marked_unreadable_not_absent() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let mut fixture = wide_store(party_for(&host), 8);
    fixture.store.deny(fixture.nested);
    let soft = temporary.path().join("soft.sqlite3");

    let (resolved, transcript) =
        resolve(&client, &host, &fixture.store, &soft, &["dir-000", "inner"]);
    let resolved = resolved.unwrap();

    assert_eq!(resolved.len(), 2, "the refused directory is not projected");
    assert_eq!(transcript.counts.directory, 3);
    let referring = resolved[1]
        .entries
        .iter()
        .find(|entry| entry.name == b"inner")
        .expect("the dirent naming the refused directory is retained");
    assert!(
        !referring.readable,
        "a refused directory leaves its dirent unreadable"
    );
}

#[test]
fn a_refused_root_directory_fails_the_walk() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let mut fixture = wide_store(party_for(&host), 4);
    fixture.store.deny(fixture.store.root_directory);
    let soft = temporary.path().join("soft.sqlite3");

    let (resolved, _) = resolve(&client, &host, &fixture.store, &soft, &["dir-000"]);
    assert!(resolved.is_err(), "a refused root is never an empty store");
}

#[test]
fn a_stale_cache_assertion_re_reads_the_path_and_not_the_store() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let fixture = wide_store(party_for(&host), 50);
    let soft = temporary.path().join("soft.sqlite3");
    let components = [b"dir-000".to_vec(), b"inner".to_vec()];
    let mut transcript = Transcript {
        stale_cache_checks: 1,
        ..Transcript::default()
    };

    let resolved = client
        .resolve_kv_path_with_fetch(
            &host,
            fixture.store.party.clone(),
            KvAuth::User,
            &fixture.store.private_keys(),
            &soft,
            &components,
            |_, request| fixture.store.serve(&mut transcript, request),
        )
        .unwrap();

    assert_eq!(resolved.len(), 3);
    assert_eq!(transcript.counts.cache_check, 2);
    assert_eq!(
        transcript.counts.directory, 6,
        "the retry re-reads the path, not the fifty-two directories"
    );
}

/// The walk persists the same monotonicity anchors a complete traversal
/// persists, so a later walk still refuses a root the server rewound.
#[test]
fn a_resolved_path_keeps_its_rollback_anchors() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let party = party_for(&host);
    let mut fixture = wide_store(party.clone(), 4);
    fixture.store.set_root_version(4);
    let soft = temporary.path().join("soft.sqlite3");

    resolve(&client, &host, &fixture.store, &soft, &["dir-000", "inner"])
        .0
        .unwrap();

    let mut rewound = wide_store(party, 4);
    rewound.store.set_root_version(3);
    let (outcome, _) = resolve(&client, &host, &rewound.store, &soft, &["dir-000"]);
    let message = outcome
        .expect_err("a rewound root is refused")
        .to_string()
        .to_lowercase();
    assert!(message.contains("rolled back"), "{message}");
}

/// A path walk refreshes the directories it visited without pruning the rest,
/// so a complete traversal of the same cache still accepts and completes it.
#[test]
fn a_path_walk_and_a_complete_traversal_share_one_cache() {
    let temporary = tempfile::tempdir().unwrap();
    let (client, host) = pinned(&temporary);
    let fixture = wide_store(party_for(&host), 8);
    let soft = temporary.path().join("soft.sqlite3");
    let complete = |transcript: &mut Transcript| {
        client.list_kv_metadata_with_fetch(
            &host,
            fixture.store.party.clone(),
            KvAuth::User,
            &fixture.store.private_keys(),
            &soft,
            |_, request| fixture.store.serve(transcript, request),
        )
    };

    resolve(&client, &host, &fixture.store, &soft, &["dir-000", "inner"])
        .0
        .unwrap();
    let mut transcript = Transcript::default();
    assert_eq!(complete(&mut transcript).unwrap().len(), 10);

    resolve(&client, &host, &fixture.store, &soft, &["dir-003"])
        .0
        .unwrap();
    let mut transcript = Transcript::default();
    let tree = complete(&mut transcript).unwrap();
    assert_eq!(
        tree.len(),
        10,
        "a path walk leaves the directories it skipped in the cache"
    );
}
