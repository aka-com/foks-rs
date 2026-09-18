use crate::agent::{
    AgentError, AgentProcessInfo, MaintenanceDisposition, MaintenanceKind,
    MaintenanceOperationOutcome, MaintenancePhase, MaintenanceSnapshot,
};
use crate::applock::LockStateDto;
use crate::commands::accounts::{
    backup_enrollment_dtos, device_dtos, device_removal_response, AccountDto, BackupEnrollmentDto,
    BackupPhraseDto, DeviceDto, DeviceRemovalDto, PassphraseReportDto,
};
use crate::commands::application::{AgentStatusDto, AppInfo};
use crate::commands::enrollment::{
    device_provision_response, pairing_offer_response, DeviceProvisionDto, GoProfileCandidateDto,
    GoProfileDiscoveryDto, PairingOfferDto, PendingOperationDto,
};
use crate::commands::execution::{map_mutation_error, MutationKind};
use crate::commands::groups::{
    DiscoveredGroupDto, FederationEntryDto, GroupDiscoveryDto, PartyDto,
};
use crate::commands::servers::{
    added_server_response, checked_server_response, forgotten_server_response,
    reset_preview_response, server_status_response, validate_compatibility, AddedServerDto,
    CheckedProfileDto, CheckedServerDto, CheckedServerVersionDto, ForgottenServerDto,
    ResetArtifactDto, ResetPreviewDto, ServerDto, ServerLabelDto, ServerStatusSnapshotDto,
    StoredHostDto,
};
use crate::commands::tests::support::test_profile_value;
use crate::commands::types::{CommandAck, MutationDto, RoleDto};
use crate::commands::validation::exact_profile_confirmation;
use crate::commands::vault::{
    CatalogDto, CatalogInventoryDto, DownloadResult, ItemDto, ReadItemDto, StoreDto,
};
use crate::commands::yubikey::{
    YubiAccountDto, YubiCardDto, YubiChangedDto, YubiEnrollmentDto, YubiFederationRefreshDto,
    YubiLifecycleDto, YubiPinStatusDto, YubiRevocationDto, YubiSubkeyRecoveryDto, YubiSyncDto,
};
use foks_agent_proto::{KvRole, Operation};
use zeroize::Zeroizing;

#[test]
fn device_list_accepts_new_account_with_backup_enrollment() {
    let device_id = format!("04{}", "01".repeat(32));
    let backup_id = format!("10{}", "02".repeat(32));
    let devices = device_dtos(serde_json::json!([
        {"id_hex": device_id, "name": "This Mac", "role": "owner", "current": true},
        {"id_hex": backup_id, "name": "cage 32", "role": "owner", "current": false}
    ]))
    .unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, device_id);
    assert!(devices[0].current);
    let backups = backup_enrollment_dtos(
        serde_json::json!([
            {"backup_alias": "paper", "account_alias": "personal", "backup_id_hex": backup_id}
        ]),
        "personal",
    )
    .unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].backup_id, backup_id);
}

#[test]
fn device_list_separates_device_keys_from_other_account_members() {
    let device_id = format!("04{}", "01".repeat(32));
    let card_id = format!("08{}", "02".repeat(33));
    let devices = device_dtos(serde_json::json!([
        {"id_hex": device_id, "name": "This Mac", "role": "owner", "current": true},
        {"id_hex": "10".repeat(33), "name": null, "role": "owner", "current": false},
        {"id_hex": "13".repeat(33), "name": "Automation", "role": "admin", "current": false},
        {"id_hex": card_id, "name": "Security key", "role": "owner", "current": false}
    ]))
    .unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].id, device_id);
    assert_eq!(devices[1].id, card_id);
}

#[test]
fn device_list_validates_non_device_members_before_excluding_them() {
    let device = serde_json::json!({
        "id_hex": "04".repeat(33), "name": "This Mac", "role": "owner", "current": true
    });
    let backup = serde_json::json!({
        "id_hex": "10".repeat(33), "name": "cage 32", "role": "owner", "current": false
    });
    for malformed in [
        serde_json::json!([device, backup, backup]),
        serde_json::json!([device, {"id_hex": "10", "role": "owner", "current": false}]),
        serde_json::json!([device, {"id_hex": "13".repeat(32), "role": "admin", "current": false}]),
        serde_json::json!([device, {"id_hex": "01".repeat(33), "role": "owner", "current": false}]),
        serde_json::json!([device, {"id_hex": "10".repeat(33), "role": "robot", "current": false}]),
        serde_json::json!([device, {"id_hex": "10".repeat(33), "name": "bad\nname", "role": "owner", "current": false}]),
        serde_json::json!([device, {"id_hex": "10".repeat(33), "role": "owner", "current": true}]),
        serde_json::json!([{"id_hex": "10".repeat(33), "role": "owner", "current": true}]),
        serde_json::json!([backup]),
    ] {
        assert_eq!(device_dtos(malformed).unwrap_err().code, "invalid-response");
    }
}

