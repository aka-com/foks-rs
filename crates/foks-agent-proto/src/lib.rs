//! Versioned, bounded local protocol between standalone FOKS frontends and agent.

#![forbid(unsafe_code)]

mod frame;
mod message;

pub use frame::{decode_request, decode_response, encode, Error, Result, MAXIMUM_MESSAGE_BYTES};
pub use message::{
    ErrorCode, Operation, Request, Response, ResponseResult, SecretString, PROTOCOL_VERSION,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_responses_round_trip_with_version_binding() {
        let request = Request::new(
            7,
            Operation::SyncAccount {
                profile: "hosted".into(),
                alias: "personal".into(),
            },
        );
        assert_eq!(decode_request(&encode(&request).unwrap()).unwrap(), request);
        let response = Response::success(7, serde_json::json!({ "ok": true }));
        assert_eq!(
            decode_response(&encode(&response).unwrap()).unwrap(),
            response
        );
    }

    #[test]
    fn rejects_truncation_trailing_bytes_and_oversized_prefixes() {
        let request = encode(&Request::new(1, Operation::Ping)).unwrap();
        assert!(matches!(
            decode_request(&request[..request.len() - 1]),
            Err(Error::Length)
        ));
        let mut trailing = request.clone();
        trailing.push(0);
        assert!(matches!(decode_request(&trailing), Err(Error::Length)));
        assert!(matches!(
            decode_request(&(u32::MAX).to_be_bytes()),
            Err(Error::TooLarge)
        ));
    }

    #[test]
    fn unsupported_versions_fail_before_dispatch() {
        let request = Request {
            version: PROTOCOL_VERSION + 1,
            id: 1,
            operation: Operation::Ping,
        };
        assert!(matches!(
            decode_request(&encode(&request).unwrap()),
            Err(Error::Version)
        ));
    }

    #[test]
    fn json_wire_shape_is_stable_without_response_dtos() {
        let request = Request::new(
            9,
            Operation::SyncTeam {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            },
        );
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "version": 1,
                "id": 9,
                "operation": {
                    "operation": "sync-team",
                    "profile": "local",
                    "team_alias": "engineering"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(Response::error(9, ErrorCode::Busy, "locked")).unwrap(),
            serde_json::json!({
                "version": 1,
                "id": 9,
                "status": "error",
                "code": "busy",
                "message": "locked"
            })
        );
    }

    #[test]
    fn signup_invites_are_serialized_but_redacted_from_debug_output() {
        let operation = Operation::CreateAccount {
            profile: "local".to_owned(),
            alias: "personal".to_owned(),
            username: "rae".to_owned(),
            device_name: "laptop".to_owned(),
            email: String::new(),
            invite: "s.secret-invite".to_owned(),
            passphrase: Some(SecretString::new("secret-passphrase")),
        };
        let encoded = encode(&Request::new(10, operation.clone())).unwrap();
        assert_eq!(decode_request(&encoded).unwrap().operation, operation);
        let debug = format!("{operation:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-invite"));
        assert!(!debug.contains("secret-passphrase"));
    }

    #[test]
    fn every_passphrase_operation_redacts_its_secret() {
        let operations = [
            Operation::SetPassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("set-secret"),
            },
            Operation::ChangePassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("change-secret"),
            },
            Operation::VerifyPassphrase {
                profile: "local".to_owned(),
                alias: "personal".to_owned(),
                passphrase: SecretString::new("verify-secret"),
            },
        ];
        for (operation, secret) in
            operations
                .into_iter()
                .zip(["set-secret", "change-secret", "verify-secret"])
        {
            let debug = format!("{operation:?}");
            assert!(debug.contains("<redacted>"));
            assert!(!debug.contains(secret));
            assert_eq!(
                decode_request(&encode(&Request::new(11, operation.clone())).unwrap())
                    .unwrap()
                    .operation,
                operation
            );
        }
    }

    #[test]
    fn every_yubikey_operation_redacts_pin_puk_and_signup_secrets() {
        let operations = [
            Operation::CreateYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                username: "rae".to_owned(),
                device_name: "key".to_owned(),
                email: String::new(),
                invite: SecretString::new("s.yubi-invite"),
                passphrase: Some(SecretString::new("yubi-passphrase")),
                card_serial: 7,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin: SecretString::new("123456"),
            },
            Operation::ResumeYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("resume-pin"),
            },
            Operation::ProvisionYubiDevice {
                profile: "local".to_owned(),
                source_alias: "personal".to_owned(),
                target_alias: "hardware".to_owned(),
                device_name: "key".to_owned(),
                serial: 2,
                card_serial: 7,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin: SecretString::new("provision-pin"),
            },
            Operation::SyncYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("sync-pin"),
            },
            Operation::ChangeYubiPin {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                old_pin: SecretString::new("234567"),
                new_pin: SecretString::new("345678"),
            },
            Operation::ChangeYubiPuk {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                old_puk: SecretString::new("456789"),
                new_puk: SecretString::new("567890"),
            },
            Operation::UnblockYubiPin {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                puk: SecretString::new("789012"),
                new_pin: SecretString::new("890123"),
            },
            Operation::ConfigureYubiRetries {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("901234"),
                puk: SecretString::new("01234567"),
                pin_attempts: 5,
                puk_attempts: 7,
            },
            Operation::RotateYubiManagementKey {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("rotate-pin"),
            },
            Operation::ResumeYubiManagementKey {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: Some(SecretString::new("management-resume-pin")),
            },
            Operation::RecoverYubiSubkey {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("678901"),
            },
        ];
        for operation in operations {
            let decoded = decode_request(&encode(&Request::new(12, operation.clone())).unwrap())
                .unwrap()
                .operation;
            assert_eq!(decoded, operation);
            let debug = format!("{operation:?}");
            assert!(debug.contains("<redacted>"));
            for secret in [
                "s.yubi-invite",
                "yubi-passphrase",
                "123456",
                "resume-pin",
                "provision-pin",
                "sync-pin",
                "234567",
                "345678",
                "456789",
                "567890",
                "678901",
                "789012",
                "890123",
                "901234",
                "01234567",
                "rotate-pin",
                "management-resume-pin",
            ] {
                assert!(!debug.contains(secret));
            }
        }
    }
}
