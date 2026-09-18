use crate::commands::execution::{execute_kv_mutation, MutationKind};
use crate::commands::validation::{DOWNLOAD_CHUNK_BYTES, MAXIMUM_TEXT_ITEM_BYTES};
use crate::commands::vault::{
    create_item_roles, download_to_path, file_create_header, file_edit_header, parse_item_role,
    read_text, remove_item_operation, require_file_item, require_text_item,
    set_create_mutation_roles, set_create_operation_roles, take_text_value, upload_file,
    CatalogDto,
};
use foks_agent_proto::{
    KvChunkResult, KvPrecondition, KvReadResult, KvRole, KvStoreRef, KvUploadHeader, Operation,
};
use foks_desktop::{CatalogItem, CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[test]
fn catalog_dto_preserves_store_read_provenance_for_empty_inventories() {
    let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "local".into(),
        account_alias: "owner".into(),
    });
    for (state, expected) in [
        (foks_desktop::CatalogStoreReadState::NotLoaded, "not-loaded"),
        (foks_desktop::CatalogStoreReadState::Complete, "complete"),
        (foks_desktop::CatalogStoreReadState::Failed, "failed"),
    ] {
        let snapshot = CatalogSnapshot {
            store_reads: vec![foks_desktop::CatalogStoreRead {
                store: store.clone(),
                state,
            }],
            ..Default::default()
        };
        let dto = CatalogDto::from_snapshot(&snapshot).unwrap();
        assert!(dto.items.is_empty());
        assert_eq!(dto.store_reads[0].state, expected);
        assert_eq!(
            serde_json::to_value(dto).unwrap()["storeReads"][0]["state"],
            expected
        );
    }
}

#[test]
fn progressive_metadata_uses_only_completed_overviews_and_preserves_scoped_errors() {
    use foks_agent_proto::{ProfileOverview, ResponseResult};
    let profiles = ["healthy.example", "slow.example", "failed.example"].map(|name| {
        serde_json::from_value(serde_json::json!({
            "name": name, "probe": name,
            "protocol": {"generation":"v019"}, "trust": {"kind":"web-pki"}
        }))
        .unwrap()
    });
    let success = |value| ResponseResult::Success { value };
    let account = foks_agent_proto::AccountStoreRef {
        profile: "healthy.example".into(),
        account_alias: "alice".into(),
    };
    let snapshot = CatalogSnapshot {
        profiles: profiles
            .iter()
            .map(|profile: &crate::commands::servers::ProfileSummary| profile.name.clone())
            .collect(),
        stores: vec![CatalogStoreSummary::Account { store: account }],
        profile_overviews: vec![
            ProfileOverview {
                profile: "healthy.example".into(),
                accounts: success(
                    serde_json::json!([{"profile":"healthy.example", "alias":"alice", "username":"alice"}]),
                ),
                teams: success(serde_json::json!([])),
                server_status: success(
                    serde_json::json!({"profile":"healthy.example", "configured_probe":"healthy.example", "host":null,"chat_supported":null,"compatibility":{"status":"not-required"}}),
                ),
            },
            ProfileOverview {
                profile: "failed.example".into(),
                accounts: success(serde_json::json!([])),
                teams: success(serde_json::json!([])),
                server_status: ResponseResult::Error {
                    code: foks_agent_proto::ErrorCode::ProfileBusy,
                    message: "profile busy".into(),
                    fields: Default::default(),
                },
            },
        ],
        ..Default::default()
    };
    let metadata = crate::commands::vault::catalog_local_metadata(&snapshot, &profiles).unwrap();
    assert_eq!(metadata.accounts.len(), 1);
    assert_eq!(metadata.accounts[0].profile, "healthy.example");
    assert!(metadata.profiles[0].status.is_some());
    assert!(metadata.profiles[1].status.is_none());
    assert!(metadata.profiles[1].error.is_none());
    assert_eq!(
        metadata.profiles[2].error.as_ref().unwrap().code,
        "profile-busy"
    );
}

pub(super) const READ_VALUE: &[u8] = b"guest-password";

