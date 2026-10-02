use super::*;
use foks_agent_proto::{chat::ChatAction, AccountStoreRef, TeamRole, TeamStoreRef};

fn chat(action: ChatAction) -> Operation {
    Operation::Chat {
        store: TeamStoreRef {
            profile: "local".into(),
            account_alias: "owner".into(),
            team_alias: "team".into(),
            team_id: "03".repeat(33),
        },
        action,
    }
}

#[test]
fn namespace_and_membership_writes_keep_exclusive_nonabandonable_admission() {
    let cases = [
        (
            Operation::MoveKv {
                dirent_id: [7; 16],
                store: KvStoreRef::Account(AccountStoreRef {
                    profile: "local".into(),
                    account_alias: "owner".into(),
                }),
                path: "/old".into(),
                destination: "/new".into(),
                version: 7,
            },
            Scope::profile("local"),
        ),
        (
            Operation::PromoteTeamMember {
                profile: "local".into(),
                team_alias: "team".into(),
                party_id_hex: "04".repeat(33),
                role: TeamRole::Admin,
                visibility: 0,
            },
            Scope::SecurityRoot,
        ),
        (
            Operation::DemoteTeamMember {
                profile: "local".into(),
                team_alias: "team".into(),
                party_id_hex: "04".repeat(33),
                role: TeamRole::Member,
                visibility: 0,
            },
            Scope::SecurityRoot,
        ),
        (
            Operation::RemoveProfile {
                name: "local".into(),
            },
            Scope::Root,
        ),
    ];
    for (operation, expected) in cases {
        assert_eq!(
            operation_scope(&operation),
            expected,
            "{}",
            operation.name()
        );
        assert_eq!(
            admission_scope(&operation),
            expected,
            "{}",
            operation.name()
        );
        assert!(operation.is_mutation(), "{}", operation.name());
        assert!(
            !crate::read_cache::operation_serves_reads(&operation),
            "{}",
            operation.name()
        );
        assert!(
            !crate::read_cache::operation_leaves_retained_material(&operation),
            "{}",
            operation.name()
        );
        assert!(
            !crate::read_cache::operation_is_abandonable(&operation),
            "{}",
            operation.name()
        );
    }
}

#[test]
fn cached_reads_do_not_implicitly_authorize_shared_profile_locks() {
    for operation in [
        Operation::DiscoverTeams {
            profile: "local".into(),
            account_alias: "owner".into(),
        },
        chat(ChatAction::Channels),
        chat(ChatAction::Inbox),
    ] {
        assert!(
            crate::read_cache::operation_serves_reads(&operation),
            "{}",
            operation.name()
        );
        assert_eq!(
            admission_scope(&operation),
            Scope::profile("local"),
            "{}",
            operation.name()
        );
    }
    let roster = Operation::ListTeamMembers {
        profile: "local".into(),
        team_alias: "team".into(),
    };
    assert!(crate::read_cache::operation_serves_reads(&roster));
    assert_eq!(
        admission_scope(&roster),
        Scope::SharedProfile("local".into())
    );
}

#[test]
fn retaining_cached_material_does_not_authorize_abandoning_intent_writes() {
    let write = chat(ChatAction::ClearIntent {
        host: "02".repeat(33),
        actor: "03".repeat(33),
        channel: "04".repeat(16),
        submission: "05".repeat(16),
    });
    let read = chat(ChatAction::Pending);
    for operation in [&read, &write] {
        assert!(crate::read_cache::operation_leaves_retained_material(
            operation
        ));
    }
    assert!(write.is_mutation());
    assert!(!crate::read_cache::operation_is_abandonable(&write));
    assert_eq!(admission_scope(&write), Scope::profile("local"));
    assert!(!read.is_mutation());
    assert!(crate::read_cache::operation_is_abandonable(&read));
    assert_eq!(admission_scope(&read), Scope::SharedProfile("local".into()));
}

#[test]
fn cross_profile_admission_is_canonical_and_never_shared() {
    for (local, remote, expected) in [
        ("z", "a", vec!["a".to_owned(), "z".to_owned()]),
        ("a", "a", vec!["a".to_owned()]),
    ] {
        let operation = Operation::AddFederatedTeamMember {
            local_profile: local.into(),
            local_team_alias: "destination".into(),
            remote_profile: remote.into(),
            remote_team_alias: "source".into(),
            role: foks_agent_proto::FederationRole::Member,
            visibility: 0,
        };
        assert_eq!(admission_scope(&operation), Scope::Profiles(expected));
        assert!(!crate::read_cache::operation_is_abandonable(&operation));
    }
}
