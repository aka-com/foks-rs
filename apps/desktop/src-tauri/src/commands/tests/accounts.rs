use crate::agent::AgentHandle;
use crate::commands::accounts::{
    load_account_devices, load_accounts, load_backup_enrollments, AccountDto, DeviceDto,
};
use crate::commands::context::AppState;
use crate::commands::execution::execute_read_profile_operation;
use crate::commands::tests::support::{account_ref, phase_four_state, test_profile_value};
use crate::commands::vault::store_id;
use foks_agent_proto::{Operation, ProfileOverview, ResponseResult};
use foks_desktop::{CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::sync::{Arc, Mutex};

pub(super) struct PhaseSixReadTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for PhaseSixReadTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles => {
                Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
            }
            Operation::ListDevices { .. } => Ok(serde_json::json!([
                {"id_hex":"04".repeat(33),"role":"owner","current":true},
                {"id_hex":format!("04{}", "06".repeat(32)),"role":"owner","current":false}
            ])),
            Operation::ListBackupEnrollments { .. } => Ok(serde_json::json!([{
                "backup_alias":"paper",
                "account_alias":"personal",
                "backup_id_hex":"10".repeat(33)
            }])),
            other => panic!("unexpected account read operation {other:?}"),
        }
    }
}

#[test]
fn phase_six_account_reads_resolve_profile_then_issue_one_bound_call() {
    let account = account_ref("work", "personal");
    let transport = PhaseSixReadTransport {
        calls: Mutex::new(Vec::new()),
    };
    assert_eq!(load_account_devices(&transport, &account).unwrap().len(), 2);
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::ListDevices {
                profile: "work".to_owned(),
                alias: "personal".to_owned()
            }
        ]
    );

    let transport = PhaseSixReadTransport {
        calls: Mutex::new(Vec::new()),
    };
    assert_eq!(
        load_backup_enrollments(&transport, &account).unwrap().len(),
        1
    );
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::ListBackupEnrollments {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned()
            }
        ]
    );
}

pub(super) struct ReadOnlyPrepareLoss {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for ReadOnlyPrepareLoss {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles => {
                Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
            }
            Operation::PrepareOwnerBackup { .. } => Err(foks_desktop::AgentError::Transport(
                "agent disappeared".to_owned(),
            )),
            other => panic!("unexpected preparation operation {other:?}"),
        }
    }
}

#[test]
fn backup_prepare_failure_is_read_only_and_never_mutation_ambiguous() {
    let transport = ReadOnlyPrepareLoss {
        calls: Mutex::new(Vec::new()),
    };
    let error = execute_read_profile_operation(
        &transport,
        "work",
        Operation::PrepareOwnerBackup {
            profile: "work".to_owned(),
            account_alias: "personal".to_owned(),
            backup_alias: "paper".to_owned(),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "agent-lost");
    assert!(!error.ambiguous);
    assert_eq!(transport.calls.lock().unwrap().len(), 2);
}

pub(super) struct AccountsTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
    pub(super) response: serde_json::Value,
}

impl foks_desktop::AgentTransport for AccountsTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation);
        Ok(self.response.clone())
    }
}

