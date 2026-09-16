use crate::agent::AgentHandle;
use crate::commands::accounts::AccountDto;
use crate::commands::context::AppState;
use crate::commands::groups::{
    add_group_member_operation, admit_group_operation, create_group_operation,
    demote_group_member_operation, federation_dtos, group_detail_result, party_dtos,
    remove_group_member_operation, rerun_group_admission_operation, FederationEntryDto,
    FederationResponse, GroupDetailResultDto, GroupKindInput, MemberResponse, MemberRole, PartyDto,
    RoleInput,
};
use crate::commands::tests::support::{
    account_ref, phase_four_catalog, phase_four_state, team_ref,
};
use crate::commands::types::RoleDto;
use crate::commands::validation::{
    invalid_response, require_response_row_cap, MAXIMUM_FIRST_RUN_ROWS,
};
use crate::commands::vault::store_id;
use foks_agent_proto::{FederationRole, KvRole, Operation, ResponseResult, TeamKind, TeamRole};
use foks_desktop::{CatalogItem, CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[test]
fn catalog_activity_and_profile_health_gate_group_writes() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let team = foks_agent_proto::TeamStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "engineering".to_owned(),
        team_id: "03".to_owned(),
    };
    let id = store_id(&CatalogStoreRef::Team(team.clone()));
    *state.catalog.lock().unwrap() = Some(CatalogSnapshot {
        stores: vec![CatalogStoreSummary::Team {
            store: team.clone(),
            kind: "named".to_owned(),
            name: Some("Engineering".to_owned()),
            active: false,
            creation_phase: None,
        }],
        items: vec![CatalogItem {
            store: CatalogStoreRef::Team(team.clone()),
            metadata: foks_agent_proto::KvEntryMetadata {
                path: "/shared".to_owned(),
                node_type: "small-file".to_owned(),
                version: 4,
                size: Some(1),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
            },
        }],
        ..Default::default()
    });
    assert_eq!(
        state.selected_create_store(&id).unwrap_err().code,
        "inactive-group"
    );
    assert_eq!(
        state
            .selected_mutation_item(&id, "/shared", 4)
            .unwrap_err()
            .code,
        "inactive-group"
    );
    *state.catalog.lock().unwrap() = Some(CatalogSnapshot {
        stores: vec![CatalogStoreSummary::Team {
            store: team.clone(),
            kind: "named".to_owned(),
            name: Some("Engineering".to_owned()),
            active: true,
            creation_phase: None,
        }],
        items: vec![CatalogItem {
            store: CatalogStoreRef::Team(team.clone()),
            metadata: foks_agent_proto::KvEntryMetadata {
                path: "/shared".to_owned(),
                node_type: "small-file".to_owned(),
                version: 4,
                size: Some(1),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
            },
        }],
        ..Default::default()
    });
    assert_eq!(
        state.selected_create_store(&id).unwrap(),
        CatalogStoreRef::Team(team.clone())
    );
    assert_eq!(
        state
            .selected_mutation_item(&id, "/shared", 4)
            .unwrap()
            .store,
        CatalogStoreRef::Team(team)
    );
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .blocked_profiles
        .push("foks.example".to_owned());
    assert_eq!(
        state.selected_create_store(&id).unwrap_err().code,
        "capability-unavailable"
    );
    assert_eq!(
        state
            .selected_mutation_item(&id, "/shared", 4)
            .unwrap_err()
            .code,
        "capability-unavailable"
    );
}

