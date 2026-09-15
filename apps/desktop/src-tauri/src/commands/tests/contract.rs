use crate::agent::AgentError;
use crate::commands::accounts::{
    backup_enrollment_dtos, device_dtos, device_removal_response, AccountDto, BackupEnrollmentDto,
    BackupPhraseDto, DeviceDto, DeviceRemovalDto, PassphraseReportDto,
};
use crate::commands::application::AppInfo;
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
    reset_preview_response, server_status_response, AddedServerDto, CheckedProfileDto,
    CheckedServerDto, CheckedServerVersionDto, ForgottenServerDto, ResetArtifactDto,
    ResetPreviewDto, ServerStatusSnapshotDto, StoredHostDto,
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
fn wire_contract_fixture_matches_serialized_shapes() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../wire-contract.json")).unwrap();
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
            username: Some("raymond".to_owned()),
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
        store: "opaque-account-ref".to_owned(),
        profile: "foks.example".to_owned(),
        alias: "personal".to_owned(),
        username: "rae.chen".to_owned(),
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
            fields: foks_agent_proto::ErrorFields::default(),
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
            },
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
        lease_required: true,
        lease_expires_at: Some(1_900_000_000),
        chat_available: true,
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
        })
        .unwrap(),
        fixture["yubiEnrollment"]
    );
    assert_eq!(
        serde_json::to_value(YubiAccountDto {
            alias: "work_key".to_owned(),
            username: "rae".to_owned(),
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
            username: "rae".to_owned(),
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
            "lease_required":true,
            "lease_expires_at":1_900_000_000u64,
            "chat_available":true
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
