use super::support::{account_ref, phase_four_state, team_ref};
use super::vault::{download_item, DownloadTransport, ReadTransport, READ_VALUE};
use crate::commands::store_key::store_id;
use crate::commands::validation::DOWNLOAD_CHUNK_BYTES;
use crate::commands::vault_copy::{
    copy_scope, copy_target, read_copy_source, require_spool_space, write_copy, CopySource,
    CopyTarget, SealedSpool,
};
use foks_agent_proto::{KvPrecondition, KvRole, KvStoreRef, KvUploadHeader, Operation};
use foks_desktop::{CatalogItem, CatalogStoreRef};
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;
use std::sync::Mutex;

fn engineering() -> CatalogStoreRef {
    CatalogStoreRef::Team(team_ref("work.example", "personal", "engineering"))
}

fn team_target(path: &str) -> CopyTarget {
    CopyTarget {
        store: engineering(),
        path: path.to_owned(),
        read_role: KvRole::Member { visibility: 0 },
        write_role: KvRole::Admin,
    }
}

fn secret_item() -> CatalogItem {
    CatalogItem {
        store: CatalogStoreRef::Account(account_ref("foks.example", "personal")),
        metadata: foks_agent_proto::KvEntryMetadata {
            dirent_id: [7; 16],
            path: "/wifi/password".to_owned(),
            node_type: "small-file".to_owned(),
            version: 7,
            size: None,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            shape: None,
        },
    }
}

/// Serves a large file's reads and records, in order, every read and the
/// upload, with the bytes the upload consumed.
struct CopyTransport {
    source: DownloadTransport,
    events: Mutex<Vec<&'static str>>,
    upload: Mutex<Option<(KvUploadHeader, Vec<u8>)>>,
    writes: Mutex<Vec<Operation>>,
}

impl CopyTransport {
    fn new(total: u64) -> Self {
        Self {
            source: DownloadTransport {
                total,
                calls: Mutex::new(Vec::new()),
            },
            events: Mutex::new(Vec::new()),
            upload: Mutex::new(None),
            writes: Mutex::new(Vec::new()),
        }
    }
}

impl foks_desktop::AgentTransport for CopyTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if operation.is_mutation() {
            self.events.lock().unwrap().push("write");
            self.writes.lock().unwrap().push(operation);
            return Ok(serde_json::Value::Null);
        }
        self.events.lock().unwrap().push("read");
        self.source.call(operation)
    }

    fn put_kv_stream(
        &self,
        header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.events.lock().unwrap().push("upload");
        let mut content = Vec::new();
        reader
            .read_to_end(&mut content)
            .map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))?;
        *self.upload.lock().unwrap() = Some((header, content));
        Ok(serde_json::Value::Null)
    }
}

#[test]
fn a_large_file_is_read_whole_before_one_create_upload_into_the_target() {
    let total = 2 * u64::from(DOWNLOAD_CHUNK_BYTES) + 5;
    let item = download_item(total);
    let transport = CopyTransport::new(total);

    let source = read_copy_source(
        &transport,
        &item,
        &crate::applock::AppLock::unlocked_for_test().interval_for_test(),
        &deadline(),
    )
    .unwrap();
    assert!(matches!(source, CopySource::Spooled(_)));
    write_copy(&transport, source, &team_target("/shared/large.bin")).unwrap();

    // The agent admits an upload exclusively per profile, so every chunk
    // read finishes before the upload starts.
    let events = transport.events.lock().unwrap();
    assert_eq!(events.last(), Some(&"upload"));
    assert_eq!(events.iter().filter(|event| **event == "upload").count(), 1);
    assert_eq!(
        events.len(),
        2 + total.div_ceil(u64::from(DOWNLOAD_CHUNK_BYTES)) as usize
    );
    let (header, content) = transport.upload.lock().unwrap().take().unwrap();
    assert_eq!(
        header.store,
        KvStoreRef::Team(team_ref("work.example", "personal", "engineering"))
    );
    assert_eq!(header.path, "/shared/large.bin");
    assert_eq!(header.total_length, total);
    assert_eq!(header.read_role, KvRole::Member { visibility: 0 });
    assert_eq!(header.write_role, KvRole::Admin);
    assert_eq!(header.precondition, KvPrecondition::Create);
    assert!(header.mkdir_p);
    assert_eq!(content.len() as u64, total);
    assert!(content.iter().all(|byte| *byte == 0x5a));
    assert!(transport.writes.lock().unwrap().is_empty());
}

