use crate::commands::execution::{execute_kv_mutation, MutationKind};
use crate::commands::validation::{DOWNLOAD_CHUNK_BYTES, MAXIMUM_TEXT_ITEM_BYTES};
use crate::commands::vault::{
    create_item_roles, file_create_header, file_edit_header, item_field, parse_item_role,
    read_store_usage, read_text, require_file_item, require_text_item, set_create_mutation_roles,
    set_create_operation_roles, take_text_value, upload_file, website_url, CatalogDto, ItemField,
};
use foks_agent_proto::{
    KvChunkResult, KvPrecondition, KvReadResult, KvRole, KvStoreRef, KvUploadHeader, Operation,
};
use foks_desktop::{CatalogItem, CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

/// Answers one whole-catalog walk over two configured profiles, one of which
/// cannot be reached. Every call is recorded so a read can be counted.
struct CatalogReadTransport {
    calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for CatalogReadTransport {
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles => Ok(serde_json::json!([
                {"name": "work.example", "probe": "work.example",
                 "protocol": {"generation": "v019"}, "trust": {"kind": "web-pki"}},
                {"name": "down.example", "probe": "down.example",
                 "protocol": {"generation": "v019"}, "trust": {"kind": "web-pki"}},
            ])),
            Operation::ListKnownStores { .. } => Ok(serde_json::json!([])),
            Operation::ListProfileOverview { profile } if profile == "down.example" => Err(
                foks_desktop::AgentError::Transport("profile unreachable".into()),
            ),
            Operation::ListProfileOverview { profile } => {
                Ok(serde_json::to_value(foks_agent_proto::ProfileOverview {
                    profile: profile.clone(),
                    accounts: foks_agent_proto::ResponseResult::Success {
                        value: serde_json::json!([
                            {"profile": profile, "alias": "personal", "username": "alice"}
                        ]),
                    },
                    teams: foks_agent_proto::ResponseResult::Success {
                        value: serde_json::json!([]),
                    },
                    server_status: foks_agent_proto::ResponseResult::Success {
                        value: serde_json::json!({
                            "profile": profile, "configured_probe": profile, "host": null,
                            "compatibility": {"status": "not-required"}, "chat_supported": null,
                        }),
                    },
                })
                .unwrap())
            }
            Operation::ListKv { .. } => Ok(serde_json::to_value(foks_agent_proto::KvPage {
                snapshot_version: 1,
                entries: vec![],
                next_cursor: None,
            })
            .unwrap()),
            other => panic!("unexpected catalog operation {other:?}"),
        }
    }
}

#[test]
fn a_whole_catalog_read_lists_profiles_once_and_carries_local_metadata() {
    let transport = Arc::new(CatalogReadTransport {
        calls: Mutex::new(vec![]),
    });
    let profiles = crate::commands::vault::catalog_profile_listing(transport.as_ref()).unwrap();
    let snapshot = foks_desktop::load_catalog_cancellable_with_profiles(
        transport.clone(),
        profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect(),
        foks_desktop::CatalogLoadToken::default(),
        false,
    )
    .unwrap();
    assert_eq!(
        transport
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, Operation::ListProfiles))
            .count(),
        1,
        "the preloaded listing is the only one the read issues"
    );
    let dto = crate::commands::vault::catalog_dto(&snapshot, 7, Some(&profiles)).unwrap();
    assert_eq!(dto.generation, 7);
    let encoded = serde_json::to_value(&dto).unwrap();
    assert_eq!(
        encoded["localMetadata"]["profiles"][0]["configuredEndpoint"],
        "work.example"
    );
    assert!(encoded["localMetadata"]["profiles"][0]
        .get("configuredProbe")
        .is_none());
    let metadata = dto.local_metadata.expect("the read attaches its metadata");
    // The metadata is built from the registry listing, so the profile whose
    // read failed is still a configured server the renderer can draw.
    assert_eq!(
        metadata
            .profiles
            .iter()
            .map(|profile| profile.profile.as_str())
            .collect::<Vec<_>>(),
        ["work.example", "down.example"]
    );
    assert!(metadata.profiles[0].status.is_some());
    assert!(metadata.profiles[1].status.is_none());
    assert!(metadata.profiles[1].error.is_none());
    assert_eq!(metadata.accounts.len(), 1);
    assert_eq!(metadata.accounts[0].profile, "work.example");
    assert!(dto
        .failures
        .iter()
        .any(|failure| failure.profile.as_deref() == Some("down.example")));
}