#[test]
fn device_list_distinguishes_invalid_ids_duplicates_and_names() {
    let device = serde_json::json!({
        "id_hex": "04".repeat(33), "name": "This Mac", "role": "owner", "current": true
    });
    for (rows, expected) in [
        (
            serde_json::json!([{ "id_hex": "04", "role": "owner", "current": true }]),
            "The agent returned an unsupported or malformed account key identifier.",
        ),
        (
            serde_json::json!([device, device]),
            "The agent returned a duplicate account key identifier.",
        ),
        (
            serde_json::json!([{
                "id_hex": "04".repeat(33), "name": "bad\nname", "role": "owner", "current": true
            }]),
            "The agent returned an invalid device name.",
        ),
    ] {
        assert_eq!(device_dtos(rows).unwrap_err().message, expected);
    }
}

fn assert_lifecycle_cases<T: serde::Serialize, const N: usize>(
    fixture: &serde_json::Value,
    key: &str,
    cases: [(&str, T); N],
) {
    let values: serde_json::Map<String, serde_json::Value> = cases
        .into_iter()
        .map(|(name, value)| (name.to_owned(), serde_json::to_value(value).unwrap()))
        .collect();
    assert_eq!(values.len(), N);
    assert_eq!(
        serde_json::Value::Object(values),
        fixture[key]["valid"],
        "{key}"
    );
}

#[test]
fn shared_agent_lifecycle_contract_matches_native_dtos() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../wire-contract.json")).unwrap();
    assert_lifecycle_cases(
        &fixture,
        "agentStatusCases",
        [
            (
                "ready",
                AgentStatusDto::from(foks_agent_proto::AgentStatus::Ready),
            ),
            (
                "bootstrap",
                AgentStatusDto::from(foks_agent_proto::AgentStatus::Bootstrap {
                    step: "unlock".to_owned(),
                }),
            ),
        ],
    );
    assert_lifecycle_cases(
        &fixture,
        "agentProcessInfoCases",
        [
            ("absent", AgentProcessInfo::default()),
            (
                "owned",
                AgentProcessInfo {
                    pid: Some(42),
                    executable: Some("/opt/foks-agent".to_owned()),
                    started_at: Some(1_700_000_000),
                    owned: true,
                },
            ),
            (
                "external",
                AgentProcessInfo {
                    pid: Some(43),
                    executable: Some("/usr/local/bin/foks-agent".to_owned()),
                    started_at: Some(1_700_000_001),
                    owned: false,
                },
            ),
            (
                "uninspectable",
                AgentProcessInfo {
                    pid: Some(44),
                    executable: None,
                    started_at: None,
                    owned: true,
                },
            ),
            (
                "pathOnly",
                AgentProcessInfo {
                    pid: Some(45),
                    executable: Some("/opt/foks-agent".to_owned()),
                    started_at: None,
                    owned: false,
                },
            ),
            (
                "timeOnly",
                AgentProcessInfo {
                    pid: Some(46),
                    executable: None,
                    started_at: Some(0),
                    owned: false,
                },
            ),
        ],
    );
}

#[test]
fn shared_app_lock_contract_matches_native_dtos() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../wire-contract.json")).unwrap();
    let available = |locked, mechanism| LockStateDto {
        locked,
        available: true,
        mechanism,
        unavailable_reason: None,
    };
    assert_lifecycle_cases(
        &fixture,
        "appLockStateCases",
        [
            ("biometryLocked", available(true, "biometry")),
            ("biometryUnlocked", available(false, "biometry")),
            ("passwordLocked", available(true, "password")),
            ("passwordUnlocked", available(false, "password")),
            (
                "unavailable",
                LockStateDto {
                    locked: false,
                    available: false,
                    mechanism: "none",
                    unavailable_reason: Some("OS authentication unavailable".to_owned()),
                },
            ),
        ],
    );
}