pub(super) struct ReadTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for ReadTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ReadKv {
                store,
                path,
                version,
            } => serde_json::to_value(KvReadResult {
                store,
                path,
                version,
                node_type: "small-file".to_owned(),
                size: Some(READ_VALUE.len() as u64),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                content: Some(READ_VALUE.to_vec()),
                symlink_target: None,
            })
            .map_err(|error| foks_desktop::AgentError::Transport(error.to_string())),
            other => panic!("unexpected read operation {other:?}"),
        }
    }
}

pub(super) struct DownloadTransport {
    pub(super) total: u64,
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for DownloadTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        let value = match operation {
            Operation::ReadKv {
                store,
                path,
                version,
            } => serde_json::to_value(KvReadResult {
                store,
                path,
                version,
                node_type: "file".to_owned(),
                size: Some(self.total),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                content: None,
                symlink_target: None,
            }),
            Operation::ReadKvChunk {
                store,
                path,
                version,
                offset,
                length,
            } => {
                let count = usize::try_from((self.total - offset).min(u64::from(length))).unwrap();
                serde_json::to_value(KvChunkResult {
                    store,
                    path,
                    version,
                    offset,
                    content: vec![0x5a; count],
                    eof: offset + count as u64 == self.total,
                })
            }
            other => panic!("unexpected download operation {other:?}"),
        };
        value.map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))
    }
}

pub(super) fn download_item(total: u64) -> CatalogItem {
    let store = foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    };
    CatalogItem {
        store: CatalogStoreRef::Account(store),
        metadata: foks_agent_proto::KvEntryMetadata {
            path: "/large.bin".to_owned(),
            node_type: "file".to_owned(),
            version: 9,
            size: Some(total),
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
        },
    }
}

#[test]
fn read_text_issues_exactly_one_version_bound_read() {
    let item = CatalogItem {
        store: CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
        }),
        metadata: foks_agent_proto::KvEntryMetadata {
            path: "/wifi/password".to_owned(),
            node_type: "small-file".to_owned(),
            version: 7,
            size: Some(READ_VALUE.len() as u64),
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
        },
    };
    let transport = ReadTransport {
        calls: Mutex::new(Vec::new()),
    };

    let read = read_text(&transport, &item).unwrap();

    assert_eq!(read.value.as_str(), "guest-password");
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[Operation::ReadKv {
            store: KvStoreRef::Account(foks_agent_proto::AccountStoreRef {
                profile: "foks.example".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            path: "/wifi/password".to_owned(),
            version: 7,
        }]
    );
}

#[test]
fn download_streams_files_larger_than_the_reveal_limit_in_bound_chunks() {
    let total = 16 * 1024 * 1024 + 1;
    let item = download_item(total);
    let transport = DownloadTransport {
        total,
        calls: Mutex::new(Vec::new()),
    };
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("large.bin");
    download_to_path(&transport, &item, &destination).unwrap();
    assert_eq!(std::fs::metadata(destination).unwrap().len(), total);
    let calls = transport.calls.lock().unwrap();
    assert!(matches!(
        calls.first(),
        Some(Operation::ReadKv { version: 9, .. })
    ));
    assert!(calls[1..].iter().all(|call| matches!(
        call,
        Operation::ReadKvChunk { version: 9, length, .. } if *length <= DOWNLOAD_CHUNK_BYTES
    )));
    assert_eq!(
        calls.len(),
        1 + total.div_ceil(u64::from(DOWNLOAD_CHUNK_BYTES)) as usize
    );
}

