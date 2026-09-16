use crate::agent::AgentHandle;
use crate::commands::accounts::DeviceDto;
use crate::commands::context::AppState;
use crate::commands::execution::{ambiguous_mutation_response, execute_pending_operation};
use crate::commands::tests::support::test_profile_value;
use crate::commands::validation::{yubi_retry_configuration, yubi_slots, YUBI_ID_HEX_BYTES};
use crate::commands::yubikey::{
    require_software_revocation_signer, require_yubi_enrollment, yubi_account_response,
    yubi_card_dtos, yubi_changed_response, yubi_enrollment_dtos, yubi_lifecycle_response,
    yubi_pin_status_response, yubi_revocation_response, yubi_subkey_recovery_response,
    yubi_sync_response, YubiCardDto,
};
use foks_agent_proto::{Operation, PendingOperationKind, SecretString};
use std::sync::{Arc, Mutex};

#[test]
fn security_key_responses_are_exact_typed_and_state_bound() {
    assert_eq!(yubi_slots(0x82, 0x83).unwrap(), (0x82, 0x83));
    for (signing, pq) in [(0x82, 0x82), (0x81, 0x83), (0x82, 0x96)] {
        assert_eq!(yubi_slots(signing, pq).unwrap_err().code, "invalid-request");
    }
    assert_eq!(
        yubi_retry_configuration("12345678".to_owned(), 0, 3)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let cards = yubi_card_dtos(serde_json::json!([
        {"name":"YubiKey 5","serial":42},
        {"name":"YubiKey Bio","serial":43}
    ]))
    .unwrap();
    assert_eq!(cards[0], YubiCardDto { serial: 42 });
    for malformed in [
        serde_json::json!([{"name":"YubiKey","serial":0}]),
        serde_json::json!([
            {"name":"YubiKey","serial":42},
            {"name":"Other","serial":42}
        ]),
        serde_json::json!([{"name":"YubiKey","serial":42,"status":"ready"}]),
    ] {
        assert_eq!(
            yubi_card_dtos(malformed).unwrap_err().code,
            "invalid-response"
        );
    }

    let enrollments = yubi_enrollment_dtos(serde_json::json!([
        {"alias":"pending_key","state":"pending"},
        {"alias":"ready_key","state":"complete"}
    ]))
    .unwrap();
    assert!(require_yubi_enrollment(&enrollments, "pending_key", "pending").is_ok());
    assert!(require_yubi_enrollment(&enrollments, "ready_key", "complete").is_ok());
    let completed_for_resume =
        require_yubi_enrollment(&enrollments, "ready_key", "pending").unwrap_err();
    assert_eq!(completed_for_resume.code, "security-key-state-changed");
    assert!(completed_for_resume.message.contains("Refresh"));
    assert!(!completed_for_resume.message.contains("Resume"));
    assert_eq!(
        require_yubi_enrollment(&enrollments, "unknown", "complete")
            .unwrap_err()
            .code,
        "security-key-not-found"
    );
    for crash_recovery_rows in [
        serde_json::json!([
            {"alias":"resumable_key","state":"pending"},
            {"alias":"resumable_key","state":"complete"}
        ]),
        serde_json::json!([
            {"alias":"resumable_key","state":"complete"},
            {"alias":"resumable_key","state":"pending"}
        ]),
    ] {
        let crash_recovery = yubi_enrollment_dtos(crash_recovery_rows).unwrap();
        assert!(require_yubi_enrollment(&crash_recovery, "resumable_key", "pending").is_ok());
        let error =
            require_yubi_enrollment(&crash_recovery, "resumable_key", "complete").unwrap_err();
        assert_eq!(error.code, "security-key-state-changed");
        assert!(error.message.contains("Resume"));
        assert!(error.message.contains("refresh"));
    }
    for malformed in [
        serde_json::json!([
            {"alias":"same","state":"pending"},
            {"alias":"same","state":"pending"}
        ]),
        serde_json::json!([
            {"alias":"same","state":"complete"},
            {"alias":"same","state":"complete"}
        ]),
        serde_json::json!([{"alias":"ready","state":"maybe"}]),
        serde_json::json!([{"alias":"ready","state":"complete","serial":42}]),
    ] {
        assert_eq!(
            yubi_enrollment_dtos(malformed).unwrap_err().code,
            "invalid-response"
        );
    }

    let account = yubi_account_response(
        serde_json::json!({
            "alias":"ready_key",
            "username":"satoshi",
            "yubi_id_hex":"08".repeat(34),
            "subkey_id_hex":"0d".repeat(33),
            "user_chain_sequence":0,
            "management_enrolled":true
        }),
        "ready_key",
        None,
    )
    .unwrap();
    assert_eq!(account.yubi_id.len(), YUBI_ID_HEX_BYTES);
    assert_eq!(
        yubi_account_response(
            serde_json::json!({
                "alias":"ready_key","username":"other",
                "yubi_id_hex":"08".repeat(34),"subkey_id_hex":"0d".repeat(33),
                "user_chain_sequence":0,"management_enrolled":true
            }),
            "ready_key",
            Some("satoshi"),
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    for malformed in [
        serde_json::json!({
            "alias":"other","username":"satoshi","yubi_id_hex":"08".repeat(34),
            "subkey_id_hex":"0d".repeat(33),"user_chain_sequence":0,
            "management_enrolled":true
        }),
        serde_json::json!({
            "alias":"ready_key","username":"satoshi","yubi_id_hex":"04".repeat(34),
            "subkey_id_hex":"0d".repeat(33),"user_chain_sequence":0,
            "management_enrolled":true
        }),
        serde_json::json!({
            "alias":"ready_key","username":"satoshi","yubi_id_hex":"08".repeat(34),
            "subkey_id_hex":"0d".repeat(33),"user_chain_sequence":0,
            "management_enrolled":false
        }),
    ] {
        assert_eq!(
            yubi_account_response(malformed, "ready_key", None)
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }

    assert!(yubi_pin_status_response(serde_json::json!({"remaining":3,"blocked":false})).is_ok());
    assert_eq!(
        yubi_pin_status_response(serde_json::json!({"remaining":0,"blocked":false}))
            .unwrap_err()
            .code,
        "invalid-response"
    );
    assert!(yubi_lifecycle_response(
        serde_json::json!({
            "alias":"ready_key","management_enrolled":true,"management_generation":1
        }),
        "ready_key"
    )
    .is_ok());
    for malformed in [
        serde_json::json!({
            "alias":"ready_key","management_enrolled":true,"management_generation":0
        }),
        serde_json::json!({
            "alias":"ready_key","management_enrolled":true,"management_generation":null
        }),
        serde_json::json!({
            "alias":"ready_key","management_enrolled":false,"management_generation":1
        }),
    ] {
        assert_eq!(
            yubi_lifecycle_response(malformed, "ready_key")
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }
    assert!(yubi_revocation_response(
        serde_json::json!({
            "alias":"ready_key","user_chain_sequence":0,"removed_local_credential":true
        }),
        "ready_key"
    )
    .is_ok());
    assert_eq!(
        yubi_revocation_response(
            serde_json::json!({
                "alias":"ready_key","user_chain_sequence":0,
                "removed_local_credential":false
            }),
            "ready_key"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let software_current = vec![DeviceDto {
        id: "04".repeat(33),
        name: None,
        role: "owner",
        current: true,
    }];
    assert!(require_software_revocation_signer(&software_current).is_ok());
    let yubi_current = vec![DeviceDto {
        id: "08".repeat(34),
        name: None,
        role: "owner",
        current: true,
    }];
    assert_eq!(
        require_software_revocation_signer(&yubi_current)
            .unwrap_err()
            .code,
        "security-key-current"
    );
}

#[test]
fn security_key_sync_and_lifecycle_reports_reject_malformed_success() {
    let sync = yubi_sync_response(
        serde_json::json!({
            "sync":{
                "username":"satoshi","user_chain_sequence":4,"directories":2,"entries":3
            },
            "federation":[{
                "local_profile":"work","local_team_alias":"engineering",
                "refreshed":true,"deferred":null
            }]
        }),
        "work",
        true,
    )
    .unwrap();
    assert_eq!(sync.federation.len(), 1);
    for malformed in [
        serde_json::json!({
            "sync":{"username":"satoshi","user_chain_sequence":4,"directories":2,"entries":3},
            "federation":[{
                "local_profile":"other","local_team_alias":"engineering",
                "refreshed":true,"deferred":null
            }]
        }),
        serde_json::json!({
            "sync":{"username":"satoshi","user_chain_sequence":4,"directories":2,"entries":3},
            "federation":[{
                "local_profile":"work","local_team_alias":"engineering",
                "refreshed":false,"deferred":null
            }]
        }),
        serde_json::json!({
            "sync":{"username":"satoshi","user_chain_sequence":4,"directories":2,"entries":3},
            "federation":[{
                "local_profile":"work","local_team_alias":"engineering",
                "refreshed":true,"deferred":null,"invented":true
            }]
        }),
    ] {
        assert_eq!(
            yubi_sync_response(malformed, "work", true)
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }
    assert!(yubi_subkey_recovery_response(
        serde_json::json!({
            "alias":"ready_key","subkey_id_hex":"0d".repeat(33),"certificate_count":2
        }),
        "ready_key"
    )
    .is_ok());
    assert!(yubi_changed_response(
        serde_json::json!({"alias":"ready_key","changed":true}),
        "ready_key"
    )
    .is_ok());

    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let error = yubi_revocation_response(
        serde_json::json!({
            "alias":"other","user_chain_sequence":9,"removed_local_credential":true
        }),
        "ready_key",
    )
    .map_err(|error| ambiguous_mutation_response(&state, error.message))
    .unwrap_err();
    assert_eq!(error.code, "response-binding");
    assert!(error.ambiguous && error.fatal);
}

pub(super) struct YubiResumeTransport {
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl foks_desktop::AgentTransport for YubiResumeTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles => {
                Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
            }
            Operation::ListPendingOperations { .. } => Ok(serde_json::json!([{
                "kind":"yubi-enrollment","alias":"work_key","target":null
            }])),
            Operation::ResumeYubiAccount { .. } => Ok(serde_json::json!({
                "alias":"work_key","username":"satoshi",
                "yubi_id_hex":"08".repeat(34),"subkey_id_hex":"0d".repeat(33),
                "user_chain_sequence":8,"management_enrolled":true
            })),
            other => panic!("unexpected security-key resume operation {other:?}"),
        }
    }
}

#[test]
fn security_key_resume_re_resolves_one_pending_journal_before_one_attempt() {
    let transport = YubiResumeTransport {
        calls: Mutex::new(Vec::new()),
    };
    let value = execute_pending_operation(
        &transport,
        "work",
        PendingOperationKind::YubiEnrollment,
        "work_key",
        None,
        Operation::ResumeYubiAccount {
            profile: "work".to_owned(),
            alias: "work_key".to_owned(),
            pin: SecretString::new("123456"),
        },
    )
    .unwrap();
    assert!(yubi_account_response(value, "work_key", None).is_ok());
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::ListPendingOperations {
                profile: "work".to_owned(),
            },
            Operation::ResumeYubiAccount {
                profile: "work".to_owned(),
                alias: "work_key".to_owned(),
                pin: SecretString::new("123456"),
            },
        ]
    );
}

#[test]
fn enrollment_identity_is_preserved_and_validated() {
    let id = format!("0802{}", "ab".repeat(32));
    let rows = yubi_enrollment_dtos(serde_json::json!([{
        "alias": "travel", "state": "complete", "device_id_hex": id, "card_serial": 123
    }]))
    .unwrap();
    assert_eq!(rows[0].device_id.as_deref(), Some(id.as_str()));
    assert_eq!(rows[0].card_serial, Some(123));
    assert!(yubi_enrollment_dtos(serde_json::json!([{
        "alias": "bad", "state": "complete", "device_id_hex": "bad", "card_serial": 123
    }]))
    .is_err());
}