#[test]
fn shared_maintenance_contract_preserves_operation_and_restoration_outcomes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../wire-contract.json")).unwrap();
    let active = |revision, kind, phase| MaintenanceSnapshot::Active {
        generation: 1,
        revision,
        kind,
        phase,
    };
    let complete = |kind, operation, disposition| MaintenanceSnapshot::Complete {
        generation: 1,
        revision: 6,
        kind,
        operation,
        disposition,
    };
    let failed = || MaintenanceOperationOutcome::Failed {
        error: AgentError::new("io", "Destination full", false),
    };
    let restart = || MaintenanceDisposition::RestartSelectedRoot {
        root: "/private/foks/selected".to_owned(),
    };
    let recovery = || MaintenanceDisposition::RecoveryRequired {
        root: "/private/foks/selected".to_owned(),
    };
    let restoration_failed = || MaintenanceDisposition::RestorationFailed {
        root: "/private/foks/current".to_owned(),
        error: AgentError {
            ambiguous: true,
            fatal: true,
            ..AgentError::new("agent-lost", "Restore failed", true)
        },
    };
    assert_lifecycle_cases(
        &fixture,
        "maintenanceCases",
        [
            (
                "idle",
                MaintenanceSnapshot::Idle {
                    generation: 0,
                    revision: 0,
                },
            ),
            (
                "selecting",
                active(1, MaintenanceKind::Export, MaintenancePhase::Selecting),
            ),
            (
                "confirming",
                active(2, MaintenanceKind::Import, MaintenancePhase::Confirming),
            ),
            (
                "quiescing",
                active(3, MaintenanceKind::Relocate, MaintenancePhase::Quiescing),
            ),
            (
                "running",
                active(4, MaintenanceKind::Verify, MaintenancePhase::Running),
            ),
            (
                "restoring",
                active(5, MaintenanceKind::Restart, MaintenancePhase::Restoring),
            ),
            (
                "cancelled",
                complete(
                    MaintenanceKind::Export,
                    MaintenanceOperationOutcome::Cancelled,
                    MaintenanceDisposition::ContinueCurrentRoot,
                ),
            ),
            (
                "completed",
                complete(
                    MaintenanceKind::Verify,
                    MaintenanceOperationOutcome::Completed,
                    MaintenanceDisposition::ContinueCurrentRoot,
                ),
            ),
            (
                "failed",
                complete(
                    MaintenanceKind::Import,
                    failed(),
                    MaintenanceDisposition::ContinueCurrentRoot,
                ),
            ),
            (
                "restart",
                complete(
                    MaintenanceKind::Relocate,
                    MaintenanceOperationOutcome::Completed,
                    restart(),
                ),
            ),
            (
                "recovery",
                complete(
                    MaintenanceKind::Import,
                    MaintenanceOperationOutcome::Completed,
                    recovery(),
                ),
            ),
            (
                "failedRestart",
                complete(MaintenanceKind::Relocate, failed(), restart()),
            ),
            (
                "failedRecovery",
                complete(MaintenanceKind::Import, failed(), recovery()),
            ),
            (
                "restorationFailed",
                complete(
                    MaintenanceKind::Restart,
                    MaintenanceOperationOutcome::Completed,
                    restoration_failed(),
                ),
            ),
            (
                "cancelledRestorationFailed",
                complete(
                    MaintenanceKind::Export,
                    MaintenanceOperationOutcome::Cancelled,
                    restoration_failed(),
                ),
            ),
            (
                "failedRestorationFailed",
                complete(MaintenanceKind::Import, failed(), restoration_failed()),
            ),
        ],
    );
}

#[test]
fn server_status_requires_consistent_explicit_policy_facts() {
    let status = serde_json::json!({
        "profile":"work", "configured_probe":"foks.example", "host":null,
        "chat_supported":null,
        "compatibility":{"status":"incompatible","reason":"drift","expires_at":200}
    });
    assert!(server_status_response(status.clone(), "work", "foks.example", true).is_ok());
    for (key, value) in [
        (
            "compatibility",
            serde_json::json!({"status":"not-required"}),
        ),
        (
            "compatibility",
            serde_json::json!({"status":"validated","expires_at":200,"capabilities":["invented"]}),
        ),
        (
            "compatibility",
            serde_json::json!({"status":"validated","expires_at":200,"capabilities":[]}),
        ),
        ("compatibility", serde_json::Value::Null),
        ("chat_supported", serde_json::json!(true)),
    ] {
        let mut invalid = status.clone();
        invalid[key] = value;
        assert_eq!(
            server_status_response(invalid, "work", "foks.example", true)
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }
}

#[test]
fn shared_compatibility_contract_validates_grants_and_preserves_service_support() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../wire-contract.json")).unwrap();
    let host = &fixture["serverStatus"]["host"];
    let status = serde_json::json!({
        "profile": "work",
        "configured_probe": "foks.example",
        "host": {
            "lookup_name": host["lookupName"],
            "canonical_name": host["canonicalName"],
            "host_id_hex": host["hostId"],
            "host_chain_sequence": host["chain"],
            "merkle_epoch": host["epoch"]
        },
        "compatibility": fixture["serverStatus"]["compatibility"],
        "chat_supported": true
    });
    for case in fixture["compatibilityCases"].as_array().unwrap() {
        let mut value = status.clone();
        value["compatibility"] = case["wire"].clone();
        let expected_required = case["wire"]["status"] != "not-required";
        let result = server_status_response(value, "work", "foks.example", expected_required);
        if case.get("decoded").is_some() {
            let result = result.unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
            assert_eq!(result.chat_supported, Some(true), "{}", case["name"]);
            assert!(result.host.is_some(), "{}", case["name"]);
            let expected: foks_agent_proto::CompatibilityStatus =
                serde_json::from_value(case["wire"].clone()).unwrap();
            assert_eq!(result.compatibility, expected, "{}", case["name"]);
        } else {
            assert_eq!(
                result.unwrap_err().code,
                "invalid-response",
                "{}",
                case["name"]
            );
        }
    }
    for capability in fixture["protocolCapabilities"].as_array().unwrap() {
        let compatibility = serde_json::json!({
            "status": "validated", "expires_at": 200, "capabilities": [capability]
        });
        assert!(
            validate_compatibility(compatibility).is_ok(),
            "{capability}"
        );
    }
    for field in fixture["serverStatusRequiredFields"].as_array().unwrap() {
        let key = match field.as_str().unwrap() {
            "configuredProbe" => "configured_probe",
            "chatSupported" => "chat_supported",
            key => key,
        };
        for no_host in [false, true] {
            let mut value = status.clone();
            if no_host {
                value["host"] = serde_json::Value::Null;
                value["chat_supported"] = serde_json::Value::Null;
            }
            value.as_object_mut().unwrap().remove(key);
            assert_eq!(
                server_status_response(value, "work", "foks.example", true)
                    .unwrap_err()
                    .code,
                "invalid-response",
                "{key}"
            );
        }
    }
}