#[test]
fn catalog_dto_rejects_unknown_kinds_and_preserves_unknown_sizes() {
    let account = foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    };
    let mut snapshot = CatalogSnapshot {
        profiles: vec!["foks.example".to_owned()],
        stores: vec![CatalogStoreSummary::Account {
            store: account.clone(),
        }],
        items: vec![CatalogItem {
            store: CatalogStoreRef::Account(account.clone()),
            metadata: foks_agent_proto::KvEntryMetadata {
                path: "/bad".to_owned(),
                node_type: "socket".to_owned(),
                version: 1,
                size: Some(1),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
            },
        }],
        ..CatalogSnapshot::default()
    };
    assert_eq!(
        CatalogDto::from_snapshot(&snapshot).unwrap_err().code,
        "invalid-response"
    );
    snapshot.items[0].metadata.node_type = "small-file".to_owned();
    snapshot.items[0].metadata.size = None;
    for kind in ["small-file", "file", "symlink", "directory"] {
        snapshot.items[0].metadata.node_type = kind.to_owned();
        let dto = CatalogDto::from_snapshot(&snapshot).unwrap();
        assert_eq!(dto.items[0].size, None);
        assert!(serde_json::to_value(&dto).unwrap()["items"][0]["size"].is_null());
    }
    snapshot.items[0].metadata.size = Some(0);
    assert_eq!(
        CatalogDto::from_snapshot(&snapshot).unwrap().items[0].size,
        Some(0)
    );

    snapshot.stores = vec![CatalogStoreSummary::Team {
        store: foks_agent_proto::TeamStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "group".to_owned(),
            team_id: "03".to_owned(),
        },
        kind: "mystery".to_owned(),
        name: Some("Group".to_owned()),
        active: true,
        creation_phase: None,
    }];
    assert_eq!(
        CatalogDto::from_snapshot(&snapshot).unwrap_err().code,
        "invalid-response"
    );
}

pub(super) struct MutationTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for MutationTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation);
        Ok(serde_json::Value::Null)
    }
}

#[test]
fn item_role_strings_are_strict_and_account_defaults_cannot_be_overridden() {
    let account = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    });
    assert_eq!(
        create_item_roles(&account, None, None).unwrap(),
        (KvRole::Owner, KvRole::Owner)
    );
    assert_eq!(
        create_item_roles(&account, Some("Owner"), Some("Owner"))
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let team = CatalogStoreRef::Team(foks_agent_proto::TeamStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "engineering".to_owned(),
        team_id: "03".repeat(33),
    });
    for roles in [(None, None), (Some("Owner"), None), (None, Some("Admin"))] {
        assert_eq!(
            create_item_roles(&team, roles.0, roles.1).unwrap_err().code,
            "invalid-request"
        );
    }
    assert_eq!(parse_item_role("Admin").unwrap(), KvRole::Admin);
    assert_eq!(
        parse_item_role("Member:-16384").unwrap(),
        KvRole::Member { visibility: -16384 }
    );
    for invalid in [
        "Member",
        "Member:32768",
        "Member:+1",
        "Member:00",
        "member:0",
        "Owner:0",
        " Admin",
    ] {
        assert_eq!(
            parse_item_role(invalid).unwrap_err().code,
            "invalid-request"
        );
    }
}