#[test]
fn a_store_only_read_attaches_no_local_metadata() {
    let transport = Arc::new(CatalogReadTransport {
        calls: Mutex::new(vec![]),
    });
    let snapshot =
        foks_desktop::load_stores_cancellable(transport.clone(), Default::default()).unwrap();
    let dto = crate::commands::vault::catalog_dto(&snapshot, 3, None).unwrap();
    assert!(dto.local_metadata.is_none());
    // The walk still lists the profiles for itself; preloading the listing for
    // an item read did not add a second one here.
    assert_eq!(
        transport
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, Operation::ListProfiles))
            .count(),
        1
    );
    assert!(!transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Operation::ListKv { .. })));
}

pub(super) const READ_VALUE: &[u8] = b"guest-password";

pub(super) struct ReadTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for ReadTransport {
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ReadKv {
                store,
                path,
                dirent_id,
                version,
            } => serde_json::to_value(KvReadResult {
                store,
                path,
                dirent_id,
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
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        let value = match operation {
            Operation::ReadKv {
                store,
                path,
                dirent_id,
                version,
            } => serde_json::to_value(KvReadResult {
                store,
                path,
                dirent_id,
                version,
                node_type: "file".to_owned(),
                // Model a legacy file without the optional size extension;
                // the download loop must still use its end-of-file flag.
                size: None,
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                content: None,
                symlink_target: None,
            }),
            Operation::ReadKvChunk {
                store,
                path,
                dirent_id,
                version,
                offset,
                length,
            } => {
                let count = usize::try_from((self.total - offset).min(u64::from(length))).unwrap();
                serde_json::to_value(KvChunkResult {
                    store,
                    path,
                    dirent_id,
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

pub(super) fn download_item(_total: u64) -> CatalogItem {
    let store = foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    };
    CatalogItem {
        store: CatalogStoreRef::Account(store),
        metadata: foks_agent_proto::KvEntryMetadata {
            dirent_id: [7; 16],
            path: "/large.bin".to_owned(),
            node_type: "file".to_owned(),
            version: 9,
            // The catalog never carries a size; a fixture that does hides the
            // binding defect this shape used to trigger.
            size: None,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            shape: None,
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
            dirent_id: [7; 16],
            path: "/wifi/password".to_owned(),
            node_type: "small-file".to_owned(),
            version: 7,
            size: None,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            shape: None,
        },
    };
    let transport = Arc::new(ReadTransport {
        calls: Mutex::new(Vec::new()),
    });
    let lock = crate::applock::AppLock::unlocked_for_test();
    let read = read_text(transport.clone(), &item, &lock.interval_for_test()).unwrap();

    assert_eq!(read.value.as_str(), "guest-password");
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[Operation::ReadKv {
            store: KvStoreRef::Account(foks_agent_proto::AccountStoreRef {
                profile: "foks.example".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            path: "/wifi/password".to_owned(),
            dirent_id: [7; 16],
            version: 7,
        }]
    );
}

#[test]
fn download_streams_files_larger_than_the_reveal_limit_in_bound_chunks() {
    let total = 16 * 1024 * 1024 + 1;
    let item = download_item(total);
    let transport = Arc::new(DownloadTransport {
        total,
        calls: Mutex::new(Vec::new()),
    });
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("large.bin");
    let lock = crate::applock::AppLock::unlocked_for_test();
    download_to_path(
        transport.clone(),
        &item,
        &destination,
        &lock.interval_for_test(),
    )
    .unwrap();
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
                dirent_id: [7; 16],
                path: "/bad".to_owned(),
                node_type: "socket".to_owned(),
                version: 1,
                size: Some(1),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                shape: None,
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
        chain_seqno: None,
    }];
    assert_eq!(
        CatalogDto::from_snapshot(&snapshot).unwrap_err().code,
        "invalid-response"
    );
}

#[test]
fn catalog_dto_carries_the_listed_shape_and_omits_it_otherwise() {
    let mut item = download_item(0);
    item.metadata.node_type = "small-file".to_owned();
    item.metadata.shape = Some(foks_agent_proto::KvEntryShape::Login);
    let mut snapshot = CatalogSnapshot {
        profiles: vec!["foks.example".to_owned()],
        stores: vec![CatalogStoreSummary::Account {
            store: foks_agent_proto::AccountStoreRef {
                profile: "foks.example".to_owned(),
                account_alias: "personal".to_owned(),
            },
        }],
        items: vec![item],
        ..CatalogSnapshot::default()
    };
    let encoded = |snapshot: &CatalogSnapshot| {
        serde_json::to_value(CatalogDto::from_snapshot(snapshot).unwrap()).unwrap()["items"][0]
            .clone()
    };
    assert_eq!(encoded(&snapshot)["shape"], "login");
    snapshot.items[0].metadata.shape = Some(foks_agent_proto::KvEntryShape::Other);
    assert_eq!(encoded(&snapshot)["shape"], "other");
    snapshot.items[0].metadata.shape = None;
    assert!(encoded(&snapshot).get("shape").is_none());
}

#[test]
fn item_field_selects_one_login_field_or_reports_it_missing() {
    let stored =
        || zeroize::Zeroizing::new("user: satoshi\npassword: hx7-Qm2!vTe9\nurl: ".to_owned());
    assert_eq!(
        item_field(stored(), ItemField::Password).unwrap().as_str(),
        "hx7-Qm2!vTe9"
    );
    assert_eq!(
        item_field(stored(), ItemField::Username).unwrap().as_str(),
        "satoshi"
    );
    assert_eq!(item_field(stored(), ItemField::Value).unwrap(), stored());
    // A login written without a website still has its empty url line.
    let missing = item_field(stored(), ItemField::Url).unwrap_err();
    assert_eq!(missing.code, "field-missing");
    assert_eq!(missing.message, "This item has no website.");
    assert!(!missing.retryable);
    // Only the separator space is removed, so a padded password is copied whole.
    let padded = zeroize::Zeroizing::new("user: a\npassword:  pad-me \r\nurl: ".to_owned());
    assert_eq!(
        item_field(padded, ItemField::Password).unwrap().as_str(),
        " pad-me "
    );
    let note = zeroize::Zeroizing::new("postgres://service:o4Kq@db/prod".to_owned());
    assert_eq!(
        item_field(note, ItemField::Password).unwrap_err().code,
        "field-missing"
    );
    for (name, field) in [
        ("password", ItemField::Password),
        ("username", ItemField::Username),
        ("url", ItemField::Url),
        ("value", ItemField::Value),
    ] {
        assert_eq!(
            serde_json::from_value::<ItemField>(serde_json::json!(name)).unwrap(),
            field
        );
    }
    assert!(serde_json::from_value::<ItemField>(serde_json::json!("user")).is_err());
}

#[test]
fn website_url_opens_only_http_and_https_addresses() {
    for (field, expected) in [
        ("https://github.com/login", "https://github.com/login"),
        ("http://router.local:8080/", "http://router.local:8080/"),
        // A bare host is taken as https, and the padding a field may keep is
        // not part of the address.
        ("github.com/login", "https://github.com/login"),
        (" example.com:8443 ", "https://example.com:8443/"),
        ("HTTPS://Example.com", "https://example.com/"),
        // A `://` after the host, here in a sign-in redirect, is not a scheme.
        (
            "accounts.example.com/signin?continue=https://mail.example.com/",
            "https://accounts.example.com/signin?continue=https://mail.example.com/",
        ),
        (
            "https://example.com/a?next=https://b",
            "https://example.com/a?next=https://b",
        ),
    ] {
        assert_eq!(
            website_url(field).map(|url| url.to_string()).as_deref(),
            Some(expected),
            "{field}"
        );
    }
    for field in [
        "",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "ftp://files.example.com",
        "mailto:a@example.com",
        "https://user:secret@example.com",
        "github.com/a b",
        "data:text/html,a",
    ] {
        assert!(website_url(field).is_none(), "{field}");
    }
}

pub(super) struct MutationTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for MutationTransport {
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

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
fn team_create_edit_folder_and_remove_transcripts_bind_roles_and_guards() {
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
            dirent_id: [7; 16],
            path: "/wifi/password".to_owned(),
            node_type: "small-file".to_owned(),
            version: 19,
            size: Some(3),
            read_role: KvRole::Member { visibility: -2 },
            write_role: KvRole::Admin,
            shape: None,
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
            foks_desktop::create_kv_directory_operation(&store, "/folder").unwrap(),
            read_role,
            write_role,
        )
        .unwrap(),
    )
    .unwrap();
    // The app creates no symlinks, so the create-role setter refuses one.
    assert_eq!(
        set_create_operation_roles(
            foks_desktop::create_kv_symlink_operation(&store, "/docs", "/shared/docs").unwrap(),
            read_role,
            write_role,
        )
        .unwrap_err()
        .code,
        "invalid-builder"
    );
    execute_kv_mutation(
        &transport,
        foks_desktop::edit_kv_file_mutation(&item, b"changed".to_vec()).unwrap(),
        MutationKind::Guarded,
    )
    .unwrap();
    foks_desktop::AgentTransport::call(
        &transport,
        foks_desktop::remove_kv_operation(&item, false).unwrap(),
    )
    .unwrap();
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
                precondition: KvPrecondition::ExactEntry {
                    dirent_id: [7; 16],
                    version: 19
                },
                mkdir_p: false,
            },
            Operation::RemoveKv {
                expected_dirent_id: Some([7; 16]),
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
        KvPrecondition::ExactEntry {
            dirent_id: [7; 16],
            version: 19
        }
    );
}

pub(super) struct ConflictTransport {
    pub(super) calls: AtomicU64,
}

impl foks_desktop::AgentTransport for ConflictTransport {
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, _operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Err(foks_desktop::AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::Conflict,
            message: "precondition failed".to_owned(),
            fields: Box::default(),
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
            dirent_id: [7; 16],
            path: "/wifi/password".to_owned(),
            node_type: "small-file".to_owned(),
            version: 19,
            size: Some(3),
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            shape: None,
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

pub(super) struct UploadTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
    pub(super) header: Mutex<Option<KvUploadHeader>>,
    pub(super) total_read: AtomicU64,
    pub(super) largest_read: AtomicU64,
}

impl foks_desktop::AgentTransport for UploadTransport {
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        let issuance = matches!(operation, Operation::KvUploadIssuance { .. });
        self.calls.lock().unwrap().push(operation);
        Ok(if issuance {
            serde_json::json!({"database_id":vec![1;16],"issued_at":1})
        } else {
            serde_json::Value::Null
        })
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

    assert!(matches!(
        transport.calls.lock().unwrap().as_slice(),
        [Operation::KvUploadIssuance { .. }]
    ));
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
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, _operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        Ok(serde_json::json!({"database_id":vec![1;16],"issued_at":1}))
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
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, _operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        Ok(serde_json::json!({"database_id":vec![1;16],"issued_at":1}))
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
            chain_seqno: None,
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

#[test]
fn move_preserves_exact_source_and_invalidates_its_store() {
    let item = download_item(0);
    let operation = foks_desktop::move_kv_operation(&item, "/renamed").unwrap();
    assert!(matches!(&operation, Operation::MoveKv { dirent_id, .. }
        if *dirent_id == item.metadata.dirent_id));
    assert!(
        matches!(&operation, Operation::MoveKv { path, destination, version, .. }
        if path == &item.metadata.path && destination == "/renamed" && *version == item.metadata.version)
    );
    assert_eq!(
        crate::commands::execution::kv_mutation_store(&foks_desktop::KvAccountMutation::Inline(
            operation
        )),
        Some(item.store.clone())
    );
    assert!(foks_desktop::move_kv_operation(&item, &item.metadata.path).is_err());
    assert!(foks_desktop::move_kv_operation(&item, "relative").is_err());
}

/// Answers the account binding and the usage read, recording each call. The
/// binding names `bound_alias`, so a test can make it disagree with the vault.
struct UsageTransport {
    calls: Mutex<Vec<Operation>>,
    bound_alias: &'static str,
}

impl UsageTransport {
    fn new(bound_alias: &'static str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            bound_alias,
        }
    }
}

impl foks_desktop::AgentTransport for UsageTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::BindDataAccount { profile, .. } => Ok(serde_json::json!({
                "profile": profile, "account_alias": self.bound_alias,
                "host_id": "0b", "user_id": "01", "team_id": null,
            })),
            Operation::ReadData { .. } => Ok(serde_json::json!({"files": 3, "bytes": 2_469_396})),
            other => panic!("unexpected operation {other:?}"),
        }
    }
}

fn bound_scope(team_id: Option<&str>) -> foks_agent_proto::data::DataScope {
    foks_agent_proto::data::DataScope {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        host_id: "0b".to_owned(),
        user_id: "01".to_owned(),
        team_id: team_id.map(str::to_owned),
    }
}

#[test]
fn store_usage_reads_the_bound_account_or_its_team() {
    let account = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    });
    let team = CatalogStoreRef::Team(foks_agent_proto::TeamStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "group".to_owned(),
        team_id: "0d".repeat(33),
    });
    for (store, team_id) in [(account, None), (team, Some("0d".repeat(33)))] {
        let transport = UsageTransport::new("personal");
        let usage = read_store_usage(&transport, &store).unwrap();
        assert_eq!(
            serde_json::to_value(usage).unwrap(),
            serde_json::json!({"files": 3, "bytes": 2_469_396})
        );
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::BindDataAccount {
                    profile: "foks.example".to_owned(),
                    account_alias: "personal".to_owned(),
                },
                Operation::ReadData {
                    scope: bound_scope(team_id.as_deref()),
                    query: foks_agent_proto::data::DataRead::Usage,
                },
            ]
        );
    }
}