#[test]
fn wire_contract_fixture_matches_serialized_shapes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../wire-contract.json")).unwrap();
    let connectivity = crate::commands::servers::reconcile_server_response(serde_json::json!({
        "profile": "work",
        "identity": { "status": "success", "value": { "status": "connected", "host_id": fixture["profileReconciliation"]["identity"]["hostId"], "configured_probe": "foks.example:4430" } },
        "compatibility": { "status": "success", "value": { "status": "renewed" } }
    }), "work").unwrap();
    assert_eq!(
        serde_json::to_value(connectivity).unwrap(),
        fixture["profileReconciliation"]
    );
    let configured_server = ServerDto {
        id: "work".to_owned(),
        name: "work".to_owned(),
        label: Some("Work".to_owned()),
        configured_probe: "foks.example".to_owned(),
        accounts: vec!["personal".to_owned()],
    };
    assert_eq!(
        serde_json::to_value(configured_server).unwrap(),
        fixture["configuredServer"]
    );
    let app_info = AppInfo {
        version: "0.3.0".to_owned(),
        agent_socket: "/private/foks/agent.sock".to_owned(),
        managed_profile: Some("local".to_owned()),
        computer_name: None,
    };
    assert_eq!(serde_json::to_value(app_info).unwrap(), fixture["appInfo"]);
    let discovery = GoProfileDiscoveryDto {
        installed: true,
        candidates: vec![GoProfileCandidateDto {
            candidate_id: "aa".repeat(32),
            username: Some("satoshi".to_owned()),
            server_hint: Some("foks.app".to_owned()),
            host_id: format!("02{}", "bb".repeat(32)),
            user_id: format!("01{}", "cc".repeat(32)),
            device_id: format!("04{}", "dd".repeat(32)),
            role: "owner".to_owned(),
            storage_kind: "macos-keychain".to_owned(),
            hidden: false,
            provisional: false,
            pairable: true,
            copyable: true,
        }],
    };
    assert_eq!(
        serde_json::to_value(discovery).unwrap(),
        fixture["goProfileDiscovery"]
    );
    let error = AgentError::from_agent(
        foks_agent_proto::ErrorCode::VersionMismatch,
        "changed".to_owned(),
    );
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        fixture["commandError"]
    );
    let role: RoleDto = KvRole::Member { visibility: 0 }.into();
    assert_eq!(serde_json::to_value(role).unwrap(), fixture["memberRole"]);
    assert_eq!(
        serde_json::json!({"readRole": "Member:0", "writeRole": "Admin"}),
        fixture["teamItemCreateAccess"]
    );
    let account = AccountDto {
        local_alias: None,
        store: "opaque-account-ref".to_owned(),
        profile: "foks.example".to_owned(),
        alias: "personal".to_owned(),
        username: "vitalik".to_owned(),
    };
    assert_eq!(serde_json::to_value(account).unwrap(), fixture["account"]);
    let party = PartyDto {
        store: "opaque-team-ref".to_owned(),
        username: Some("dana.okafor".to_owned()),
        party_kind: "user".to_owned(),
        generation: 5,
        locally_manageable: true,
        party_id_hex: "01".repeat(33),
        scoped_host_id_hex: None,
        source_role: KvRole::Member { visibility: 0 }.into(),
        destination_role: KvRole::Member { visibility: 0 }.into(),
    };
    assert_eq!(serde_json::to_value(party).unwrap(), fixture["party"]);
    let federation = FederationEntryDto {
        store: "opaque-team-ref".to_owned(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "homelab".to_owned(),
        remote_host_id_hex: "02".repeat(33),
        remote_team_id_hex: "03".repeat(33),
        destination: KvRole::Member { visibility: -2 }.into(),
        operation_id_hex: Some("07".repeat(16)),
        active: false,
    };
    assert_eq!(
        serde_json::to_value(federation).unwrap(),
        fixture["federationEntry"]
    );
    assert_eq!(
        serde_json::to_value(Operation::ExpelFederatedTeam {
            profile: "foks.example".to_owned(),
            team_alias: "engineering".to_owned(),
            remote_host_id_hex: "02".repeat(33),
            remote_team_id_hex: "03".repeat(33),
        })
        .unwrap(),
        fixture["federationExpulsionOperation"]
    );
    assert_eq!(
        serde_json::to_value(CommandAck { ok: true }).unwrap(),
        fixture["commandAck"]
    );
    assert_eq!(
        serde_json::to_value(DownloadResult { saved: false }).unwrap(),
        fixture["cancelledDownload"]
    );
    assert_eq!(
        serde_json::to_value(MutationDto { applied: true }).unwrap(),
        fixture["appliedMutation"]
    );
    assert_eq!(
        serde_json::to_value(MutationDto { applied: false }).unwrap(),
        fixture["cancelledMutation"]
    );
    let exists = map_mutation_error(
        foks_desktop::AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::Conflict,
            message: "exists".to_owned(),
            fields: Box::default(),
        },
        MutationKind::Create,
    );
    assert_eq!(
        serde_json::to_value(exists).unwrap(),
        fixture["alreadyExistsError"]
    );
    let capability = map_mutation_error(
        foks_desktop::AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::CapabilityDenied,
            message: "denied".to_owned(),
            fields: foks_agent_proto::ErrorFields {
                capability: Some("kv".to_owned()),
                ..Default::default()
            }
            .into(),
        },
        MutationKind::Guarded,
    );
    assert_eq!(
        serde_json::to_value(capability).unwrap(),
        fixture["capabilityUnavailableError"]
    );
    let agent_lost = map_mutation_error(
        foks_desktop::AgentError::Transport("gone".to_owned()),
        MutationKind::Guarded,
    );
    assert_eq!(
        serde_json::to_value(agent_lost).unwrap(),
        fixture["agentLostError"]
    );
    let store = StoreDto {
        id: "opaque-store-ref".to_owned(),
        kind: "account",
        name: "Personal".to_owned(),
        server: "foks.example".to_owned(),
        account: "personal".to_owned(),
        alias: None,
        active: None,
        creation_phase: None,
        team_kind: None,
        team_id_hex: None,
    };
    let catalog = CatalogDto {
        profiles: vec!["foks.example".to_owned()],
        stores: vec![store.clone()],
        known_stores: vec![store],
        inventory: vec![CatalogInventoryDto {
            profile: "foks.example".to_owned(),
            accounts_complete: true,
            teams_complete: true,
        }],
        store_reads: vec![],
        full_item_reads: None,
        items: vec![ItemDto {
            store: "opaque-store-ref".to_owned(),
            path: "/wifi/password".to_owned(),
            kind: "Secret",
            size: Some(42),
            version: 7,
            read: KvRole::Member { visibility: -16384 }.into(),
            write: KvRole::Admin.into(),
        }],
        failures: vec![],
        blocked_profiles: vec![],
        local_metadata: None,
        generation: 7,
    };
    assert_eq!(serde_json::to_value(catalog).unwrap(), fixture["catalog"]);
    let read = ReadItemDto {
        store: "opaque-store-ref".to_owned(),
        path: "/wifi/password".to_owned(),
        version: 7,
        value: Zeroizing::new("secret".to_owned()),
    };
    assert_eq!(serde_json::to_value(read).unwrap(), fixture["readItem"]);
    let checked = CheckedProfileDto {
        profile: "work".to_owned(),
        acceptance: "inserted".to_owned(),
        lookup_name: "foks.example".to_owned(),
        canonical_name: "foks.example".to_owned(),
        host_id: "02".repeat(33),
        chain: 9,
        epoch: 12,
    };
    assert_eq!(
        serde_json::to_value(checked).unwrap(),
        fixture["checkedProfile"]
    );
    let pending = PendingOperationDto {
        kind: "account-recovery",
        alias: "recovered".to_owned(),
        target: Some("owner".to_owned()),
    };
    assert_eq!(
        serde_json::to_value(pending).unwrap(),
        fixture["pendingOperation"]
    );
    let backup = BackupPhraseDto {
        backup_alias: "paper".to_owned(),
        phrase: Zeroizing::new(
            "abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon"
                .to_owned(),
        ),
    };
    assert_eq!(
        serde_json::to_value(backup).unwrap(),
        fixture["backupPhrase"]
    );
    let discovery = GroupDiscoveryDto {
        account_alias: "personal".to_owned(),
        groups: vec![DiscoveredGroupDto {
            alias: "engineering".to_owned(),
            account_alias: "personal".to_owned(),
            team_id_hex: "03".repeat(33),
            kind: "named",
            name: Some("Engineering".to_owned()),
            active: true,
        }],
    };
    assert_eq!(
        serde_json::to_value(discovery).unwrap(),
        fixture["groupDiscovery"]
    );
    let status = ServerStatusSnapshotDto {
        profile: "work".to_owned(),
        configured_probe: "foks.example".to_owned(),
        host: Some(StoredHostDto {
            lookup_name: "foks.example".to_owned(),
            canonical_name: "foks.example".to_owned(),
            host_id: "02".repeat(33),
            chain: 11,
            epoch: 42,
        }),
        chat_supported: Some(true),
        compatibility: foks_agent_proto::CompatibilityStatus::Validated {
            expires_at: 1_900_000_000,
            capabilities: ["chat".to_owned(), "kv".to_owned()].into_iter().collect(),
        },
    };
    assert_eq!(
        serde_json::to_value(status).unwrap(),
        fixture["serverStatus"]
    );
    let added = AddedServerDto {
        profile: "partner".to_owned(),
        configured_probe: "foks.partner.example".to_owned(),
    };
    assert_eq!(serde_json::to_value(added).unwrap(), fixture["addedServer"]);
    let labeled = ServerLabelDto {
        profile: "partner".to_owned(),
        label: Some("Partners".to_owned()),
        changed: true,
    };
    assert_eq!(
        serde_json::to_value(labeled).unwrap(),
        fixture["serverLabel"]
    );
    let forgotten = ForgottenServerDto {
        profile: "partner".to_owned(),
        removed: true,
    };
    assert_eq!(
        serde_json::to_value(forgotten).unwrap(),
        fixture["forgottenServer"]
    );
    let checked = CheckedServerDto {
        profile: "work".to_owned(),
        acceptance: "advanced".to_owned(),
        lookup_name: "foks.example".to_owned(),
        canonical_name: "foks.example".to_owned(),
        host_id: "02".repeat(33),
        chain: 12,
        epoch: 43,
        server_version: Some(CheckedServerVersionDto {
            minimum: None,
            newest: None,
            message: String::new(),
            compatible: true,
        }),
    };
    assert_eq!(
        serde_json::to_value(checked).unwrap(),
        fixture["checkedServer"]
    );
    let device = DeviceDto {
        id: "04".repeat(33),
        name: Some("This Mac".to_owned()),
        role: "owner",
        current: true,
    };
    assert_eq!(serde_json::to_value(device).unwrap(), fixture["device"]);
    let removal = DeviceRemovalDto {
        device_id: "04".repeat(33),
        user_chain_sequence: 15,
        already_absent: false,
    };
    assert_eq!(
        serde_json::to_value(removal).unwrap(),
        fixture["deviceRemoval"]
    );
    let enrollment = BackupEnrollmentDto {
        backup_alias: "paper".to_owned(),
        account_alias: "personal".to_owned(),
        backup_id: "10".repeat(33),
    };
    assert_eq!(
        serde_json::to_value(enrollment).unwrap(),
        fixture["backupEnrollment"]
    );
    assert_eq!(
        serde_json::to_value(YubiCardDto { serial: 424_242 }).unwrap(),
        fixture["yubiCard"]
    );
    assert_eq!(
        serde_json::to_value(YubiEnrollmentDto {
            alias: "work_key".to_owned(),
            state: "complete",
            device_id: None,
            card_serial: None,
        })
        .unwrap(),
        fixture["yubiEnrollment"]
    );
    assert_eq!(
        serde_json::to_value(YubiAccountDto {
            alias: "work_key".to_owned(),
            username: "satoshi".to_owned(),
            yubi_id: "08".repeat(34),
            subkey_id: "0d".repeat(33),
            user_chain_sequence: 21,
            management_enrolled: true,
        })
        .unwrap(),
        fixture["yubiAccount"]
    );
    assert_eq!(
        serde_json::to_value(YubiSyncDto {
            username: "satoshi".to_owned(),
            user_chain_sequence: 22,
            directories: 2,
            entries: 7,
            federation: vec![YubiFederationRefreshDto {
                local_profile: "work".to_owned(),
                local_team_alias: "engineering".to_owned(),
                refreshed: true,
                deferred: None,
            }],
        })
        .unwrap(),
        fixture["yubiSync"]
    );
    assert_eq!(
        serde_json::to_value(YubiPinStatusDto {
            remaining: 3,
            blocked: false,
        })
        .unwrap(),
        fixture["yubiPinStatus"]
    );
    assert_eq!(
        serde_json::to_value(YubiLifecycleDto {
            alias: "work_key".to_owned(),
            management_enrolled: true,
            management_generation: Some(4),
        })
        .unwrap(),
        fixture["yubiLifecycle"]
    );
    assert_eq!(
        serde_json::to_value(YubiSubkeyRecoveryDto {
            alias: "work_key".to_owned(),
            subkey_id: "0d".repeat(33),
            certificate_count: 2,
        })
        .unwrap(),
        fixture["yubiSubkeyRecovery"]
    );
    assert_eq!(
        serde_json::to_value(YubiRevocationDto {
            alias: "work_key".to_owned(),
            user_chain_sequence: 23,
            removed_local_credential: true,
        })
        .unwrap(),
        fixture["yubiRevocation"]
    );
    assert_eq!(
        serde_json::to_value(YubiChangedDto {
            alias: "work_key".to_owned(),
            changed: true,
        })
        .unwrap(),
        fixture["yubiChanged"]
    );
    let offer = PairingOfferDto {
        account_alias: "personal".to_owned(),
        phrase: Zeroizing::new(
            "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount".to_owned(),
        ),
    };
    assert_eq!(
        serde_json::to_value(offer).unwrap(),
        fixture["pairingOffer"]
    );
    let provision = DeviceProvisionDto {
        alias: "paired".to_owned(),
        device_id: "04".repeat(33),
        user_chain_sequence: 14,
    };
    assert_eq!(
        serde_json::to_value(provision).unwrap(),
        fixture["deviceProvision"]
    );
    let passphrase = PassphraseReportDto {
        generation: 2,
        stretch_version: "v1".to_owned(),
        verified: true,
    };
    assert_eq!(
        serde_json::to_value(passphrase).unwrap(),
        fixture["passphraseReport"]
    );
    let preview = ResetPreviewDto {
        profile: "work".to_owned(),
        resumables: vec![PendingOperationDto {
            kind: "pairing-offer",
            alias: "personal".to_owned(),
            target: None,
        }],
        artifacts: vec![ResetArtifactDto {
            kind: "hard-state",
            entries: 3,
            bytes: 4096,
        }],
        token: Zeroizing::new("one-use-reset-token".to_owned()),
        expires_in_seconds: 300,
    };
    assert_eq!(
        serde_json::to_value(preview).unwrap(),
        fixture["resetPreview"]
    );
}