#[test]
fn team_create_edit_link_folder_and_remove_transcripts_bind_roles_and_guards() {
    let team = foks_agent_proto::TeamStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "engineering".to_owned(),
        team_id: "03".repeat(33),
    };
    let store = CatalogStoreRef::Team(team.clone());
    let (read_role, write_role) =
        create_item_roles(&store, Some("Member:0"), Some("Admin")).unwrap();
    let item = CatalogItem {
        store: store.clone(),
        metadata: foks_agent_proto::KvEntryMetadata {
            path: "/wifi/password".to_owned(),
            node_type: "small-file".to_owned(),
            version: 19,
            size: Some(3),
            read_role: KvRole::Member { visibility: -2 },
            write_role: KvRole::Admin,
        },
    };
    let transport = MutationTransport {
        calls: Mutex::new(Vec::new()),
    };
    execute_kv_mutation(
        &transport,
        set_create_mutation_roles(
            foks_desktop::create_kv_file_mutation(&store, "/new", b"new".to_vec()).unwrap(),
            read_role,
            write_role,
        )
        .unwrap(),
        MutationKind::Create,
    )
    .unwrap();
    foks_desktop::AgentTransport::call(
        &transport,
        set_create_operation_roles(
            foks_desktop::create_kv_symlink_operation(&store, "/docs", "/shared/docs").unwrap(),
            read_role,
            write_role,
        )
        .unwrap(),
    )
    .unwrap();
    foks_desktop::AgentTransport::call(
        &transport,
        set_create_operation_roles(
            foks_desktop::create_kv_directory_operation(&store, "/folder").unwrap(),
            read_role,
            write_role,
        )
        .unwrap(),
    )
    .unwrap();
    execute_kv_mutation(
        &transport,
        foks_desktop::edit_kv_file_mutation(&item, b"changed".to_vec()).unwrap(),
        MutationKind::Guarded,
    )
    .unwrap();
    foks_desktop::AgentTransport::call(&transport, remove_item_operation(&item).unwrap()).unwrap();
    let calls = transport.calls.lock().unwrap();
    assert_eq!(
        calls.as_slice(),
        &[
            Operation::PutKv {
                store: KvStoreRef::Team(team.clone()),
                path: "/new".to_owned(),
                content: b"new".to_vec(),
                read_role: KvRole::Member { visibility: 0 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::Create,
                mkdir_p: true,
            },
            Operation::PutKvSymlink {
                store: KvStoreRef::Team(team.clone()),
                path: "/docs".to_owned(),
                target: "/shared/docs".to_owned(),
                read_role: KvRole::Member { visibility: 0 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::Create,
                mkdir_p: true,
            },
            Operation::MkdirKv {
                store: KvStoreRef::Team(team.clone()),
                path: "/folder".to_owned(),
                read_role: KvRole::Member { visibility: 0 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::Create,
                mkdir_p: true,
            },
            Operation::PutKv {
                store: KvStoreRef::Team(team.clone()),
                path: "/wifi/password".to_owned(),
                content: b"changed".to_vec(),
                read_role: KvRole::Member { visibility: -2 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::ExactVersion { version: 19 },
                mkdir_p: false,
            },
            Operation::RemoveKv {
                store: KvStoreRef::Team(team.clone()),
                path: "/wifi/password".to_owned(),
                recursive: false,
                precondition: KvPrecondition::ExactVersion { version: 19 },
            },
        ]
    );
    drop(calls);
    let mut file = item.clone();
    file.metadata.node_type = "file".to_owned();
    let replacement = file_edit_header(&file, 84 * 1024 * 1024).unwrap();
    assert_eq!(replacement.store, KvStoreRef::Team(team));
    assert_eq!(replacement.read_role, KvRole::Member { visibility: -2 });
    assert_eq!(replacement.write_role, KvRole::Admin);
    assert_eq!(
        replacement.precondition,
        KvPrecondition::ExactVersion { version: 19 }
    );
}

pub(super) struct ConflictTransport {
    pub(super) calls: AtomicU64,
}

impl foks_desktop::AgentTransport for ConflictTransport {
    fn call(&self, _operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Err(foks_desktop::AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::Conflict,
            message: "precondition failed".to_owned(),
            fields: foks_agent_proto::ErrorFields::default(),
        })
    }
}

#[test]
fn team_create_and_edit_conflicts_are_returned_after_one_attempt_without_retry() {
    let team = foks_agent_proto::TeamStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "engineering".to_owned(),
        team_id: "03".repeat(33),
    };
    let store = CatalogStoreRef::Team(team.clone());
    let item = CatalogItem {
        store,
        metadata: foks_agent_proto::KvEntryMetadata {
            path: "/wifi/password".to_owned(),
            node_type: "small-file".to_owned(),
            version: 19,
            size: Some(3),
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
        },
    };
    let create_transport = ConflictTransport {
        calls: AtomicU64::new(0),
    };
    let create = set_create_mutation_roles(
        foks_desktop::create_kv_file_mutation(&item.store, "/new", b"new".to_vec()).unwrap(),
        KvRole::Member { visibility: 0 },
        KvRole::Admin,
    )
    .unwrap();
    let error = execute_kv_mutation(&create_transport, create, MutationKind::Create).unwrap_err();
    assert_eq!(error.code, "already-exists");
    assert_eq!(create_transport.calls.load(Ordering::Acquire), 1);

    let edit_transport = ConflictTransport {
        calls: AtomicU64::new(0),
    };
    let error = execute_kv_mutation(
        &edit_transport,
        foks_desktop::edit_kv_file_mutation(&item, b"new".to_vec()).unwrap(),
        MutationKind::Guarded,
    )
    .unwrap_err();
    assert_eq!(error.code, "conflict");
    assert_eq!(edit_transport.calls.load(Ordering::Acquire), 1);
}

#[test]
fn text_values_cannot_silently_turn_into_streamed_files() {
    assert_eq!(
        take_text_value("x".repeat(MAXIMUM_TEXT_ITEM_BYTES + 1))
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        take_text_value("x".repeat(MAXIMUM_TEXT_ITEM_BYTES))
            .unwrap()
            .len(),
        MAXIMUM_TEXT_ITEM_BYTES
    );
}

#[test]
fn renderer_text_edits_accept_only_small_file_catalog_nodes() {
    let mut item = download_item(1);
    assert_eq!(
        require_text_item(&item).unwrap_err().code,
        "invalid-request"
    );
    item.metadata.node_type = "symlink".to_owned();
    assert_eq!(
        require_text_item(&item).unwrap_err().code,
        "invalid-request"
    );
    item.metadata.node_type = "small-file".to_owned();
    assert!(require_text_item(&item).is_ok());
}

#[test]
fn native_file_replacement_accepts_both_file_encodings() {
    let mut item = download_item(1);
    assert!(require_file_item(&item).is_ok());
    item.metadata.node_type = "small-file".to_owned();
    assert!(require_file_item(&item).is_ok());
    item.metadata.node_type = "symlink".to_owned();
    assert_eq!(
        require_file_item(&item).unwrap_err().code,
        "invalid-request"
    );
}

#[test]
fn renderer_remove_boundary_rejects_folders_and_is_never_recursive() {
    let file = download_item(1);
    assert!(matches!(
        remove_item_operation(&file).unwrap(),
        Operation::RemoveKv {
            recursive: false,
            precondition: KvPrecondition::ExactVersion { version: 9 },
            ..
        }
    ));
    let mut folder = file;
    folder.metadata.node_type = "directory".to_owned();
    assert_eq!(
        remove_item_operation(&folder).unwrap_err().code,
        "invalid-request"
    );
}

pub(super) struct UploadTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
    pub(super) header: Mutex<Option<KvUploadHeader>>,
    pub(super) total_read: AtomicU64,
    pub(super) largest_read: AtomicU64,
}