#[test]
fn store_usage_refuses_a_binding_for_another_account() {
    let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    });
    let transport = UsageTransport::new("other");
    assert_eq!(
        read_store_usage(&transport, &store).unwrap_err().code,
        "invalid-response"
    );
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}

// Checkpoints pause the real worker at named boundaries; no timing assumptions
// or sleeps decide whether invalidation wins.
fn publication_barrier(
    lock: &Arc<crate::applock::AppLock>,
    phase: &'static str,
) -> (
    crate::applock::UnlockedInterval,
    Arc<std::sync::Barrier>,
    Arc<std::sync::Barrier>,
) {
    let reached = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let worker_reached = reached.clone();
    let worker_resume = resume.clone();
    let once = std::sync::atomic::AtomicBool::new(false);
    let interval = lock.interval_for_test().with_checkpoint(move |actual| {
        if actual == phase && !once.swap(true, Ordering::AcqRel) {
            worker_reached.wait();
            worker_resume.wait();
        }
    });
    (interval, reached, resume)
}

#[test]
fn locking_during_secret_read_rejects_plaintext_even_after_unlock() {
    for unlock in [false, true] {
        let lock = crate::applock::AppLock::unlocked_for_test();
        let (interval, reached, resume) = publication_barrier(&lock, "after-agent-read");
        let mut item = download_item(READ_VALUE.len() as u64);
        item.metadata.node_type = "small-file".into();
        let worker = std::thread::spawn(move || {
            read_text(
                Arc::new(ReadTransport {
                    calls: Mutex::new(Vec::new()),
                }),
                &item,
                &interval,
            )
        });
        reached.wait();
        lock.invalidate_for_test(unlock);
        resume.wait();
        assert_eq!(worker.join().unwrap().unwrap_err().code, "app-locked");
    }
}

