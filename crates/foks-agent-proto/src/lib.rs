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
}