impl foks_desktop::AgentTransport for UploadTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation);
        Ok(serde_json::Value::Null)
    }

    fn put_kv_stream(
        &self,
        header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        let total = header.total_length;
        *self.header.lock().unwrap() = Some(header);
        let mut buffer = [0u8; 128 * 1024];
        let mut seen = 0u64;
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))?;
            if count == 0 {
                break;
            }
            seen += count as u64;
            self.largest_read.fetch_max(count as u64, Ordering::AcqRel);
        }
        assert_eq!(seen, total);
        self.total_read.store(seen, Ordering::Release);
        Ok(serde_json::Value::Null)
    }
}

#[test]
fn eighty_four_megabyte_drop_uses_only_the_bounded_stream_boundary() {
    const TOTAL: u64 = 84 * 1024 * 1024;
    let source = tempfile::NamedTempFile::new().unwrap();
    source.as_file().set_len(TOTAL).unwrap();
    let team = foks_agent_proto::TeamStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "engineering".to_owned(),
        team_id: "03".repeat(33),
    };
    let store = CatalogStoreRef::Team(team.clone());
    let transport = UploadTransport {
        calls: Mutex::new(Vec::new()),
        header: Mutex::new(None),
        total_read: AtomicU64::new(0),
        largest_read: AtomicU64::new(0),
    };

    let header =
        file_create_header(&store, "/bundle.tar", 0, Some("Member:0"), Some("Admin")).unwrap();
    upload_file(&transport, header, source.path(), MutationKind::Create).unwrap();

    assert!(transport.calls.lock().unwrap().is_empty());
    let header = transport.header.lock().unwrap().clone().unwrap();
    assert_eq!(header.store, KvStoreRef::Team(team));
    assert_eq!(header.read_role, KvRole::Member { visibility: 0 });
    assert_eq!(header.write_role, KvRole::Admin);
    assert_eq!(header.total_length, TOTAL);
    assert_eq!(header.precondition, KvPrecondition::Create);
    assert_eq!(transport.total_read.load(Ordering::Acquire), TOTAL);
    assert!(transport.largest_read.load(Ordering::Acquire) <= 128 * 1024);
}

