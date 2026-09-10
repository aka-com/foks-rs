use crate::commands::accounts::{
    account_sync_response, backup_commit_response, backup_phrase_response,
    backup_revocation_response, passphrase_response,
};
use crate::commands::enrollment::{
    device_provision_response, go_profile_discovery_response, recovery_response,
    validated_pending_dtos, PendingOperationResponse,
};
use crate::commands::execution::execute_pending_operation;
use crate::commands::groups::{DiscoveredGroupResponse, GroupDiscoveryDto, GroupDiscoveryResponse};
use crate::commands::servers::{
    CheckedProbeResponse, CheckedProfileDto, CheckedProfileIdentity, CheckedProfileResponse,
};
use crate::commands::tests::support::test_profile_value;
use crate::commands::validation::{require_nested_response_row_cap, MAXIMUM_FIRST_RUN_ROWS};
use foks_agent_proto::{Operation, PendingOperationKind, PendingOperationSummary};
use std::sync::Mutex;

#[test]
fn first_run_response_projection_fails_closed() {
    let checked = CheckedProfileResponse {
        profile: CheckedProfileIdentity {
            name: "different".to_owned(),
            probe: "foks.example".to_owned(),
            protocol: serde_json::json!({"generation":"v019"}),
            trust: serde_json::json!({"kind":"web-pki"}),
        },
        probe: CheckedProbeResponse {
            acceptance: "inserted".to_owned(),
            lookup_name: "foks.example".to_owned(),
            canonical_name: "foks.example".to_owned(),
            host_id_hex: "01".to_owned(),
            host_chain_sequence: 1,
            merkle_epoch: 2,
            server_version: None,
        },
    };
    assert_eq!(
        CheckedProfileDto::from_response("work", "foks.example", checked)
            .unwrap_err()
            .code,
        "invalid-response"
    );

    let wrong_prefix_host = CheckedProfileResponse {
        profile: CheckedProfileIdentity {
            name: "work".to_owned(),
            probe: "foks.example".to_owned(),
            protocol: serde_json::json!({"generation":"v019"}),
            trust: serde_json::json!({"kind":"web-pki"}),
        },
        probe: CheckedProbeResponse {
            acceptance: "inserted".to_owned(),
            lookup_name: "foks.example".to_owned(),
            canonical_name: "foks.example".to_owned(),
            host_id_hex: "01".repeat(33),
            host_chain_sequence: 1,
            merkle_epoch: 2,
            server_version: None,
        },
    };
    assert_eq!(
        CheckedProfileDto::from_response("work", "foks.example", wrong_prefix_host)
            .unwrap_err()
            .code,
        "invalid-response"
    );
    assert!(CheckedProfileDto::from_response(
        "work",
        "foks.example",
        CheckedProfileResponse {
            profile: CheckedProfileIdentity {
                name: "work".to_owned(),
                probe: "foks.example".to_owned(),
                protocol: serde_json::json!({"generation":"v019"}),
                trust: serde_json::json!({"kind":"web-pki"}),
            },
            probe: CheckedProbeResponse {
                acceptance: "inserted".to_owned(),
                lookup_name: "foks.example".to_owned(),
                canonical_name: "foks.example".to_owned(),
                host_id_hex: "02".repeat(33),
                host_chain_sequence: 1,
                merkle_epoch: 2,
                server_version: None,
            },
        }
    )
    .is_ok());

    for acceptance in ["advanced", "unchanged"] {
        let response = CheckedProfileResponse {
            profile: CheckedProfileIdentity {
                name: "work".to_owned(),
                probe: "foks.example".to_owned(),
                protocol: serde_json::json!({"generation":"v019"}),
                trust: serde_json::json!({"kind":"web-pki"}),
            },
            probe: CheckedProbeResponse {
                acceptance: acceptance.to_owned(),
                lookup_name: "foks.example".to_owned(),
                canonical_name: "foks.example".to_owned(),
                host_id_hex: "02".repeat(33),
                host_chain_sequence: 1,
                merkle_epoch: 2,
                server_version: None,
            },
        };
        assert_eq!(
            CheckedProfileDto::from_response("work", "foks.example", response)
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }

    let discovery = GroupDiscoveryResponse {
        account_alias: "personal".to_owned(),
        teams: vec![DiscoveredGroupResponse {
            alias: "engineering".to_owned(),
            account_alias: "other".to_owned(),
            team_id_hex: "03".to_owned(),
            kind: "named".to_owned(),
            name: Some("Engineering".to_owned()),
            active: true,
        }],
    };
    assert_eq!(
        GroupDiscoveryDto::from_response("personal", discovery)
            .unwrap_err()
            .code,
        "invalid-response"
    );

    let duplicate_alias = GroupDiscoveryResponse {
        account_alias: "personal".to_owned(),
        teams: vec![
            DiscoveredGroupResponse {
                alias: "engineering".to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: "03".repeat(33),
                kind: "named".to_owned(),
                name: Some("Engineering".to_owned()),
                active: true,
            },
            DiscoveredGroupResponse {
                alias: "engineering".to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: format!("03{}", "04".repeat(32)),
                kind: "named".to_owned(),
                name: Some("Other".to_owned()),
                active: true,
            },
        ],
    };
    assert_eq!(
        GroupDiscoveryDto::from_response("personal", duplicate_alias)
            .unwrap_err()
            .code,
        "invalid-response"
    );
    let duplicate_id = GroupDiscoveryResponse {
        account_alias: "personal".to_owned(),
        teams: ["engineering", "operations"]
            .into_iter()
            .map(|alias| DiscoveredGroupResponse {
                alias: alias.to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: "03".repeat(33),
                kind: "named".to_owned(),
                name: Some(alias.to_owned()),
                active: true,
            })
            .collect(),
    };
    assert_eq!(
        GroupDiscoveryDto::from_response("personal", duplicate_id)
            .unwrap_err()
            .code,
        "invalid-response"
    );
    let invalid_local_alias = GroupDiscoveryResponse {
        account_alias: "personal".to_owned(),
        teams: vec![DiscoveredGroupResponse {
            alias: "engineering/group".to_owned(),
            account_alias: "personal".to_owned(),
            team_id_hex: "03".repeat(33),
            kind: "named".to_owned(),
            name: Some("Engineering".to_owned()),
            active: true,
        }],
    };
    assert_eq!(
        GroupDiscoveryDto::from_response("personal", invalid_local_alias)
            .unwrap_err()
            .code,
        "invalid-response"
    );

    for (kind, name, wrong_prefix) in [
        ("named", Some("Engineering".to_owned()), "14"),
        ("ad-hoc", None, "03"),
    ] {
        let wrong_type = GroupDiscoveryResponse {
            account_alias: "personal".to_owned(),
            teams: vec![DiscoveredGroupResponse {
                alias: "engineering".to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: wrong_prefix.repeat(33),
                kind: kind.to_owned(),
                name,
                active: true,
            }],
        };
        assert_eq!(
            GroupDiscoveryDto::from_response("personal", wrong_type)
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }
    let valid_typed_groups = GroupDiscoveryResponse {
        account_alias: "personal".to_owned(),
        teams: vec![
            DiscoveredGroupResponse {
                alias: "engineering".to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: "03".repeat(33),
                kind: "named".to_owned(),
                name: Some("Engineering".to_owned()),
                active: true,
            },
            DiscoveredGroupResponse {
                alias: "friends".to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: "14".repeat(33),
                kind: "ad-hoc".to_owned(),
                name: None,
                active: true,
            },
        ],
    };
    assert_eq!(
        GroupDiscoveryDto::from_response("personal", valid_typed_groups)
            .unwrap()
            .groups
            .len(),
        2
    );

    let duplicate_pending = vec![
        PendingOperationSummary {
            kind: PendingOperationKind::AccountSignup,
            alias: "personal".to_owned(),
            target: None,
        },
        PendingOperationSummary {
            kind: PendingOperationKind::AccountSignup,
            alias: "personal".to_owned(),
            target: None,
        },
    ];
    assert_eq!(
        validated_pending_dtos(&duplicate_pending).unwrap_err().code,
        "invalid-response"
    );
    let too_many_pending = vec![
        PendingOperationSummary {
            kind: PendingOperationKind::AccountSignup,
            alias: "personal".to_owned(),
            target: None,
        };
        MAXIMUM_FIRST_RUN_ROWS + 1
    ];
    assert_eq!(
        validated_pending_dtos(&too_many_pending).unwrap_err().code,
        "invalid-response"
    );
    assert!(
        serde_json::from_value::<PendingOperationResponse>(serde_json::json!({
            "kind":"account-signup",
            "alias":"personal",
            "target":null,
            "invented":true
        }))
        .is_err()
    );
    assert_eq!(
        require_nested_response_row_cap(
            &serde_json::json!({
                "teams": vec![serde_json::Value::Null; MAXIMUM_FIRST_RUN_ROWS + 1]
            }),
            "teams",
            "discovered groups",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
}

#[test]
fn every_first_run_mutation_success_is_shape_and_request_bound() {
    assert!(backup_phrase_response(
        serde_json::json!({
            "backup_alias":"paper",
            "phrase":"abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon"
        }),
        "paper"
    )
    .is_ok());
    for malformed in [
        serde_json::json!({"backup_alias":"other","phrase":"abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon"}),
        serde_json::json!({"backup_alias":"paper","phrase":"not a recovery phrase"}),
        serde_json::json!({"backup_alias":"paper","phrase":"abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon","invented":true}),
    ] {
        assert_eq!(
            backup_phrase_response(malformed, "paper").unwrap_err().code,
            "invalid-response"
        );
    }
    assert!(account_sync_response(serde_json::json!({
        "username": "Sol",
        "user_chain_sequence": 0,
        "directories": 1,
        "entries": 4
    }))
    .is_ok());
    assert_eq!(
        account_sync_response(serde_json::json!({
            "username": "Sol",
            "user_chain_sequence": 1,
            "directories": 1,
            "entries": 4,
            "invented": true
        }))
        .unwrap_err()
        .code,
        "invalid-response"
    );
    assert!(passphrase_response(serde_json::json!({
        "generation": 1,
        "stretch_version": "v1",
        "verified": true
    }))
    .is_ok());
    for malformed in [
        serde_json::json!({
            "generation": 0,
            "stretch_version": "v1",
            "verified": true
        }),
        serde_json::json!({
            "generation": 1,
            "stretch_version": "test",
            "verified": true
        }),
        serde_json::json!({
            "generation": 1,
            "stretch_version": "v1",
            "verified": false
        }),
    ] {
        assert_eq!(
            passphrase_response(malformed).unwrap_err().code,
            "invalid-response"
        );
    }

    assert!(backup_commit_response(
        serde_json::json!({
            "backup_alias": "paper",
            "account_alias": "personal",
            "backup_id_hex": "10".repeat(33),
            "user_chain_sequence": 0
        }),
        "personal",
        "paper"
    )
    .is_ok());
    assert_eq!(
        backup_commit_response(
            serde_json::json!({
                "backup_alias": "other",
                "account_alias": "personal",
                "backup_id_hex": "10".repeat(33),
                "user_chain_sequence": 1
            }),
            "personal",
            "paper"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    assert_eq!(
        backup_commit_response(
            serde_json::json!({
                "backup_alias": "paper",
                "account_alias": "personal",
                "backup_id_hex": "04".repeat(33),
                "user_chain_sequence": 1
            }),
            "personal",
            "paper"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let backup_id = "10".repeat(33);
    assert!(backup_revocation_response(
        serde_json::json!({
            "backup_alias": "paper",
            "account_alias": "personal",
            "backup_id_hex": backup_id.clone(),
            "user_chain_sequence": 2,
            "already_absent": false,
            "removed_local_enrollment": true
        }),
        "personal",
        "paper",
        &backup_id,
    )
    .is_ok());
    for malformed in [
        serde_json::json!({
            "backup_alias": "other",
            "account_alias": "personal",
            "backup_id_hex": backup_id.clone(),
            "user_chain_sequence": 2,
            "already_absent": false,
            "removed_local_enrollment": true
        }),
        serde_json::json!({
            "backup_alias": "paper",
            "account_alias": "personal",
            "backup_id_hex": backup_id.clone(),
            "user_chain_sequence": 2,
            "already_absent": false,
            "removed_local_enrollment": false
        }),
        serde_json::json!({
            "backup_alias": "paper",
            "account_alias": "personal",
            "backup_id_hex": backup_id.clone(),
            "user_chain_sequence": 2,
            "already_absent": false,
            "removed_local_enrollment": true,
            "invented": true
        }),
    ] {
        assert_eq!(
            backup_revocation_response(malformed, "personal", "paper", &backup_id)
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }

    assert!(recovery_response(
        serde_json::json!({
            "alias": "recovered",
            "device_id_hex": "04".repeat(33),
            "user_chain_sequence": 0
        }),
        "recovered"
    )
    .is_ok());
    assert_eq!(
        recovery_response(
            serde_json::json!({
                "alias": "different",
                "device_id_hex": "04".repeat(33),
                "user_chain_sequence": 1
            }),
            "recovered"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    assert_eq!(
        recovery_response(
            serde_json::json!({
                "alias": "recovered",
                "device_id_hex": "05".repeat(33),
                "user_chain_sequence": 1
            }),
            "recovered"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
}

pub(super) struct FirstRunTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for FirstRunTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles => {
                Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
            }
            Operation::ListPendingOperations { .. } => Ok(serde_json::json!([{
                "kind": "account-signup",
                "alias": "personal",
                "target": null
            }])),
            Operation::ResumeAccount { .. } => Ok(serde_json::json!({
                "username": "sol",
                "user_chain_sequence": 3,
                "directories": 1,
                "entries": 0
            })),
            other => panic!("unexpected first-run operation {other:?}"),
        }
    }
}

#[test]
fn first_run_mutation_resolves_profile_then_issues_one_explicit_resume() {
    let transport = FirstRunTransport {
        calls: Mutex::new(Vec::new()),
    };
    execute_pending_operation(
        &transport,
        "work",
        PendingOperationKind::AccountSignup,
        "personal",
        None,
        Operation::ResumeAccount {
            profile: "work".to_owned(),
            alias: "personal".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::ListPendingOperations {
                profile: "work".to_owned(),
            },
            Operation::ResumeAccount {
                profile: "work".to_owned(),
                alias: "personal".to_owned(),
            },
        ]
    );
}

#[test]
fn go_profile_candidates_are_bounded_and_copy_support_is_coherent() {
    let candidate = serde_json::json!({
        "candidate_id": "aa".repeat(32),
        "username": null,
        "server_hint": null,
        "host_id_hex": format!("02{}", "bb".repeat(32)),
        "user_id_hex": format!("01{}", "cc".repeat(32)),
        "device_id_hex": format!("04{}", "dd".repeat(32)),
        "role": "owner",
        "storage_kind": "macos-keychain",
        "hidden": false,
        "provisional": false,
        "pairable": true,
        "copyable": true
    });
    let decoded = go_profile_discovery_response(serde_json::json!({
        "installed": true,
        "candidates": [candidate.clone()]
    }))
    .unwrap();
    assert_eq!(decoded.candidates.len(), 1);
    assert!(decoded.candidates[0].copyable);
    let mut invalid = candidate;
    invalid["storage_kind"] = serde_json::json!("passphrase");
    assert!(go_profile_discovery_response(serde_json::json!({
        "installed": true,
        "candidates": [invalid]
    }))
    .is_err());
}

pub(super) struct PairingResumeTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for PairingResumeTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles => {
                Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
            }
            Operation::ListPendingOperations { .. } => Ok(serde_json::json!([{
                "kind":"pairing-acceptance",
                "alias":"paired",
                "target":null
            }])),
            Operation::ResumeDevicePairingAcceptance { .. } => Ok(serde_json::json!({
                "alias":"paired",
                "device_id_hex":"04".repeat(33),
                "user_chain_sequence":7
            })),
            other => panic!("unexpected pairing resume operation {other:?}"),
        }
    }
}

#[test]
fn pairing_resume_rechecks_one_authenticated_pending_identity() {
    let transport = PairingResumeTransport {
        calls: Mutex::new(Vec::new()),
    };
    let value = execute_pending_operation(
        &transport,
        "work",
        PendingOperationKind::PairingAcceptance,
        "paired",
        None,
        Operation::ResumeDevicePairingAcceptance {
            profile: "work".to_owned(),
            target_alias: "paired".to_owned(),
        },
    )
    .unwrap();
    assert!(device_provision_response(value, "paired").is_ok());
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::ListPendingOperations {
                profile: "work".to_owned()
            },
            Operation::ResumeDevicePairingAcceptance {
                profile: "work".to_owned(),
                target_alias: "paired".to_owned()
            }
        ]
    );
}