/// Verify that accounts sharing an alias across different profiles produce
/// distinct store IDs and resolve deterministically regardless of catalog order.
#[test]
fn two_profiles_sharing_an_account_alias_get_distinct_store_refs() {
    let home = account_ref("home.example", "personal");
    let work = account_ref("work.example", "personal");
    let home_id = store_id(&CatalogStoreRef::Account(home.clone()));
    let work_id = store_id(&CatalogStoreRef::Account(work.clone()));
    assert_ne!(home_id, work_id);
    // Store IDs are deterministic for a given account across catalog refreshes.
    assert_eq!(
        home_id,
        store_id(&CatalogStoreRef::Account(account_ref(
            "home.example",
            "personal"
        )))
    );

    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let catalog = |stores: Vec<CatalogStoreSummary>| CatalogSnapshot {
        profiles: vec!["home.example".to_owned(), "work.example".to_owned()],
        stores,
        ..CatalogSnapshot::default()
    };
    *state.catalog.lock().unwrap() = Some(catalog(vec![
        CatalogStoreSummary::Account {
            store: home.clone(),
        },
        CatalogStoreSummary::Account {
            store: work.clone(),
        },
    ]));

    let resolved = state.selected_account(&home_id).unwrap();
    assert_eq!(resolved.profile, "home.example");
    assert_eq!(resolved.account_alias, "personal");
    let resolved = state.selected_account(&work_id).unwrap();
    assert_eq!(resolved.profile, "work.example");
    assert_eq!(resolved.account_alias, "personal");

    // An unqualified account alias cannot resolve a store.
    assert_eq!(
        state.selected_account("personal").unwrap_err().code,
        "store-not-found"
    );

    // Store resolution remains deterministic regardless of catalog order.
    *state.catalog.lock().unwrap() = Some(catalog(vec![
        CatalogStoreSummary::Account {
            store: work.clone(),
        },
        CatalogStoreSummary::Account {
            store: home.clone(),
        },
    ]));
    assert_eq!(
        state.selected_account(&home_id).unwrap().profile,
        "home.example"
    );
    assert_eq!(
        state.selected_account(&work_id).unwrap().profile,
        "work.example"
    );

    // An account removed from the catalog reports gone rather than
    // resolving to an account that shares its alias.
    *state.catalog.lock().unwrap() = Some(catalog(vec![CatalogStoreSummary::Account {
        store: home.clone(),
    }]));
    assert_eq!(
        state.selected_account(&work_id).unwrap_err().code,
        "store-not-found"
    );
}

#[test]
fn account_projection_is_bound_to_catalog_identities() {
    let catalog = CatalogSnapshot {
        profiles: vec!["work.example".to_owned()],
        stores: vec![
            CatalogStoreSummary::Account {
                store: account_ref("work.example", "personal"),
            },
            CatalogStoreSummary::Account {
                store: account_ref("work.example", "automation"),
            },
        ],
        ..CatalogSnapshot::default()
    };
    let transport = AccountsTransport {
        calls: Mutex::new(Vec::new()),
        response: serde_json::json!([
            {"profile":"work.example","alias":"personal","username":"vitalik"},
            {"profile":"work.example","alias":"automation","username":"deploy-bot"}
        ]),
    };
    let accounts = load_accounts(&transport, &catalog).unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].alias, "automation");
    assert_eq!(accounts[1].username, "vitalik");
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[Operation::ListAccounts {
            profile: "work.example".to_owned()
        }]
    );

    let unknown = AccountsTransport {
        calls: Mutex::new(Vec::new()),
        response: serde_json::json!([
            {"profile":"work.example","alias":"personal","username":"vitalik"},
            {"profile":"work.example","alias":"outside","username":"mallory"}
        ]),
    };
    assert_eq!(
        load_accounts(&unknown, &catalog).unwrap_err().code,
        "invalid-response"
    );

    for malformed in [
        serde_json::json!([
            {"profile":"work.example","alias":"personal","username":"vitalik","extra":true},
            {"profile":"work.example","alias":"automation","username":"deploy-bot"}
        ]),
        serde_json::json!([
            {"profile":"work.example","alias":"personal","username":"satoshi\nvitalik"},
            {"profile":"work.example","alias":"automation","username":"deploy-bot"}
        ]),
    ] {
        let transport = AccountsTransport {
            calls: Mutex::new(Vec::new()),
            response: malformed,
        };
        assert_eq!(
            load_accounts(&transport, &catalog).unwrap_err().code,
            "invalid-response"
        );
    }
}

pub(super) struct MixedAccountsTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for MixedAccountsTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListAccounts { profile } if profile == "available" => Ok(
                serde_json::json!([{"profile":"available","alias":"personal","username":"satoshi"}]),
            ),
            Operation::ListAccounts { profile } => {
                panic!("blocked profile {profile} must not be read")
            }
            other => panic!("unexpected account operation {other:?}"),
        }
    }
}