#[test]
fn roster_and_federation_responses_fail_closed() {
    let party_id = "01".repeat(33);
    let remote_host_id = "02".repeat(33);
    let remote_team_id = "03".repeat(33);
    let operation_id = "07".repeat(16);
    let good = MemberResponse {
        username: Some("dana.okafor".to_owned()),
        party_id_hex: party_id.clone(),
        scoped_host_id_hex: None,
        party_kind: "user".to_owned(),
        source_role: MemberRole::Member { visibility: 0 },
        destination_role: MemberRole::Member { visibility: 0 },
        generation: 5,
        locally_manageable: true,
    };
    assert_eq!(party_dtos("team", vec![good]).unwrap().len(), 1);
    let malformed_user = MemberResponse {
        username: Some("dana.okafor".to_owned()),
        party_id_hex: "01DANA".to_owned(),
        scoped_host_id_hex: None,
        party_kind: "user".to_owned(),
        source_role: MemberRole::Member { visibility: 0 },
        destination_role: MemberRole::Member { visibility: 0 },
        generation: 5,
        locally_manageable: true,
    };
    assert_eq!(
        party_dtos("team", vec![malformed_user]).unwrap_err().code,
        "invalid-response"
    );
    for (party_kind, party_id_hex, username, scoped_host_id_hex) in [
        (
            "user",
            "04".repeat(33),
            Some("dana.okafor".to_owned()),
            None,
        ),
        ("named-team", "14".repeat(33), None, None),
        ("ad-hoc-team", "03".repeat(33), None, None),
        ("named-team", "03".repeat(33), None, Some("09".repeat(33))),
    ] {
        let wrong_entity_type = MemberResponse {
            username,
            party_id_hex,
            scoped_host_id_hex,
            party_kind: party_kind.to_owned(),
            source_role: MemberRole::Member { visibility: 0 },
            destination_role: MemberRole::Member { visibility: 0 },
            generation: 5,
            locally_manageable: false,
        };
        assert_eq!(
            party_dtos("team", vec![wrong_entity_type])
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }
    assert_eq!(
        party_dtos(
            "team",
            vec![MemberResponse {
                username: None,
                party_id_hex: "14".repeat(33),
                scoped_host_id_hex: Some("02".repeat(33)),
                party_kind: "ad-hoc-team".to_owned(),
                source_role: MemberRole::Member { visibility: 0 },
                destination_role: MemberRole::Member { visibility: 0 },
                generation: 5,
                locally_manageable: false,
            }]
        )
        .unwrap()
        .len(),
        1
    );
    let unsupported = MemberResponse {
        username: None,
        party_id_hex: "04".repeat(33),
        scoped_host_id_hex: None,
        party_kind: "device".to_owned(),
        source_role: MemberRole::Owner,
        destination_role: MemberRole::Owner,
        generation: 1,
        locally_manageable: false,
    };
    assert_eq!(
        party_dtos("team", vec![unsupported]).unwrap_err().code,
        "invalid-response"
    );
    let mislabeled_group = MemberResponse {
        username: Some("looks-like-a-person".to_owned()),
        party_id_hex: "03".repeat(33),
        scoped_host_id_hex: Some("09".repeat(33)),
        party_kind: "named-team".to_owned(),
        source_role: MemberRole::Owner,
        destination_role: MemberRole::Member { visibility: 0 },
        generation: 3,
        locally_manageable: false,
    };
    assert_eq!(
        party_dtos("team", vec![mislabeled_group]).unwrap_err().code,
        "invalid-response"
    );
    let unsupported_destination = FederationResponse {
        local_team_alias: "engineering".to_owned(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "homelab".to_owned(),
        remote_host_id_hex: remote_host_id.clone(),
        remote_team_id_hex: remote_team_id.clone(),
        destination: MemberRole::Admin,
        operation_id_hex: Some(operation_id.clone()),
        active: true,
    };
    assert_eq!(
        federation_dtos(
            "team",
            "work.example",
            "engineering",
            vec![unsupported_destination],
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    for (wrong_host, wrong_team) in [
        ("09".repeat(33), remote_team_id.clone()),
        (remote_host_id.clone(), "04".repeat(33)),
    ] {
        let wrong_entity_type = FederationResponse {
            local_team_alias: "engineering".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: wrong_host,
            remote_team_id_hex: wrong_team,
            destination: MemberRole::Member { visibility: 0 },
            operation_id_hex: None,
            active: true,
        };
        assert_eq!(
            federation_dtos(
                "team",
                "work.example",
                "engineering",
                vec![wrong_entity_type],
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
    }
    let uppercase_operation_id = FederationResponse {
        local_team_alias: "engineering".to_owned(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "homelab".to_owned(),
        remote_host_id_hex: remote_host_id.clone(),
        remote_team_id_hex: remote_team_id.clone(),
        destination: MemberRole::Member { visibility: 0 },
        operation_id_hex: Some("AA".repeat(16)),
        active: false,
    };
    assert_eq!(
        federation_dtos(
            "team",
            "work.example",
            "engineering",
            vec![uppercase_operation_id],
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    let wrong_local_team = FederationResponse {
        local_team_alias: "some-other-group".to_owned(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "homelab".to_owned(),
        remote_host_id_hex: remote_host_id.clone(),
        remote_team_id_hex: remote_team_id.clone(),
        destination: MemberRole::Member { visibility: 0 },
        operation_id_hex: Some(operation_id),
        active: true,
    };
    assert_eq!(
        federation_dtos(
            "team",
            "work.example",
            "engineering",
            vec![wrong_local_team],
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
    let empty_optional_id = FederationResponse {
        local_team_alias: "engineering".to_owned(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "homelab".to_owned(),
        remote_host_id_hex: remote_host_id.clone(),
        remote_team_id_hex: remote_team_id.clone(),
        destination: MemberRole::Member { visibility: 0 },
        operation_id_hex: Some(String::new()),
        active: false,
    };
    assert_eq!(
        federation_dtos(
            "team",
            "work.example",
            "engineering",
            vec![empty_optional_id],
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let good_federation = FederationResponse {
        local_team_alias: "engineering".to_owned(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "homelab".to_owned(),
        remote_host_id_hex: remote_host_id,
        remote_team_id_hex: remote_team_id,
        destination: MemberRole::Member { visibility: 0 },
        operation_id_hex: Some("0a".repeat(16)),
        active: true,
    };
    assert_eq!(
        federation_dtos("team", "work.example", "engineering", vec![good_federation])
            .unwrap()
            .len(),
        1
    );
    let good_ad_hoc_federation = FederationResponse {
        local_team_alias: "engineering".to_owned(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "friends".to_owned(),
        remote_host_id_hex: "02".repeat(33),
        remote_team_id_hex: "14".repeat(33),
        destination: MemberRole::Member { visibility: -2 },
        operation_id_hex: Some("0b".repeat(16)),
        active: true,
    };
    assert_eq!(
        federation_dtos(
            "team",
            "work.example",
            "engineering",
            vec![good_ad_hoc_federation],
        )
        .unwrap()
        .len(),
        1
    );

    assert!(serde_json::from_value::<MemberResponse>(serde_json::json!({
        "username":"dana.okafor",
        "party_id_hex":party_id,
        "scoped_host_id_hex":null,
        "party_kind":"user",
        "source_role":{"member":{"visibility":0}},
        "destination_role":{"member":{"visibility":0}},
        "generation":5,
        "locally_manageable":true,
        "extra":true
    }))
    .is_err());
    assert!(serde_json::from_value::<MemberResponse>(serde_json::json!({
        "username":"dana.okafor",
        "party_id_hex":"01".repeat(33),
        "scoped_host_id_hex":null,
        "party_kind":"user",
        "source_role":{"member":{"visibility":0,"invented":true}},
        "destination_role":{"member":{"visibility":0}},
        "generation":5,
        "locally_manageable":true
    }))
    .is_err());
    assert!(
        serde_json::from_value::<FederationResponse>(serde_json::json!({
            "local_team_alias":"engineering",
            "remote_profile":"home.example",
            "remote_team_alias":"homelab",
            "remote_host_id_hex":"02".repeat(33),
            "remote_team_id_hex":"03".repeat(33),
            "destination":{"member":{"visibility":0}},
            "operation_id_hex":"07".repeat(16),
            "active":true,
            "extra":true
        }))
        .is_err()
    );

    assert_eq!(
        require_response_row_cap(
            &serde_json::Value::Array(
                (0..=MAXIMUM_FIRST_RUN_ROWS)
                    .map(|_| serde_json::Value::Null)
                    .collect()
            ),
            "entries"
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
}

#[test]
fn combined_group_detail_results_preserve_independent_errors() {
    assert_eq!(
        group_detail_result(
            ResponseResult::Success {
                value: serde_json::json!(7)
            },
            |value| {
                serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))
            }
        )
        .unwrap(),
        GroupDetailResultDto::Success { value: 7_i64 }
    );
    let mapped = group_detail_result::<i64>(
        ResponseResult::Error {
            code: foks_agent_proto::ErrorCode::RateLimited,
            message: "wait".to_owned(),
            fields: foks_agent_proto::ErrorFields {
                reason: Some("status 1012".to_owned()),
                ..foks_agent_proto::ErrorFields::default()
            },
        },
        |_| panic!("error result decoded a value"),
    )
    .unwrap();
    let GroupDetailResultDto::Error { error } = mapped else {
        panic!("nested error became success");
    };
    assert_eq!(error.code, "rate-limited");
    assert!(error.retryable);
    assert_eq!(
        error.details.unwrap().reason.as_deref(),
        Some("status 1012")
    );
}

#[test]
fn strict_demotion_orders_member_visibility_below_admin_and_owner() {
    assert!(MemberRole::Member { visibility: -1 }
        .is_strictly_lower_than(MemberRole::Member { visibility: 0 }));
    assert!(!MemberRole::Member { visibility: 0 }
        .is_strictly_lower_than(MemberRole::Member { visibility: 0 }));
    assert!(!MemberRole::Member { visibility: 1 }
        .is_strictly_lower_than(MemberRole::Member { visibility: 0 }));
    assert!(MemberRole::Member {
        visibility: i16::MAX
    }
    .is_strictly_lower_than(MemberRole::Admin));
    assert!(MemberRole::Admin.is_strictly_lower_than(MemberRole::Owner));
    assert!(!MemberRole::Owner.is_strictly_lower_than(MemberRole::Admin));
    assert_eq!(
        serde_json::from_value::<RoleInput>(serde_json::json!({
            "role": "Member",
            "visibility": -16384
        }))
        .unwrap(),
        RoleInput::Member { visibility: -16384 }
    );
    assert!(serde_json::from_value::<RoleInput>(serde_json::json!({
        "role": "Admin",
        "visibility": 0
    }))
    .is_err());
    assert_eq!(
        serde_json::from_value::<GroupKindInput>(serde_json::json!("adhoc")).unwrap(),
        GroupKindInput::Adhoc
    );
}

#[test]
fn group_operation_transcripts_use_only_resolved_identities() {
    let account = account_ref("work.example", "personal");
    assert_eq!(
        create_group_operation(
            account.clone(),
            "design-systems",
            "Design Systems",
            GroupKindInput::Named,
        )
        .unwrap(),
        Operation::CreateTeam {
            profile: "work.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "design-systems".to_owned(),
            name: "Design Systems".to_owned(),
            kind: TeamKind::Named,
        }
    );
    assert_eq!(
        create_group_operation(
            account,
            "weekend-project",
            "renderer name is not sent",
            GroupKindInput::Adhoc,
        )
        .unwrap(),
        Operation::CreateTeam {
            profile: "work.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "weekend-project".to_owned(),
            name: String::new(),
            kind: TeamKind::AdHoc,
        }
    );

    let team = team_ref("work.example", "personal", "engineering");
    assert!(matches!(
        add_group_member_operation(team.clone(), "jules.park", RoleInput::Owner).unwrap(),
        Operation::AddTeamMember {
            profile,
            team_alias,
            username,
            role: TeamRole::Owner,
            visibility: 0,
        } if profile == "work.example" && team_alias == "engineering" && username == "jules.park"
    ));
    assert!(matches!(
        demote_group_member_operation(
            team.clone(),
            "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            MemberRole::Owner,
            RoleInput::Member { visibility: -2 },
        )
        .unwrap(),
        Operation::DemoteTeamMember {
            party_id_hex,
            role: TeamRole::Member,
            visibility: -2,
            ..
        } if party_id_hex == "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ));
    assert_eq!(
        demote_group_member_operation(
            team.clone(),
            "dana.okafor",
            MemberRole::Member { visibility: 0 },
            RoleInput::Admin,
        )
        .unwrap_err()
        .code,
        "not-a-demotion"
    );
    assert!(matches!(
        remove_group_member_operation(
            team.clone(),
            "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap(),
        Operation::RemoveTeamMember { party_id_hex, .. }
            if party_id_hex == "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ));
    assert_eq!(
        remove_group_member_operation(team.clone(), "sam.ortiz")
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert!(matches!(
        admit_group_operation(
            team,
            team_ref("home.example", "home", "homelab"),
            -3,
        ),
        Operation::AdmitFederatedTeam {
            local_profile,
            remote_profile,
            role: FederationRole::Member,
            visibility: -3,
            ..
        } if local_profile == "work.example" && remote_profile == "home.example"
    ));
}

#[test]
fn member_targets_require_fresh_local_non_self_user_facts() {
    let state = phase_four_state(Vec::new());
    let local = team_ref("work.example", "personal", "engineering");
    let local_id = store_id(&CatalogStoreRef::Team(local));
    let account = account_ref("work.example", "personal");
    let account_id = store_id(&CatalogStoreRef::Account(account));
    state.accounts.lock().unwrap().insert(
        account_id.clone(),
        AccountDto {
            local_alias: None,
            store: account_id,
            profile: "work.example".to_owned(),
            alias: "personal".to_owned(),
            username: "vitalik".to_owned(),
        },
    );
    let party = |username: &str, manageable: bool| PartyDto {
        store: local_id.clone(),
        username: Some(username.to_owned()),
        party_kind: "user".to_owned(),
        generation: 4,
        locally_manageable: manageable,
        party_id_hex: format!("01{username}"),
        scoped_host_id_hex: None,
        source_role: KvRole::Member { visibility: 0 }.into(),
        destination_role: KvRole::Member { visibility: 0 }.into(),
    };
    state.rosters.lock().unwrap().insert(
        local_id.clone(),
        vec![
            party("vitalik", true),
            party("dana.okafor", true),
            party("deploy-bot", false),
        ],
    );
    assert_eq!(
        state
            .selected_member_target(&local_id, "dana.okafor")
            .unwrap()
            .2,
        MemberRole::Member { visibility: 0 }
    );
    for username in ["vitalik", "deploy-bot", "missing"] {
        assert_eq!(
            state
                .selected_member_target(&local_id, username)
                .unwrap_err()
                .code,
            "member-not-actionable"
        );
    }
    state.rosters.lock().unwrap().clear();
    assert_eq!(
        state
            .selected_member_target(&local_id, "dana.okafor")
            .unwrap_err()
            .code,
        "roster-required"
    );
}

#[test]
fn blocked_profiles_stop_group_reads_and_mutations_before_transport() {
    let state = phase_four_state(vec!["work.example".to_owned()]);
    let local_id = store_id(&CatalogStoreRef::Team(team_ref(
        "work.example",
        "personal",
        "engineering",
    )));
    assert_eq!(
        state.selected_team(&local_id).unwrap_err().code,
        "capability-unavailable"
    );
    assert_eq!(
        state
            .selected_active_team_for_mutation(&local_id)
            .unwrap_err()
            .code,
        "capability-unavailable"
    );
    let account_id = store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    assert_eq!(
        state.selected_account(&account_id).unwrap_err().code,
        "capability-unavailable"
    );

    let state = phase_four_state(Vec::new());
    let household = team_ref("work.example", "personal", "household");
    let household_id = store_id(&CatalogStoreRef::Team(household.clone()));
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .stores
        .push(CatalogStoreSummary::Team {
            store: household,
            kind: "ad-hoc".to_owned(),
            name: None,
            active: true,
            creation_phase: None,
        });
    assert_eq!(
        state
            .selected_active_team_for_mutation(&household_id)
            .unwrap_err()
            .code,
        "group-management-unavailable"
    );
}

#[test]
fn admission_rerun_requires_one_retained_inactive_operation_and_fresh_remote() {
    let state = phase_four_state(Vec::new());
    let local_id = store_id(&CatalogStoreRef::Team(team_ref(
        "work.example",
        "personal",
        "engineering",
    )));
    let operation_id = "07".repeat(16);
    let entry = FederationEntryDto {
        store: local_id.clone(),
        remote_profile: "home.example".to_owned(),
        remote_team_alias: "homelab".to_owned(),
        remote_host_id_hex: "02".repeat(33),
        remote_team_id_hex: "03".repeat(33),
        destination: KvRole::Member { visibility: -2 }.into(),
        operation_id_hex: Some(operation_id.clone()),
        active: false,
    };
    state
        .federations
        .lock()
        .unwrap()
        .insert(local_id.clone(), vec![entry.clone()]);
    let (local, retained) = state
        .selected_inactive_admission(&local_id, &operation_id)
        .unwrap();
    assert!(matches!(
        rerun_group_admission_operation(local, retained).unwrap(),
        Operation::AdmitFederatedTeam {
            remote_profile,
            remote_team_alias,
            visibility: -2,
            ..
        } if remote_profile == "home.example" && remote_team_alias == "homelab"
    ));
    assert_eq!(
        state
            .selected_inactive_admission(&local_id, "renderer-chosen")
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let mut no_operation = entry.clone();
    no_operation.operation_id_hex = None;
    state
        .federations
        .lock()
        .unwrap()
        .insert(local_id.clone(), vec![no_operation]);
    assert_eq!(
        state
            .selected_inactive_admission(&local_id, &operation_id)
            .unwrap_err()
            .code,
        "admission-not-resumable"
    );
    state
        .federations
        .lock()
        .unwrap()
        .insert(local_id.clone(), vec![entry.clone()]);

    *state.catalog.lock().unwrap() = Some(phase_four_catalog(vec!["home.example".to_owned()]));
    assert_eq!(
        state
            .selected_inactive_admission(&local_id, &operation_id)
            .unwrap_err()
            .code,
        "capability-unavailable"
    );

    *state.catalog.lock().unwrap() = Some(phase_four_catalog(Vec::new()));
    let mut duplicate = entry;
    duplicate.remote_team_alias = "other".to_owned();
    state
        .federations
        .lock()
        .unwrap()
        .insert(local_id.clone(), vec![duplicate.clone(), duplicate]);
    assert_eq!(
        state
            .selected_inactive_admission(&local_id, &operation_id)
            .unwrap_err()
            .code,
        "admission-not-resumable"
    );
}

#[test]
fn federation_expulsion_requires_exact_active_cached_federation_and_roster_rows() {
    let state = phase_four_state(Vec::new());
    let local_id = store_id(&CatalogStoreRef::Team(team_ref(
        "work.example",
        "personal",
        "engineering",
    )));
    let host = "02".repeat(33);
    let team = "03".repeat(33);
    let destination: RoleDto = KvRole::Member { visibility: -2 }.into();
    state.federations.lock().unwrap().insert(
        local_id.clone(),
        vec![FederationEntryDto {
            store: local_id.clone(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: host.clone(),
            remote_team_id_hex: team.clone(),
            destination: destination.clone(),
            operation_id_hex: Some("07".repeat(16)),
            active: true,
        }],
    );
    state.rosters.lock().unwrap().insert(
        local_id.clone(),
        vec![PartyDto {
            store: local_id.clone(),
            username: None,
            party_kind: "named-team".to_owned(),
            generation: 1,
            locally_manageable: false,
            party_id_hex: team.clone(),
            scoped_host_id_hex: Some(host.clone()),
            source_role: KvRole::Admin.into(),
            destination_role: destination,
        }],
    );
    let (local, target) = state
        .selected_active_federation_target(&local_id, &host, &team)
        .unwrap();
    assert_eq!(local.profile, "work.example");
    assert_eq!(target.remote_team_id_hex, team);
    assert_eq!(
        state
            .selected_active_federation_target(&local_id, &"02".repeat(32), &team)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    state.rosters.lock().unwrap().clear();
    assert_eq!(
        state
            .selected_active_federation_target(&local_id, &host, &team)
            .unwrap_err()
            .code,
        "roster-required"
    );
}

#[test]
fn stale_group_reads_cannot_repopulate_authorization_caches() {
    let state = phase_four_state(Vec::new());
    state.catalog_generation.store(2, Ordering::Release);
    let error = state
        .retain_roster(1, "old-store".to_owned(), &[])
        .unwrap_err();
    assert_eq!(error.code, "catalog-required");
    assert!(state.rosters.lock().unwrap().is_empty());

    state.catalog_generation.store(2, Ordering::Release);
    state.accounts.lock().unwrap().insert(
        "old-account".to_owned(),
        AccountDto {
            local_alias: None,
            store: "old-account".to_owned(),
            profile: "work.example".to_owned(),
            alias: "personal".to_owned(),
            username: "vitalik".to_owned(),
        },
    );
    state
        .federations
        .lock()
        .unwrap()
        .insert("old-store".to_owned(), Vec::new());
    state.invalidate_catalog();
    assert!(state.accounts.lock().unwrap().is_empty());
    assert!(state.federations.lock().unwrap().is_empty());
}