#[test]
fn lock_after_dialog_rejects_selection_before_reading_or_creating_a_file() {
    for unlock in [false, true] {
        let lock = crate::applock::AppLock::unlocked_for_test();
        let (interval, reached, resume) = publication_barrier(&lock, "after-dialog");
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("new.bin");
        let worker_destination = destination.clone();
        let transport = Arc::new(DownloadTransport {
            total: 1,
            calls: Mutex::new(Vec::new()),
        });
        let worker_transport = transport.clone();
        let worker = std::thread::spawn(move || {
            let path = crate::commands::vault::select_download_destination(&interval, || {
                worker_destination
            })?;
            download_to_path(worker_transport, &download_item(1), &path, &interval)
        });
        reached.wait();
        lock.invalidate_for_test(unlock);
        resume.wait();
        assert_eq!(worker.join().unwrap().unwrap_err().code, "app-locked");
        assert!(transport.calls.lock().unwrap().is_empty());
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn lock_during_download_never_creates_or_replaces_destination_and_cleans_temporary() {
    for phase in ["after-agent-read", "between-chunks", "before-publication"] {
        for unlock in [false, true] {
            for existing in [false, true] {
                let lock = crate::applock::AppLock::unlocked_for_test();
                let (interval, reached, resume) = publication_barrier(&lock, phase);
                let directory = tempfile::tempdir().unwrap();
                let destination = directory.path().join("saved.bin");
                if existing {
                    std::fs::write(&destination, b"existing destination").unwrap();
                }
                let worker_destination = destination.clone();
                let total = u64::from(DOWNLOAD_CHUNK_BYTES) + 1;
                let transport = Arc::new(DownloadTransport {
                    total,
                    calls: Mutex::new(Vec::new()),
                });
                let worker_transport = transport.clone();
                let worker = std::thread::spawn(move || {
                    download_to_path(
                        worker_transport,
                        &download_item(total),
                        &worker_destination,
                        &interval,
                    )
                });
                reached.wait();
                if phase == "before-publication" {
                    assert_eq!(transport.calls.lock().unwrap().len(), 3);
                    // A fully prepared temporary file exists, but the final
                    // destination has not been touched while the gate is pending.
                    assert_eq!(
                        std::fs::read_dir(directory.path()).unwrap().count(),
                        usize::from(existing) + 1
                    );
                }
                lock.invalidate_for_test(unlock);
                resume.wait();
                assert_eq!(worker.join().unwrap().unwrap_err().code, "app-locked");
                if existing {
                    assert_eq!(
                        std::fs::read(&destination).unwrap(),
                        b"existing destination"
                    );
                } else {
                    assert!(!destination.exists());
                }
                assert_eq!(
                    std::fs::read_dir(directory.path()).unwrap().count(),
                    usize::from(existing)
                );
                if phase == "between-chunks" {
                    assert_eq!(
                        transport.calls.lock().unwrap().len(),
                        2,
                        "no second chunk read after invalidation"
                    );
                }
            }
        }
    }
}

#[test]
fn lock_immediately_before_plaintext_or_clipboard_publication_rejects_old_interval() {
    for unlock in [false, true] {
        for clipboard in [false, true] {
            let lock = crate::applock::AppLock::unlocked_for_test();
            let (interval, reached, resume) = publication_barrier(&lock, "before-publication");
            let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let worker_writes = writes.clone();
            let worker = std::thread::spawn(move || {
                let mut item = download_item(READ_VALUE.len() as u64);
                item.metadata.node_type = "small-file".into();
                let read = read_text(
                    Arc::new(ReadTransport {
                        calls: Mutex::new(Vec::new()),
                    }),
                    &item,
                    &interval,
                )?;
                interval.publish(|| {
                    if clipboard {
                        worker_writes.fetch_add(1, Ordering::AcqRel);
                    }
                    Ok(read)
                })
            });
            reached.wait();
            lock.invalidate_for_test(unlock);
            resume.wait();
            assert_eq!(worker.join().unwrap().unwrap_err().code, "app-locked");
            assert_eq!(writes.load(Ordering::Acquire), 0);
        }
    }
}

struct BlockedReadTransport {
    inner: Arc<dyn foks_desktop::AgentTransport>,
    chunk: bool,
    reached: Arc<std::sync::Barrier>,
    resume: Arc<std::sync::Barrier>,
}
impl foks_desktop::AgentTransport for BlockedReadTransport {
    fn call_with_deadline(
        &self,
        mut operation: foks_agent_proto::Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if let Err(error) = deadline.check() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::local_deadline_error(error));
        }
        self.call_cancellable(operation, cancelled)
    }
    fn put_kv_stream_with_deadline(
        &self,
        header: foks_agent_proto::KvUploadHeader,
        reader: &mut dyn std::io::Read,
        deadline: &foks_local_deadline::Deadline,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        deadline
            .check()
            .map_err(foks_desktop::local_deadline_error)?;
        self.put_kv_stream(header, reader)
    }

    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        let block = if self.chunk {
            matches!(&operation, Operation::ReadKvChunk { offset: 0, .. })
        } else {
            matches!(&operation, Operation::ReadKv { .. })
        };
        let result = self.inner.call(operation);
        if block {
            self.reached.wait();
            self.resume.wait();
        }
        result
    }
}