#[test]
fn phase_six_wire_responses_are_exact_bounded_and_request_bound() {
    assert!(exact_profile_confirmation("work", "work", "reset").is_ok());
    for confirmation in [" work", "work ", "WORK", "other"] {
        assert_eq!(
            exact_profile_confirmation("work", confirmation, "reset")
                .unwrap_err()
                .code,
            "invalid-request"
        );
    }
    let added =
        added_server_response(test_profile_value("partner"), "partner", "foks.example").unwrap();
    assert_eq!(added.profile, "partner");
    assert_eq!(added.configured_probe, "foks.example");
    assert_eq!(
        added_server_response(
            serde_json::json!({
                "name":"partner",
                "probe":"foks.example",
                "protocol":{"generation":"v019"},
                "trust":{"kind":"web-pki"},
                "invented":true
            }),
            "partner",
            "foks.example"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    assert!(forgotten_server_response(
        serde_json::json!({"profile":"partner","removed":true}),
        "partner"
    )
    .is_ok());
    for malformed in [
        serde_json::json!({"profile":"other","removed":true}),
        serde_json::json!({"profile":"partner","removed":false}),
        serde_json::json!({"profile":"partner","removed":true,"invented":true}),
    ] {
        assert_eq!(
            forgotten_server_response(malformed, "partner")
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }

    let status = server_status_response(
        serde_json::json!({
            "profile":"work",
            "configured_probe":"foks.example",
            "host":{
                "lookup_name":"foks.example",
                "canonical_name":"foks.example",
                "host_id_hex":"02".repeat(33),
                "host_chain_sequence":4,
                "merkle_epoch":8
            },
            "chat_supported":true,
            "compatibility":{"status":"validated","expires_at":1_900_000_000u64,"capabilities":["chat","kv"]}
        }),
        "work",
        "foks.example",
        true,
    )
    .unwrap();
    assert_eq!(status.host.unwrap().chain, 4);
    assert_eq!(
        server_status_response(
            serde_json::json!({
                "profile":"work",
                "configured_probe":"foks.example",
                "host":null,
                "lease_required":false,
                "lease_expires_at":null,
                "chat_available":false
            }),
            "work",
            "foks.example",
            true,
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    for acceptance in ["inserted", "advanced", "unchanged"] {
        assert_eq!(
            checked_server_response(
                serde_json::json!({
                    "acceptance":acceptance,
                    "lookup_name":"foks.example",
                    "canonical_name":"foks.example",
                    "host_id_hex":"02".repeat(33),
                    "host_chain_sequence":5,
                    "merkle_epoch":9
                }),
                "work",
            )
            .unwrap()
            .acceptance,
            acceptance
        );
    }
    let versioned = checked_server_response(
        serde_json::json!({
            "acceptance":"unchanged",
            "lookup_name":"foks.example",
            "canonical_name":"foks.example",
            "host_id_hex":"02".repeat(33),
            "host_chain_sequence":5,
            "merkle_epoch":9,
            "server_version":{
                "minimum":"0.2.0",
                "newest":null,
                "message":"Upgrade required",
                "compatible":false
            }
        }),
        "work",
    )
    .unwrap();
    let version = versioned
        .server_version
        .expect("the server version was decoded");
    assert_eq!(version.minimum.as_deref(), Some("0.2.0"));
    assert_eq!(version.newest, None);
    assert!(!version.compatible);
    assert_eq!(
        checked_server_response(
            serde_json::json!({
                "acceptance":"unchanged",
                "lookup_name":"foks.example",
                "canonical_name":"foks.example",
                "host_id_hex":"02".repeat(33),
                "host_chain_sequence":5,
                "merkle_epoch":9,
                "server_version":{
                    "minimum":"0.2.0",
                    "newest":null,
                    "message":"",
                    "compatible":false,
                    "invented":true
                }
            }),
            "work",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    assert_eq!(
        checked_server_response(
            serde_json::json!({
                "acceptance":"invented",
                "lookup_name":"foks.example",
                "canonical_name":"foks.example",
                "host_id_hex":"02".repeat(33),
                "host_chain_sequence":5,
                "merkle_epoch":9
            }),
            "work",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    for malformed in [
        serde_json::json!({
            "profile":"other",
            "configured_probe":"foks.example",
            "host":null,
            "lease_required":false,
            "lease_expires_at":null,
                "chat_available":false
        }),
        serde_json::json!({
            "profile":"work",
            "configured_probe":"foks.example",
            "host":null,
            "lease_required":false,
            "lease_expires_at":1,
            "chat_available":false
        }),
        serde_json::json!({
            "profile":"work",
            "configured_probe":"foks.example",
            "host":null,
            "lease_required":false,
            "lease_expires_at":null,
            "chat_available":false,
            "invented":true
        }),
        serde_json::json!({
            "profile":"work",
            "configured_probe":"foks.example",
            "host":{
                "lookup_name":"foks.example",
                "canonical_name":"foks.example",
                "host_id_hex":"bad",
                "host_chain_sequence":0,
                "merkle_epoch":0
            },
            "lease_required":false,
            "lease_expires_at":null,
                "chat_available":false
        }),
    ] {
        assert_eq!(
            server_status_response(malformed, "work", "foks.example", false)
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }

    let devices = device_dtos(serde_json::json!([
        {"id_hex":"04".repeat(33),"name":"This Mac","role":"owner","current":true},
        {"id_hex":format!("04{}", "06".repeat(32)),"name":null,"role":"admin","current":false},
        {"id_hex":"08".repeat(34),"name":"Security key","role":"owner","current":false}
    ]))
    .unwrap();
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[0].name.as_deref(), Some("This Mac"));
    assert_eq!(devices[1].name, None);
    for malformed in [
        serde_json::json!([
            {"id_hex":"04".repeat(33),"role":"owner","current":true},
            {"id_hex":"04".repeat(33),"role":"owner","current":false}
        ]),
        serde_json::json!([{"id_hex":"04".repeat(33),"role":"robot","current":true}]),
        serde_json::json!([{"id_hex":"04".repeat(33),"role":"owner","current":false}]),
        serde_json::json!([{"id_hex":"04".repeat(33),"name":"bad\nname","role":"owner","current":true}]),
        serde_json::json!([{"id_hex":"04".repeat(33),"name":"x".repeat(257),"role":"owner","current":true}]),
    ] {
        assert_eq!(device_dtos(malformed).unwrap_err().code, "invalid-response");
    }

    assert!(backup_enrollment_dtos(
        serde_json::json!([{
            "backup_alias":"paper",
            "account_alias":"personal",
            "backup_id_hex":"10".repeat(33)
        }]),
        "personal"
    )
    .is_ok());
    assert_eq!(
        backup_enrollment_dtos(
            serde_json::json!([{
                "backup_alias":"paper",
                "account_alias":"other",
                "backup_id_hex":"10".repeat(33)
            }]),
            "personal"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let phrase = "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount";
    let offer = pairing_offer_response(
        serde_json::json!({"account_alias":"personal","phrase":phrase}),
        "personal",
    )
    .unwrap();
    assert!(!format!("{offer:?}").contains(phrase));
    assert!(device_provision_response(
        serde_json::json!({
            "alias":"paired",
            "device_id_hex":"04".repeat(33),
            "user_chain_sequence":0
        }),
        "paired"
    )
    .is_ok());
    assert!(device_removal_response(
        serde_json::json!({
            "device_id_hex":"04".repeat(33),
            "user_chain_sequence":0,
            "already_absent":false
        }),
        &"04".repeat(33)
    )
    .is_ok());
    assert_eq!(
        device_removal_response(
            serde_json::json!({
                "device_id_hex":"04".repeat(33),
                "user_chain_sequence":0,
                "already_absent":false,
                "invented":true
            }),
            &"04".repeat(33)
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let preview = reset_preview_response(
        serde_json::json!({
            "profile":"work",
            "resumables":[{"kind":"pairing-offer","alias":"personal","target":null}],
            "artifacts":[
                {"kind":"external-database-claim","entries":2,"bytes":100},
                {"kind":"hard-state","entries":1,"bytes":25},
                {"kind":"external-database-claim","entries":3,"bytes":250}
            ],
            "token":"one-use-token",
            "expires_in_seconds":300
        }),
        "work",
    )
    .unwrap();
    assert_eq!(preview.resumables[0].kind, "pairing-offer");
    assert_eq!(
        preview.artifacts,
        vec![
            ResetArtifactDto {
                kind: "external-database-claim",
                entries: 5,
                bytes: 350,
            },
            ResetArtifactDto {
                kind: "hard-state",
                entries: 1,
                bytes: 25,
            },
        ]
    );
    assert!(!format!("{preview:?}").contains("one-use-token"));
    for malformed in [
        serde_json::json!({
            "profile":"work","resumables":[],
            "artifacts":[
                {"kind":"external-database-claim","entries":u64::MAX,"bytes":1},
                {"kind":"external-database-claim","entries":1,"bytes":1}
            ],
            "token":"token","expires_in_seconds":300
        }),
        serde_json::json!({
            "profile":"work","resumables":[],"artifacts":[],
            "token":"","expires_in_seconds":300
        }),
        serde_json::json!({
            "profile":"work","resumables":[],"artifacts":[],
            "token":"token","expires_in_seconds":300,"invented":true
        }),
    ] {
        assert_eq!(
            reset_preview_response(malformed, "work").unwrap_err().code,
            "invalid-response"
        );
    }
}