pub(super) struct EarlyUploadLoss;

impl foks_desktop::AgentTransport for EarlyUploadLoss {
    fn call(&self, _operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        unreachable!("file upload must use put_kv_stream")
    }

    fn put_kv_stream(
        &self,
        _header: KvUploadHeader,
        reader: &mut dyn std::io::Read,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        let mut first_frame = [0u8; 64 * 1024];
        reader
            .read_exact(&mut first_frame)
            .map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))?;
        Err(foks_desktop::AgentError::Transport(
            "socket disappeared before the next frame".to_owned(),
        ))
    }
}

#[test]
fn early_transport_loss_with_an_unchanged_source_stays_agent_lost() {
    let source = tempfile::NamedTempFile::new().unwrap();
    source.as_file().set_len(1024 * 1024).unwrap();
    let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    });
    let header = file_create_header(&store, "/bundle.tar", 0, None, None).unwrap();

    let error = upload_file(
        &EarlyUploadLoss,
        header,
        source.path(),
        MutationKind::Create,
    )
    .unwrap_err();

    assert_eq!(error.code, "agent-lost");
    assert!(error.fatal);
}

pub(super) struct ResizeDuringUpload {
    pub(super) path: PathBuf,
    pub(super) length: u64,
}

impl foks_desktop::AgentTransport for ResizeDuringUpload {
    fn call(&self, _operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        unreachable!("file upload must use put_kv_stream")
    }

    fn put_kv_stream(
        &self,
        _header: KvUploadHeader,
        _reader: &mut dyn std::io::Read,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        OpenOptions::new()
            .write(true)
            .open(&self.path)
            .unwrap()
            .set_len(self.length)
            .unwrap();
        Err(foks_desktop::AgentError::Transport(
            "upload length no longer matches".to_owned(),
        ))
    }
}

#[test]
fn same_handle_length_changes_are_classified_as_source_changes() {
    const ORIGINAL: u64 = 1024 * 1024;
    let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    });
    for changed in [ORIGINAL / 2, ORIGINAL * 2] {
        let source = tempfile::NamedTempFile::new().unwrap();
        source.as_file().set_len(ORIGINAL).unwrap();
        let header = file_create_header(&store, "/bundle.tar", 0, None, None).unwrap();
        let error = upload_file(
            &ResizeDuringUpload {
                path: source.path().to_owned(),
                length: changed,
            },
            header,
            source.path(),
            MutationKind::Create,
        )
        .unwrap_err();
        assert_eq!(error.code, "upload-source-changed");
    }
}

#[test]
fn catalog_preserves_incomplete_creation_phase() {
    let snapshot = CatalogSnapshot {
        stores: vec![CatalogStoreSummary::Team {
            store: foks_agent_proto::TeamStoreRef {
                profile: "foks.example".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "group".to_owned(),
                team_id: "03".to_owned(),
            },
            kind: "named".to_owned(),
            name: Some("Group".to_owned()),
            active: false,
            creation_phase: Some("preparing".to_owned()),
        }],
        ..CatalogSnapshot::default()
    };
    let dto = CatalogDto::from_snapshot(&snapshot).unwrap();
    assert_eq!(dto.stores[0].creation_phase.as_deref(), Some("preparing"));
    assert_eq!(
        serde_json::to_value(dto).unwrap()["stores"][0]["creation_phase"],
        "preparing"
    );
}