#[test]
fn blocked_agent_reads_cannot_return_secret_or_request_more_chunks_after_lock() {
    for unlock in [false, true] {
        for chunk in [false, true] {
            let lock = crate::applock::AppLock::unlocked_for_test();
            let interval = lock.interval_for_test();
            let reached = Arc::new(std::sync::Barrier::new(2));
            let resume = Arc::new(std::sync::Barrier::new(2));
            let download = Arc::new(DownloadTransport {
                total: u64::from(DOWNLOAD_CHUNK_BYTES) + 1,
                calls: Mutex::new(Vec::new()),
            });
            let inner: Arc<dyn foks_desktop::AgentTransport> = if chunk {
                download.clone()
            } else {
                Arc::new(ReadTransport {
                    calls: Mutex::new(Vec::new()),
                })
            };
            let transport = Arc::new(BlockedReadTransport {
                inner,
                chunk,
                reached: reached.clone(),
                resume: resume.clone(),
            });
            let worker = std::thread::spawn(move || {
                let mut item = download_item(0);
                if !chunk {
                    item.metadata.node_type = "small-file".into();
                }
                read_text(transport, &item, &interval)
            });
            reached.wait();
            lock.invalidate_for_test(unlock);
            resume.wait();
            assert_eq!(worker.join().unwrap().unwrap_err().code, "app-locked");
            if chunk {
                assert_eq!(download.calls.lock().unwrap().len(), 2);
            }
        }
    }
}

