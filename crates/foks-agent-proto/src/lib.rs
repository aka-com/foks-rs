//! Versioned, bounded local protocol between standalone FOKS frontends and agent.

#![forbid(unsafe_code)]

pub mod account;
pub mod admin;
pub mod bot;
pub mod chat;
pub mod data;
mod frame;
pub mod invitations;
mod message;
pub mod sso;

pub use frame::{
    decode_request, decode_response, decode_upload_frame, encode, request_id, Error, Result,
    MAXIMUM_MESSAGE_BYTES,
};
pub use message::{
    AccountStoreRef, AccountSummary, AgentStatus, BackupEnrollmentSummary, CompatibilityFailure,
    CompatibilityStatus, CredentialBackend, DeviceSummary, ErrorCode, ErrorFields, FederationRole,
    GoProfileCandidate, GoProfileDiscovery, KnownStoreSummary, KvChunkResult, KvEntryMetadata,
    KvPage, KvPrecondition, KvReadResult, KvRole, KvStoreRef, KvUploadFrame, KvUploadHeader,
    KvUploadPayload, Operation, PendingOperationKind, PendingOperationSummary, ProfileOverview,
    ProfileProtocol, ProfileTrust, Request, ResetArtifactKind, ResetArtifactSummary,
    ResetStatePreview, Response, ResponseResult, SecretString, ServerStatusSnapshot,
    StoredHostStatus, TeamDetailsSummary, TeamKind, TeamRole, TeamStoreRef, TeamSummary,
    YubiFederationUnlockInput, YubiRetryConfiguration, PROTOCOL_VERSION,
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
    fn go_profile_discovery_is_read_only_and_has_a_stable_shape() {
        let request = Request::new(8, Operation::DiscoverGoProfiles);
        assert!(!request.operation.is_mutation());
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "version": 24,
                "id": 8,
                "operation": { "operation": "discover-go-profiles" }
            })
        );
    }

    #[test]
    fn profile_overview_is_read_only_and_profile_bound() {
        let request = Request::new(
            9,
            Operation::ListProfileOverview {
                profile: "hosted".to_owned(),
            },
        );
        assert!(!request.operation.is_mutation());
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "version": 24,
                "id": 9,
                "operation": {
                    "operation": "list-profile-overview",
                    "profile": "hosted"
                }
            })
        );
    }

    #[test]
    fn kv_stream_frames_bind_request_offset_and_commit() {
        let chunk = KvUploadFrame {
            version: PROTOCOL_VERSION,
            id: 44,
            payload: KvUploadPayload::Chunk {
                offset: 128,
                content: b"secret bytes".to_vec(),
            },
        };
        assert_eq!(
            decode_upload_frame(&encode(&chunk).unwrap()).unwrap(),
            chunk
        );
        let commit = KvUploadFrame {
            version: PROTOCOL_VERSION,
            id: 44,
            payload: KvUploadPayload::Commit,
        };
        assert_eq!(
            decode_upload_frame(&encode(&commit).unwrap()).unwrap(),
            commit
        );
    }

    #[test]
    fn serialized_inline_kv_plaintext_can_be_cleared_in_place() {
        let mut operation = Operation::PutKv {
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            path: "/secret".to_owned(),
            content: b"sensitive".to_vec(),
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            precondition: KvPrecondition::Create,
            mkdir_p: true,
        };
        operation.zeroize_plaintext();
        assert!(matches!(
            operation,
            Operation::PutKv { ref content, .. } if content.iter().all(|byte| *byte == 0)
        ));
    }

    #[test]
    fn team_creation_and_resume_keep_native_kind_and_identity() {
        for operation in [
            Operation::CreateTeam {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "engineering".to_owned(),
                name: "engineeringteam".to_owned(),
                kind: TeamKind::Named,
            },
            Operation::CreateTeam {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "project".to_owned(),
                name: String::new(),
                kind: TeamKind::AdHoc,
            },
            Operation::ResumeTeamCreation {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            },
        ] {
            assert_eq!(
                decode_request(&encode(&Request::new(91, operation.clone())).unwrap())
                    .unwrap()
                    .operation,
                operation
            );
        }
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
    fn json_wire_shape_is_stable_with_current_envelopes() {
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
                "version": 24,
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
                "version": 24,
                "id": 9,
                "status": "error",
                "code": "busy",
                "message": "locked"
            })
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::RateLimited).unwrap(),
            serde_json::json!("rate-limited")
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::QuotaExceeded).unwrap(),
            serde_json::json!("quota-exceeded")
        );
    }

    #[test]
    fn federation_operations_round_trip_with_explicit_profile_bindings() {
        let admission = Request::new(
            13,
            Operation::AdmitFederatedTeam {
                local_profile: "local".to_owned(),
                local_team_alias: "engineering".to_owned(),
                remote_profile: "partner".to_owned(),
                remote_team_alias: "security".to_owned(),
                role: FederationRole::Member,
                visibility: 0,
            },
        );
        assert_eq!(
            serde_json::to_value(&admission).unwrap(),
            serde_json::json!({
                "version": 24,
                "id": 13,
                "operation": {
                    "operation": "admit-federated-team",
                    "local_profile": "local",
                    "local_team_alias": "engineering",
                    "remote_profile": "partner",
                    "remote_team_alias": "security",
                    "role": "member",
                    "visibility": 0
                }
            })
        );
        assert_eq!(
            decode_request(&encode(&admission).unwrap()).unwrap(),
            admission
        );

        let listing = Request::new(
            14,
            Operation::ListFederatedTeams {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            },
        );
        assert_eq!(decode_request(&encode(&listing).unwrap()).unwrap(), listing);

        let expulsion = Request::new(
            15,
            Operation::ExpelFederatedTeam {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                remote_host_id_hex: format!("02{}", "22".repeat(32)),
                remote_team_id_hex: format!("03{}", "33".repeat(32)),
            },
        );
        assert_eq!(
            serde_json::to_value(&expulsion).unwrap(),
            serde_json::json!({
                "version": 24,
                "id": 15,
                "operation": {
                    "operation": "expel-federated-team",
                    "profile": "local",
                    "team_alias": "engineering",
                    "remote_host_id_hex": format!("02{}", "22".repeat(32)),
                    "remote_team_id_hex": format!("03{}", "33".repeat(32))
                }
            })
        );
        assert!(expulsion.operation.is_mutation());
        assert_eq!(
            decode_request(&encode(&expulsion).unwrap()).unwrap(),
            expulsion
        );
    }

    #[test]
    fn catalog_dtos_bind_store_cursor_and_native_roles() {
        let operation = Operation::ListTeamKv {
            store: TeamStoreRef {
                profile: "local".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "engineering".to_owned(),
                team_id: "a1".repeat(33),
            },
            cursor: Some("v2.cursor".to_owned()),
            limit: 200,
        };
        let request = Request::new(31, operation.clone());
        assert_eq!(decode_request(&encode(&request).unwrap()).unwrap(), request);
        assert!(!operation.is_mutation());

        let page = KvPage {
            snapshot_version: 9,
            entries: vec![KvEntryMetadata {
                path: "/shared.txt".to_owned(),
                node_type: "small-file".to_owned(),
                version: 4,
                size: None,
                read_role: KvRole::Member { visibility: 0 },
                write_role: KvRole::Admin,
            }],
            next_cursor: None,
        };
        let encoded = serde_json::to_value(&page).unwrap();
        assert_eq!(serde_json::from_value::<KvPage>(encoded).unwrap(), page);
    }

    #[test]
    fn protocol_extensions_bind_team_writes_and_resumable_operations() {
        let team = KvStoreRef::Team(TeamStoreRef {
            profile: "local".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "a1".repeat(33),
        });
        for operation in [
            Operation::PutKv {
                store: team.clone(),
                path: "/secret".to_owned(),
                content: b"value".to_vec(),
                read_role: KvRole::Member { visibility: 0 },
                write_role: KvRole::Admin,
                precondition: KvPrecondition::Create,
                mkdir_p: true,
            },
            Operation::RemoveKv {
                store: team,
                path: "/secret".to_owned(),
                recursive: false,
                precondition: KvPrecondition::ExactVersion { version: 3 },
            },
            Operation::ListPendingOperations {
                profile: "local".to_owned(),
            },
            Operation::RefreshLease {
                profile: "hosted".to_owned(),
            },
            Operation::RemoveDevice {
                profile: "local".to_owned(),
                signer_alias: "personal".to_owned(),
                device_id: "04".to_owned() + &"ab".repeat(32),
            },
        ] {
            let request = Request::new(47, operation.clone());
            assert_eq!(decode_request(&encode(&request).unwrap()).unwrap(), request);
        }

        let pending = vec![PendingOperationSummary {
            kind: PendingOperationKind::PairingAcceptance,
            alias: "new-laptop".to_owned(),
            target: None,
        }];
        let encoded = serde_json::to_value(&pending).unwrap();
        assert_eq!(
            serde_json::from_value::<Vec<PendingOperationSummary>>(encoded).unwrap(),
            pending
        );
    }

    #[test]
    fn bootstrap_and_profile_operations_have_stable_typed_envelopes() {
        let initialize = Operation::InitializeState {
            backend: CredentialBackend::Native,
        };
        assert!(initialize.is_mutation());
        assert_eq!(
            serde_json::to_value(&initialize).unwrap(),
            serde_json::json!({
                "operation": "initialize-state",
                "backend": "native"
            })
        );

        let profile = Operation::AddProfile {
            name: "local".to_owned(),
            probe: "foks.example:443".to_owned(),
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        };
        let request = Request::new(32, profile);
        assert_eq!(decode_request(&encode(&request).unwrap()).unwrap(), request);
        assert_eq!(
            serde_json::from_value::<AgentStatus>(serde_json::json!({
                "state": "bootstrap",
                "step": "initialize-state"
            }))
            .unwrap(),
            AgentStatus::Bootstrap {
                step: "initialize-state".to_owned()
            }
        );
    }

    #[test]
    fn request_ids_survive_version_errors_but_not_malformed_frames() {
        let request = Request {
            version: PROTOCOL_VERSION + 1,
            id: 44,
            operation: Operation::Ping,
        };
        let frame = encode(&request).unwrap();
        assert!(matches!(decode_request(&frame), Err(Error::Version)));
        assert_eq!(request_id(&frame), Some(44));
        assert_eq!(request_id(&[0, 0, 0, 1, b'{']), None);

        let response = Response::error_without_id(ErrorCode::InvalidRequest, "bad frame");
        assert_eq!(response.id, None);
    }

    #[test]
    fn version_is_rejected_before_unknown_request_or_response_shapes() {
        let request = encode(&serde_json::json!({
            "version": PROTOCOL_VERSION + 1,
            "id": 45,
            "operation": { "future-operation": { "field": true } }
        }))
        .unwrap();
        assert!(matches!(decode_request(&request), Err(Error::Version)));
        assert_eq!(request_id(&request), Some(45));

        let response = encode(&serde_json::json!({
            "version": PROTOCOL_VERSION + 1,
            "id": 45,
            "result": { "future-result": { "field": true } }
        }))
        .unwrap();
        assert!(matches!(decode_response(&response), Err(Error::Version)));
    }

    #[test]
    fn structured_error_fields_are_bounded_and_optional() {
        let response = Response::error_with_fields(
            Some(8),
            ErrorCode::CheckpointResetRequired,
            "x".repeat(5000),
            ErrorFields {
                profile: Some("personal".to_owned()),
                reason: Some("y".repeat(5000)),
                found_schema: Some(23),
                supported_schema: Some(27),
                ..ErrorFields::default()
            },
        );
        let ResponseResult::Error {
            message, fields, ..
        } = response.result
        else {
            panic!("expected error response");
        };
        assert_eq!(message.len(), 4096);
        assert_eq!(fields.reason.unwrap().len(), 4096);
        assert_eq!(fields.profile.as_deref(), Some("personal"));
        assert_eq!(fields.found_schema, Some(23));
        assert_eq!(fields.supported_schema, Some(27));
    }

    #[test]
    fn local_team_member_operations_round_trip_with_roles_and_recovery() {
        let operations = [
            Operation::ListTeamMembers {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            },
            Operation::AddTeamMember {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                username: "alice".to_owned(),
                role: TeamRole::Admin,
                visibility: 0,
            },
            Operation::ResumeTeamMemberAddition {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                username: "alice".to_owned(),
            },
            Operation::DemoteTeamMember {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                party_id_hex: format!("01{}", "11".repeat(32)),
                role: TeamRole::Member,
                visibility: -1,
            },
            Operation::RemoveTeamMember {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                party_id_hex: format!("01{}", "22".repeat(32)),
            },
            Operation::ResumeTeamMemberEdit {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
            },
        ];
        for (index, operation) in operations.into_iter().enumerate() {
            let request = Request::new(20 + index as u64, operation);
            assert_eq!(decode_request(&encode(&request).unwrap()).unwrap(), request);
        }
    }

    #[test]
    fn roster_selector_and_device_name_wire_shapes_are_stable() {
        let party_id_hex = format!("01{}", "ab".repeat(32));
        assert_eq!(
            serde_json::to_value(Request::new(
                25,
                Operation::DemoteTeamMember {
                    profile: "local".to_owned(),
                    team_alias: "engineering".to_owned(),
                    party_id_hex: party_id_hex.clone(),
                    role: TeamRole::Member,
                    visibility: 0,
                },
            ))
            .unwrap(),
            serde_json::json!({
                "version": 24,
                "id": 25,
                "operation": {
                    "operation": "demote-team-member",
                    "profile": "local",
                    "team_alias": "engineering",
                    "party_id_hex": party_id_hex,
                    "role": "member",
                    "visibility": 0,
                }
            })
        );
        assert_eq!(
            serde_json::to_value(Request::new(
                26,
                Operation::RemoveTeamMember {
                    profile: "local".to_owned(),
                    team_alias: "engineering".to_owned(),
                    party_id_hex: party_id_hex.clone(),
                },
            ))
            .unwrap(),
            serde_json::json!({
                "version": 24,
                "id": 26,
                "operation": {
                    "operation": "remove-team-member",
                    "profile": "local",
                    "team_alias": "engineering",
                    "party_id_hex": party_id_hex,
                }
            })
        );
        assert_eq!(
            serde_json::to_value(DeviceSummary {
                id_hex: format!("04{}", "cd".repeat(32)),
                name: Some("Satoshi's MacBook Air".to_owned()),
                role: "owner".to_owned(),
                current: true,
            })
            .unwrap(),
            serde_json::json!({
                "id_hex": format!("04{}", "cd".repeat(32)),
                "name": "Satoshi's MacBook Air",
                "role": "owner",
                "current": true,
            })
        );
    }

    #[test]
    fn signup_invites_are_serialized_but_redacted_from_debug_output() {
        let operation = Operation::CreateAccount {
            profile: "local".to_owned(),
            alias: "personal".to_owned(),
            username: "satoshi".to_owned(),
            device_name: "laptop".to_owned(),
            email: String::new(),
            invite: SecretString::new("s.secret-invite"),
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
    fn device_pairing_phrase_round_trips_without_entering_debug_output() {
        let operation = Operation::AcceptDevicePairing {
            profile: "local".to_owned(),
            target_alias: "laptop".to_owned(),
            device_name: "paired laptop".to_owned(),
            serial: 2,
            phrase: SecretString::new(
                "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount",
            ),
        };
        let encoded = encode(&Request::new(17, operation.clone())).unwrap();
        assert_eq!(decode_request(&encoded).unwrap().operation, operation);
        let debug = format!("{operation:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("cage"));
        assert!(operation.is_device_pairing_wait());
        let imported = Operation::AcceptGoProfilePairing {
            candidate_id: "ab".repeat(32),
            profile: "local".to_owned(),
            target_alias: "laptop".to_owned(),
            device_name: "paired laptop".to_owned(),
            serial: 3,
            phrase: SecretString::new(
                "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount",
            ),
        };
        let debug = format!("{imported:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("cage"));
        assert!(imported.is_device_pairing_wait());
        // Reading a Go CLI credential can raise a blocking Keychain prompt, so
        // the copy runs on the same long deadline as interactive pairing.
        assert!(Operation::CopyGoProfileDevice {
            candidate_id: "ab".repeat(32),
            profile: "local".to_owned(),
            target_alias: "laptop".to_owned(),
        }
        .is_device_pairing_wait());
        assert!(!Operation::StartDevicePairing {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
        }
        .is_device_pairing_wait());
    }

    #[test]
    fn owner_recovery_phrase_round_trips_without_entering_debug_output() {
        let operation = Operation::RecoverOwnerAccount {
            profile: "local".to_owned(),
            target_alias: "recovered".to_owned(),
            phrase: SecretString::new("one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen"),
            device_name: "recovery laptop".to_owned(),
            serial: 3,
        };
        assert_eq!(
            decode_request(&encode(&Request::new(18, operation.clone())).unwrap())
                .unwrap()
                .operation,
            operation
        );
        let debug = format!("{operation:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("seventeen"));
    }

    #[test]
    fn every_yubikey_operation_redacts_pin_puk_and_signup_secrets() {
        let operations = [
            Operation::CreateYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                username: "satoshi".to_owned(),
                device_name: "key".to_owned(),
                email: String::new(),
                invite: SecretString::new("s.yubi-invite"),
                passphrase: Some(SecretString::new("yubi-passphrase")),
                card_serial: 7,
                signing_slot: 0x82,
                pq_slot: 0x83,
                pin: SecretString::new("123456"),
                retry_configuration: Some(YubiRetryConfiguration {
                    puk: SecretString::new("retry-puk"),
                    pin_attempts: 5,
                    puk_attempts: 7,
                }),
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
                retry_configuration: None,
            },
            Operation::SyncYubiAccount {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("sync-pin"),
                with_federation: true,
            },
            Operation::SetYubiPassphrase {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("passphrase-pin"),
                passphrase: SecretString::new("set-yubi-passphrase"),
            },
            Operation::ChangeYubiPassphrase {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("change-passphrase-pin"),
                passphrase: SecretString::new("change-yubi-passphrase"),
            },
            Operation::VerifyYubiPassphrase {
                profile: "local".to_owned(),
                alias: "hardware".to_owned(),
                pin: SecretString::new("verify-passphrase-pin"),
                passphrase: SecretString::new("verify-yubi-passphrase"),
            },
            Operation::RefreshFederatedSecurity {
                profile: "local".to_owned(),
                team_alias: "engineering".to_owned(),
                local_yubi_alias: Some("hardware".to_owned()),
                local_pin: Some(SecretString::new("federation-local-pin")),
                remote_profile: Some("partner".to_owned()),
                remote_yubi_alias: Some("partner-hardware".to_owned()),
                remote_pin: Some(SecretString::new("federation-remote-pin")),
                unlocks: vec![YubiFederationUnlockInput {
                    profile: "third".to_owned(),
                    alias: "third-hardware".to_owned(),
                    pin: SecretString::new("federation-third-pin"),
                }],
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
                "retry-puk",
                "resume-pin",
                "provision-pin",
                "sync-pin",
                "passphrase-pin",
                "set-yubi-passphrase",
                "change-passphrase-pin",
                "change-yubi-passphrase",
                "verify-passphrase-pin",
                "verify-yubi-passphrase",
                "234567",
                "345678",
                "456789",
                "567890",
                "678901",
                "789012",
                "890123",
                "rotate-pin",
                "management-resume-pin",
                "federation-local-pin",
                "federation-remote-pin",
                "federation-third-pin",
            ] {
                assert!(!debug.contains(secret));
            }
        }
    }
}