#[test]
fn account_projection_reuses_the_catalog_profile_overview() {
    let catalog = CatalogSnapshot {
        profiles: vec!["work.example".to_owned()],
        stores: vec![CatalogStoreSummary::Account {
            store: account_ref("work.example", "personal"),
        }],
        profile_overviews: vec![ProfileOverview {
            profile: "work.example".to_owned(),
            accounts: ResponseResult::Success {
                value: serde_json::json!([{
                    "profile":"work.example",
                    "alias":"personal",
                    "username":"vitalik"
                }]),
            },
            teams: ResponseResult::Success {
                value: serde_json::json!([]),
            },
            server_status: ResponseResult::Success {
                value: serde_json::json!({
                    "profile":"work.example",
                    "configured_probe":"work.example",
                    "host":null,
                    "chat_supported":null,
                    "compatibility":{"status":"not-required"}
                }),
            },
        }],
        ..CatalogSnapshot::default()
    };
    let transport = AccountsTransport {
        calls: Mutex::new(Vec::new()),
        response: serde_json::Value::Null,
    };

    let accounts = load_accounts(&transport, &catalog).unwrap();
    assert_eq!(accounts[0].username, "vitalik");
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[test]
fn account_projection_skips_blocked_profiles_and_keeps_available_accounts() {
    let catalog = CatalogSnapshot {
        profiles: vec!["available".to_owned(), "blocked".to_owned()],
        stores: vec![
            CatalogStoreSummary::Account {
                store: account_ref("available", "personal"),
            },
            CatalogStoreSummary::Account {
                store: account_ref("blocked", "work"),
            },
        ],
        blocked_profiles: vec!["blocked".to_owned()],
        ..CatalogSnapshot::default()
    };
    let transport = MixedAccountsTransport {
        calls: Mutex::new(Vec::new()),
    };

    let accounts = load_accounts(&transport, &catalog).unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].profile, "available");
    assert_eq!(accounts[0].alias, "personal");
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[Operation::ListAccounts {
            profile: "available".to_owned(),
        }]
    );
}

#[test]
fn device_removal_preflight_refuses_current_unknown_and_duplicate_ids() {
    let state = phase_four_state(Vec::new());
    let account_id = store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    let current = DeviceDto {
        id: "04".repeat(33),
        name: Some("This Mac".to_owned()),
        role: "owner",
        current: true,
    };
    let other = DeviceDto {
        id: format!("04{}", "06".repeat(32)),
        name: Some("Spare".to_owned()),
        role: "owner",
        current: false,
    };
    let yubi = DeviceDto {
        id: "08".repeat(34),
        name: None,
        role: "owner",
        current: false,
    };
    assert_eq!(
        state.selected_account_dto(&account_id).unwrap_err().code,
        "accounts-required"
    );
    state.accounts.lock().unwrap().insert(
        account_id.clone(),
        AccountDto {
            local_alias: None,
            store: account_id.clone(),
            profile: "work.example".to_owned(),
            alias: "personal".to_owned(),
            username: "satoshi".to_owned(),
        },
    );
    assert_eq!(
        state.selected_account_dto(&account_id).unwrap().username,
        "satoshi"
    );
    state.devices.lock().unwrap().insert(
        account_id.clone(),
        vec![current.clone(), other.clone(), yubi.clone()],
    );
    assert_eq!(
        state
            .selected_device_target(&account_id, &current.id)
            .unwrap_err()
            .code,
        "current-device"
    );
    assert_eq!(
        state
            .selected_device_target(&account_id, &format!("04{}", "07".repeat(32)))
            .unwrap_err()
            .code,
        "device-not-found"
    );
    assert_eq!(
        state
            .selected_device_target(&account_id, &other.id)
            .unwrap(),
        account_ref("work.example", "personal")
    );
    assert_eq!(
        state
            .selected_device_target(&account_id, &yubi.id)
            .unwrap_err()
            .code,
        "device-not-removable"
    );
    state
        .devices
        .lock()
        .unwrap()
        .insert(account_id.clone(), vec![other.clone(), other]);
    assert_eq!(
        state
            .selected_device_target(&account_id, &format!("04{}", "06".repeat(32)))
            .unwrap_err()
            .code,
        "invalid-response"
    );
    state.devices.lock().unwrap().clear();
    assert_eq!(
        state
            .selected_device_target(&account_id, &format!("04{}", "06".repeat(32)))
            .unwrap_err()
            .code,
        "devices-required"
    );
}