#[test]
fn download_rejects_changed_entry_identity_without_publishing_or_leaving_temporary_files() {
    struct Replacement {
        inner: DownloadTransport,
        between_chunks: bool,
    }
    impl foks_desktop::AgentTransport for Replacement {
        fn call_with_deadline(
            &self,
            mut operation: foks_agent_proto::Operation,
            deadline: &foks_local_deadline::Deadline,
            cancelled: &dyn Fn() -> bool,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            if let Err(error) = deadline.check() {
                operation.zeroize_plaintext();
                return Err(foks_desktop::local_deadline_error(error));
            }
            self.call_cancellable(operation, cancelled)
        }
        fn put_kv_stream_with_deadline(
            &self,
            header: foks_agent_proto::KvUploadHeader,
            reader: &mut dyn std::io::Read,
            deadline: &foks_local_deadline::Deadline,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            deadline
                .check()
                .map_err(foks_desktop::local_deadline_error)?;
            self.put_kv_stream(header, reader)
        }

        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            let replace = if self.between_chunks {
                matches!(&operation, Operation::ReadKvChunk { offset, .. } if *offset > 0)
            } else {
                matches!(&operation, Operation::ReadKv { .. })
            };
            let mut value = foks_desktop::AgentTransport::call(&self.inner, operation)?;
            if replace {
                value["dirent_id"] = serde_json::to_value([8; 16]).unwrap();
            }
            Ok(value)
        }
    }
    for between_chunks in [false, true] {
        for existing in [false, true] {
            let total = u64::from(DOWNLOAD_CHUNK_BYTES) + 17;
            let item = download_item(total);
            let transport = Replacement {
                inner: DownloadTransport {
                    total,
                    calls: Mutex::new(Vec::new()),
                },
                between_chunks,
            };
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("download.bin");
            if existing {
                std::fs::write(&destination, b"keep existing").unwrap();
            }
            let lock = crate::applock::AppLock::unlocked_for_test();
            assert!(download_to_path(
                Arc::new(transport),
                &item,
                &destination,
                &lock.interval_for_test()
            )
            .is_err());
            if existing {
                assert_eq!(std::fs::read(&destination).unwrap(), b"keep existing");
            } else {
                assert!(!destination.exists());
            }
            assert_eq!(
                std::fs::read_dir(directory.path()).unwrap().count(),
                usize::from(existing)
            );
        }
    }
}