#[test]
fn a_secret_is_created_inline_with_the_target_roles_and_a_create_precondition() {
    let item = secret_item();
    let transport = ReadTransport {
        calls: Mutex::new(Vec::new()),
    };
    let source = read_copy_source(
        &transport,
        &item,
        &crate::applock::AppLock::unlocked_for_test().interval_for_test(),
        &deadline(),
    )
    .unwrap();
    assert_eq!(transport.calls.lock().unwrap().len(), 1);

    let writes = CopyTransport::new(0);
    write_copy(&writes, source, &team_target("/wifi/password")).unwrap();

    assert_eq!(
        writes.writes.lock().unwrap().as_slice(),
        &[Operation::PutKv {
            store: KvStoreRef::Team(team_ref("work.example", "personal", "engineering")),
            path: "/wifi/password".to_owned(),
            content: READ_VALUE.to_vec(),
            read_role: KvRole::Member { visibility: 0 },
            write_role: KvRole::Admin,
            precondition: KvPrecondition::Create,
            mkdir_p: true,
        }]
    );
    assert!(writes.upload.lock().unwrap().is_none());
}

#[test]
fn folders_and_links_are_refused_before_any_read() {
    for node_type in ["directory", "symlink"] {
        let mut item = secret_item();
        item.metadata.node_type = node_type.to_owned();
        let transport = ReadTransport {
            calls: Mutex::new(Vec::new()),
        };
        let error = read_copy_source(
            &transport,
            &item,
            &crate::applock::AppLock::unlocked_for_test().interval_for_test(),
            &deadline(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "invalid-request", "{node_type}");
        assert!(transport.calls.lock().unwrap().is_empty(), "{node_type}");
    }
}

#[test]
fn a_file_larger_than_the_free_temporary_space_is_refused_before_any_read() {
    let total = 2 * u64::from(DOWNLOAD_CHUNK_BYTES) + 5;
    // The plaintext and one tag for each of the three chunks.
    let needed = total + 3 * 16;
    let directory = Path::new("/tmp");
    require_spool_space(total, needed, directory).unwrap();
    let error = require_spool_space(total, needed - 1, directory).unwrap_err();
    assert_eq!(error.code, "copy-buffer");
    assert!(error.message.contains(&needed.to_string()));
    require_spool_space(0, 0, directory).unwrap();
}

/// Flips the low bit of the byte at `offset`, so the held ciphertext always
/// changes whatever the random key made it.
fn flip_byte(file: &mut File, offset: u64) {
    let mut byte = [0u8];
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0x01;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
}

#[test]
fn the_spool_holds_only_sealed_chunks_and_refuses_altered_ones() {
    let plaintext = b"correct horse battery staple ".repeat(64);
    let mut spool = SealedSpool::new().unwrap();
    for chunk in plaintext.chunks(500) {
        spool.append(chunk).unwrap();
    }
    assert_eq!(spool.len(), plaintext.len() as u64);
    let mut held = Vec::new();
    let file = spool.file_mut();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.read_to_end(&mut held).unwrap();
    assert!(!held
        .windows(b"battery".len())
        .any(|window| window == b"battery"));

    let mut read = Vec::new();
    spool.into_reader().unwrap().read_to_end(&mut read).unwrap();
    assert_eq!(read, plaintext);

    let mut spool = SealedSpool::new().unwrap();
    spool.append(b"first").unwrap();
    spool.append(b"second").unwrap();
    flip_byte(spool.file_mut(), 0);
    let mut reader = spool.into_reader().unwrap();
    assert!(reader.read_to_end(&mut Vec::new()).is_err());
    assert!(reader.failure.is_some());
}

#[test]
fn a_spool_read_failure_fails_the_copy_without_reporting_an_upload() {
    let mut spool = SealedSpool::new().unwrap();
    spool.append(&[1; 64]).unwrap();
    flip_byte(spool.file_mut(), 3);
    let transport = CopyTransport::new(0);
    let error = write_copy(
        &transport,
        CopySource::Spooled(spool),
        &team_target("/large.bin"),
    )
    .unwrap_err();
    assert_eq!(error.code, "copy-buffer");
    assert!(!error.ambiguous);
}

#[test]
fn a_copy_reserves_one_profile_or_every_profile_when_it_crosses_servers() {
    let state = phase_four_state(Vec::new());
    let personal = store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    let engineering = store_id(&engineering());
    let homelab = store_id(&CatalogStoreRef::Team(team_ref(
        "home.example",
        "home",
        "homelab",
    )));

    let same = copy_scope(&state, &personal, &engineering).unwrap();
    assert_eq!(same.mutation_profile(), Some("work.example"));

    let across = copy_scope(&state, &personal, &homelab).unwrap();
    assert_eq!(across.mutation_profile(), None);
    let _permit = across.begin_mutation().unwrap();
    for profile in ["work.example", "home.example"] {
        assert_eq!(
            state
                .for_profile(profile)
                .unwrap()
                .begin_mutation()
                .err()
                .unwrap()
                .code,
            "mutation-in-flight"
        );
    }

    assert_eq!(
        copy_scope(&state, "not a store", &homelab)
            .err()
            .unwrap()
            .code,
        "invalid-request"
    );
}

#[test]
fn the_source_is_bound_to_the_listed_version_and_dirent() {
    let state = phase_four_state(Vec::new());
    let store = CatalogStoreRef::Account(account_ref("work.example", "personal"));
    let mut item = secret_item();
    item.store = store.clone();
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .items
        .push(item.clone());
    let id = store_id(&store);
    let listed = "07".repeat(16);

    assert_eq!(
        state
            .selected_bound_item(&id, "/wifi/password", 7, &listed, "copying")
            .unwrap(),
        item
    );
    for (version, dirent) in [(8, listed.as_str()), (7, &*"08".repeat(16))] {
        assert_eq!(
            state
                .selected_bound_item(&id, "/wifi/password", version, dirent, "copying")
                .err()
                .unwrap()
                .code,
            "conflict"
        );
    }
    assert_eq!(
        state
            .selected_bound_item(&id, "/missing", 7, &listed, "copying")
            .err()
            .unwrap()
            .code,
        "conflict"
    );
}

#[test]
fn the_target_is_checked_before_the_source_is_read() {
    let state = phase_four_state(Vec::new());
    let mut item = secret_item();
    item.store = CatalogStoreRef::Account(account_ref("work.example", "personal"));
    let engineering_id = store_id(&engineering());
    let roles = (Some("Member:0"), Some("Admin"));

    let target = copy_target(
        &state,
        &item,
        &engineering_id,
        "/wifi/password".to_owned(),
        roles.0,
        roles.1,
    )
    .unwrap();
    assert_eq!(target.store, engineering());
    assert_eq!(target.path, "/wifi/password");
    assert_eq!(target.read_role, KvRole::Member { visibility: 0 });
    assert_eq!(target.write_role, KvRole::Admin);

    // The item's own store, a path the upload would refuse, and a team
    // without roles are each refused here, before read_copy_source runs.
    let refusals = [
        (store_id(&item.store), "/wifi/password", (None, None)),
        (engineering_id.clone(), "wifi/password", roles),
        (engineering_id.clone(), "/wifi/password", (None, None)),
    ];
    for (store, path, (read, write)) in refusals {
        let error = copy_target(&state, &item, &store, path.to_owned(), read, write)
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid-request", "{store} {path}");
    }
}

fn deadline() -> foks_local_deadline::Deadline {
    foks_local_deadline::Deadline::capture(
        std::time::Instant::now() + std::time::Duration::from_secs(300),
    )
    .unwrap()
}