fn download_to_path(
    transport: Arc<dyn foks_desktop::AgentTransport>,
    item: &CatalogItem,
    destination: &std::path::Path,
    interval: &crate::applock::UnlockedInterval,
) -> Result<(), crate::agent::AgentError> {
    let deadline = foks_local_deadline::Deadline::capture(
        std::time::Instant::now() + std::time::Duration::from_secs(300),
    )
    .unwrap();
    crate::commands::vault::download_to_path(
        transport.as_ref(),
        item,
        destination,
        interval,
        &deadline,
    )
}

#[tokio::test]
async fn expired_download_never_publishes_and_cleans_its_temporary_file() {
    for phase in ["after-agent-read", "between-chunks", "before-publication"] {
        for existing in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("download.bin");
            if existing {
                std::fs::write(&destination, b"original destination").unwrap();
            }
            let lock = crate::applock::AppLock::unlocked_for_test();
            let (reached_tx, reached_rx) = tokio::sync::oneshot::channel();
            let reached_tx = Mutex::new(Some(reached_tx));
            let (resume_tx, resume_rx) = std::sync::mpsc::channel();
            let resume_rx = Mutex::new(resume_rx);
            let interval = lock.interval_for_test().with_checkpoint(move |actual| {
                if actual == phase {
                    if let Some(tx) = reached_tx.lock().unwrap().take() {
                        tx.send(()).unwrap();
                        resume_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(std::time::Duration::from_secs(10))
                            .unwrap();
                    }
                }
            });
            let deadline = foks_local_deadline::Deadline::capture(
                std::time::Instant::now() + std::time::Duration::from_secs(1),
            )
            .unwrap();
            let worker_deadline = deadline.clone();
            let worker_destination = destination.clone();
            let worker = tokio::task::spawn_blocking(move || {
                let total = u64::from(DOWNLOAD_CHUNK_BYTES) + 17;
                crate::commands::vault::download_to_path(
                    &DownloadTransport {
                        total,
                        calls: Mutex::new(Vec::new()),
                    },
                    &download_item(total),
                    &worker_destination,
                    &interval,
                    &worker_deadline,
                )
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), reached_rx)
                .await
                .unwrap()
                .unwrap();
            // The barrier chooses the boundary; the absolute expiry chooses release.
            tokio::time::sleep_until(deadline.instant().into()).await;
            assert!(deadline.check().is_err());
            resume_tx.send(()).unwrap();
            assert!(worker.await.unwrap().is_err());
            if existing {
                assert_eq!(
                    std::fs::read(&destination).unwrap(),
                    b"original destination"
                );
            } else {
                assert!(!destination.exists());
            }
            assert_eq!(
                std::fs::read_dir(directory.path()).unwrap().count(),
                usize::from(existing)
            );
        }
    }
}
