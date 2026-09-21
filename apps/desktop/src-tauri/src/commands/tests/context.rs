use crate::agent::AgentHandle;
use crate::commands::accounts::{AccountDto, DeviceDto};
use crate::commands::context::AppState;
use crate::commands::tests::servers::ProfileListTransport;
use crate::commands::tests::support::{account_ref, phase_four_state, team_ref};
use crate::commands::vault::store_id;
use foks_desktop::{CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn chat_catalog() -> CatalogSnapshot {
    let team = team_ref("chat", "owner", "team");
    let store = CatalogStoreRef::Team(team.clone());
    let mut catalog = complete_profile_catalog("chat");
    catalog.stores.push(CatalogStoreSummary::Team {
        store: team,
        kind: "named".into(),
        name: Some("Team".into()),
        active: true,
        creation_phase: None,
        chain_seqno: None,
    });
    catalog.known_stores = catalog.stores.clone();
    catalog.store_reads.push(foks_desktop::CatalogStoreRead {
        store: store.clone(),
        state: foks_desktop::CatalogStoreReadState::Complete,
    });
    catalog.items.push(foks_desktop::CatalogItem {
        store,
        metadata: foks_agent_proto::KvEntryMetadata {
            path: "/item".into(),
            node_type: "small-file".into(),
            version: 1,
            size: Some(1),
            read_role: foks_agent_proto::KvRole::Owner,
            write_role: foks_agent_proto::KvRole::Owner,
        },
    });
    let success = |value| foks_agent_proto::ResponseResult::Success { value };
    catalog.profile_overviews.push(foks_agent_proto::ProfileOverview {
        profile: "chat".into(),
        accounts: success(serde_json::json!([{"profile":"chat", "alias":"owner", "username":"alice"}])),
        teams: success(serde_json::json!([])),
        server_status: success(serde_json::json!({"profile":"chat", "configured_probe":"chat.example", "host":null, "chat_supported":true, "compatibility":{"status":"not-required"}})),
    });
    catalog
}

#[test]
fn stale_item_versions_do_not_report_agent_protocol_mismatch() {
    let state = phase_four_state(vec![]);
    let catalog = chat_catalog();
    let item = catalog.items[0].clone();
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog, |_| {}));
    let store = store_id(&item.store);
    let error = state
        .selected_item(&store, &item.metadata.path, item.metadata.version + 1)
        .unwrap_err();
    assert_eq!(error.code, "item-version-mismatch");
    assert!(!error.fatal);
    assert_eq!(
        state
            .selected_mutation_item(&store, &item.metadata.path, item.metadata.version + 1)
            .unwrap_err()
            .code,
        "conflict"
    );
}

#[test]
fn kv_refresh_and_content_invalidation_preserve_chat_access_revision() {
    let state = phase_four_state(vec![]);
    let chat = state.for_profile("chat").unwrap();
    let mut catalog = chat_catalog();
    let id = store_id(&catalog.stores[0].store_ref());
    let written = catalog.items[0].store.clone();
    let (load, _) = chat.begin_catalog_load_checked().unwrap();
    assert!(chat.publish_catalog(load, catalog.clone(), |_| {}));
    let (revision, _) = chat.selected_chat(&id).unwrap();
    let item_generation = chat.catalog_at(None).unwrap().0;
    catalog.items[0].metadata.version += 1;
    let (load, _) = chat.begin_catalog_load_checked().unwrap();
    assert!(chat.publish_catalog(load, catalog, |_| {}));
    assert_eq!(chat.selected_chat(&id).unwrap().0, revision);
    assert_ne!(chat.catalog_at(None).unwrap().0, item_generation);
    chat.invalidate_catalog_items(&written);
    assert_eq!(chat.selected_chat(&id).unwrap().0, revision);
    chat.invalidate_catalog();
    assert_ne!(chat.chat_generation.load(Ordering::Acquire), revision);
    assert!(chat.selected_chat(&id).is_err());
}

#[test]
fn lease_renewal_and_kv_capability_changes_do_not_retire_chat() {
    let state = phase_four_state(vec![]).for_profile("chat").unwrap();
    let mut catalog = chat_catalog();
    if let foks_agent_proto::ResponseResult::Success { value } =
        &mut catalog.profile_overviews[0].server_status
    {
        value["compatibility"] = serde_json::json!({"status":"validated", "expires_at":9_007_199_254_740_990u64, "capabilities":["chat", "kv", "device-administration"]});
    }
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog.clone(), |_| {}));
    let revision = state.chat_generation.load(Ordering::Acquire);
    if let foks_agent_proto::ResponseResult::Success { value } =
        &mut catalog.profile_overviews[0].server_status
    {
        value["compatibility"] = serde_json::json!({"status":"validated", "expires_at":9_007_199_254_740_991u64, "capabilities":["chat"]});
    }
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog.clone(), |_| {}));
    assert_eq!(state.chat_generation.load(Ordering::Acquire), revision);
    if let foks_agent_proto::ResponseResult::Success { value } =
        &mut catalog.profile_overviews[0].server_status
    {
        value["compatibility"]["expires_at"] = serde_json::json!(1);
    }
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog, |_| {}));
    assert_ne!(state.chat_generation.load(Ordering::Acquire), revision);
}

#[test]
fn partial_publication_keeps_accepted_facts_and_does_not_clear_mutation_gate() {
    let state = phase_four_state(vec![]);
    let chat = state.for_profile("chat").unwrap();
    let catalog = chat_catalog();
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog.clone(), |_| {}));
    let revision = chat.chat_generation.load(Ordering::Acquire);
    chat.mutation_requires_refresh
        .store(true, Ordering::Release);
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    let partial = CatalogSnapshot {
        profiles: vec!["chat".into()],
        ..Default::default()
    };
    assert!(
        state.publish_catalog_snapshot(load, partial, |_, accepted| {
            assert_eq!(accepted.items, catalog.items);
            assert_eq!(accepted.stores, catalog.stores);
            assert_eq!(accepted.full_item_reads, None);
            assert_eq!(
                accepted.store_reads[0].state,
                foks_desktop::CatalogStoreReadState::NotLoaded
            );
        })
    );
    let mut pending_items = catalog.clone();
    pending_items.items.clear();
    pending_items.full_item_reads = None;
    pending_items.store_reads[0].state = foks_desktop::CatalogStoreReadState::NotLoaded;
    assert!(
        state.publish_catalog_snapshot(load, pending_items.clone(), |_, accepted| {
            assert_eq!(accepted.items, catalog.items);
            assert_eq!(accepted.full_item_reads, None);
            assert_eq!(
                accepted.store_reads[0].state,
                foks_desktop::CatalogStoreReadState::NotLoaded
            );
        })
    );
    assert!(chat.mutation_requires_refresh.load(Ordering::Acquire));
    assert_eq!(chat.chat_generation.load(Ordering::Acquire), revision);
    pending_items.store_reads[0].state = foks_desktop::CatalogStoreReadState::Failed;
    pending_items.failures.push(foks_desktop::CatalogFailure {
        scope: foks_desktop::CatalogFailureScope::Store(catalog.stores[0].store_ref()),
        error: foks_desktop::AgentError::Transport("unavailable".into()),
    });
    assert!(
        state.publish_catalog_snapshot(load, pending_items, |_, accepted| {
            assert!(accepted.items.is_empty());
            assert_eq!(
                accepted.store_reads[0].state,
                foks_desktop::CatalogStoreReadState::Failed
            );
        })
    );
    assert_eq!(chat.chat_generation.load(Ordering::Acquire), revision);
    assert!(chat.mutation_requires_refresh.load(Ordering::Acquire));
}

#[test]
fn catalog_access_and_verified_actor_changes_retire_reused_aliases() {
    let state = phase_four_state(vec![]);
    let chat = state.for_profile("chat").unwrap();
    let catalog = chat_catalog();
    let id = store_id(&catalog.stores[0].store_ref());
    let (load, _) = chat.begin_catalog_load_checked().unwrap();
    assert!(chat.publish_catalog(load, catalog.clone(), |_| {}));
    let (revision, team) = chat.selected_chat(&id).unwrap();
    let scope = foks_agent_proto::chat::ChatScope {
        store: team,
        host: "02".to_owned() + &"ab".repeat(32),
        actor: "01".to_owned() + &"ab".repeat(32),
    };
    chat.accept_chat_scope(&id, revision, &scope).unwrap();
    let mut replacement = scope.clone();
    replacement.actor = "01".to_owned() + &"cd".repeat(32);
    assert_eq!(
        chat.accept_chat_scope(&id, revision, &replacement)
            .unwrap_err()
            .code,
        "chat-restart"
    );
    assert!(chat.accept_chat_scope(&id, revision, &scope).is_err());
    let revision = chat.chat_generation.load(Ordering::Acquire);
    chat.accept_chat_scope(&id, revision, &replacement).unwrap();
    let mut changed = catalog.clone();
    changed.profile_overviews[0].accounts = foks_agent_proto::ResponseResult::Success {
        value: serde_json::json!([{"profile":"chat", "alias":"owner", "username":"replacement"}]),
    };
    let (load, _) = chat.begin_catalog_load_checked().unwrap();
    assert!(chat.publish_catalog(load, changed, |_| {}));
    assert_ne!(chat.chat_generation.load(Ordering::Acquire), revision);
    let revision = chat.chat_generation.load(Ordering::Acquire);
    let mut blocked = catalog;
    blocked.blocked_profiles.push("chat".into());
    let (load, _) = chat.begin_catalog_load_checked().unwrap();
    assert!(chat.publish_catalog(load, blocked, |_| {}));
    assert_ne!(chat.chat_generation.load(Ordering::Acquire), revision);
    assert!(chat.selected_chat(&id).is_err());
}

#[test]
fn root_and_profile_loads_retire_each_other_without_retiring_siblings() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("a").unwrap();
    let b = state.for_profile("b").unwrap();
    let (root_load, root_token) = state.begin_catalog_load_checked().unwrap();
    let (a_load, a_token) = a.begin_catalog_load_checked().unwrap();
    assert!(root_token.is_cancelled());
    assert!(
        !state.publish_catalog(root_load, CatalogSnapshot::default(), |_| panic!(
            "retired root"
        ))
    );
    let (b_load, _) = b.begin_catalog_load_checked().unwrap();
    assert!(!a_token.is_cancelled());
    assert!(
        a.publish_catalog_snapshot(a_load, complete_profile_catalog("a"), |_, accepted| {
            assert_eq!(accepted.profiles, ["a"]);
        })
    );
    assert!(
        b.publish_catalog_snapshot(b_load, complete_profile_catalog("b"), |_, accepted| {
            assert_eq!(accepted.profiles, ["b"]);
        })
    );
    let (a_load, a_token) = a.begin_catalog_load_checked().unwrap();
    let (root_load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(a_token.is_cancelled());
    assert!(
        !a.publish_catalog(a_load, complete_profile_catalog("a"), |_| panic!(
            "retired profile"
        ))
    );
    assert!(state.publish_catalog(root_load, complete_profile_catalog("root"), |_| {}));
    let (a_load, _) = a.begin_catalog_load_checked().unwrap();
    state.invalidate_catalog();
    assert!(
        !a.publish_catalog(a_load, complete_profile_catalog("a"), |_| panic!(
            "retired by root change"
        ))
    );
}

#[test]
fn denied_expired_and_failed_profile_facts_do_not_authorize_chat() {
    for status in [
        serde_json::json!({"status":"missing"}),
        serde_json::json!({"status":"validated", "expires_at":1, "capabilities":["chat"]}),
        serde_json::json!({"status":"validated", "expires_at":9_007_199_254_740_991u64, "capabilities":["kv"]}),
    ] {
        let state = phase_four_state(vec![]).for_profile("chat").unwrap();
        let mut catalog = chat_catalog();
        let id = store_id(&catalog.stores[0].store_ref());
        if let foks_agent_proto::ResponseResult::Success { value } =
            &mut catalog.profile_overviews[0].server_status
        {
            value["compatibility"] = status;
        }
        let (load, _) = state.begin_catalog_load_checked().unwrap();
        assert!(state.publish_catalog(load, catalog, |_| {}));
        assert!(state.selected_chat(&id).is_err());
    }
    let state = phase_four_state(vec![]).for_profile("chat").unwrap();
    let catalog = chat_catalog();
    let id = store_id(&catalog.stores[0].store_ref());
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog.clone(), |_| {}));
    let mut failed = CatalogSnapshot {
        profiles: vec!["chat".into()],
        known_stores: catalog.known_stores,
        failures: vec![foks_desktop::CatalogFailure {
            scope: foks_desktop::CatalogFailureScope::Profile {
                profile: "chat".into(),
                source: "profile overview".into(),
            },
            error: foks_desktop::AgentError::Transport("unavailable".into()),
        }],
        ..Default::default()
    };
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, failed.clone(), |_| {}));
    assert!(state.selected_chat(&id).is_err());
    failed.failures.clear();
    assert!(state.publish_catalog(load, failed, |_| {}));
    assert!(state.selected_chat(&id).is_err());
}

#[test]
fn successful_empty_store_read_replaces_previous_items() {
    let state = phase_four_state(vec![]).for_profile("chat").unwrap();
    let mut catalog = chat_catalog();
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog.clone(), |_| {}));
    catalog.items.clear();
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(
        state.publish_catalog_snapshot(load, catalog, |_, accepted| {
            assert!(accepted.items.is_empty());
            assert_eq!(
                accepted.store_reads[0].state,
                foks_desktop::CatalogStoreReadState::Complete
            );
        })
    );
}

/// A write lands in one store. The other stores' items are still what their
/// own reads published, so only the written store's items and read state are
/// discarded; the profile's full-item claim goes with them, because the
/// profile as a whole is no longer read whole.
#[test]
fn a_write_discards_only_the_items_of_the_store_it_landed_in() {
    let state = phase_four_state(vec![]).for_profile("chat").unwrap();
    let mut catalog = chat_catalog();
    let written = catalog.items[0].store.clone();
    let untouched = CatalogStoreRef::Account(account_ref("chat", "owner"));
    catalog.items.push(foks_desktop::CatalogItem {
        store: untouched.clone(),
        metadata: foks_agent_proto::KvEntryMetadata {
            path: "/other".into(),
            node_type: "small-file".into(),
            version: 3,
            size: Some(2),
            read_role: foks_agent_proto::KvRole::Owner,
            write_role: foks_agent_proto::KvRole::Owner,
        },
    });
    catalog.store_reads.push(foks_desktop::CatalogStoreRead {
        store: untouched.clone(),
        state: foks_desktop::CatalogStoreReadState::Complete,
    });
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog, |_| {}));

    state.invalidate_catalog_items(&written);

    let catalog = state.catalog.lock().unwrap();
    let catalog = catalog.as_ref().unwrap();
    assert_eq!(
        catalog
            .items
            .iter()
            .map(|item| item.store.clone())
            .collect::<Vec<_>>(),
        vec![untouched.clone()],
    );
    for read in &catalog.store_reads {
        assert_eq!(
            read.state,
            if read.store == written {
                foks_desktop::CatalogStoreReadState::NotLoaded
            } else {
                foks_desktop::CatalogStoreReadState::Complete
            },
            "{:?}",
            read.store
        );
    }
    assert_eq!(catalog.full_item_reads, Some(vec![]));
}

#[test]
fn kv_denial_does_not_revoke_account_or_chat_target_bindings() {
    let state = phase_four_state(vec![]);
    let account = store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    let team = store_id(&CatalogStoreRef::Team(team_ref(
        "work.example",
        "personal",
        "engineering",
    )));
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .failures
        .push(foks_desktop::CatalogFailure {
            scope: foks_desktop::CatalogFailureScope::Profile {
                profile: "work.example".into(),
                source: "KV catalog".into(),
            },
            error: foks_desktop::AgentError::Protocol {
                code: foks_agent_proto::ErrorCode::CapabilityDenied,
                message: "KV unavailable".into(),
                fields: foks_agent_proto::ErrorFields {
                    capability: Some("kv".into()),
                    ..Default::default()
                }
                .into(),
            },
        });
    assert!(state.selected_account(&account).is_ok());
    assert!(state.selected_store(&team).is_ok());
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .blocked_profiles
        .push("work.example".into());
    assert!(state.selected_account(&account).is_err());
    assert!(state.selected_store(&team).is_err());
}

#[test]
fn a_catalog_listing_after_a_local_write_is_read_fresh() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    // Nothing has been written through this desktop, so the agent's retained
    // first pages are inside their own freshness bound.
    assert_eq!(state.catalog_read_freshness(false), (false, 0));
    // The user's own Refresh asks for a listing regardless.
    assert_eq!(state.catalog_read_freshness(true), (true, 0));

    let profile = state.for_profile("work.example").unwrap();
    profile.invalidate_catalog_items(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    let (fresh, epoch) = state.catalog_read_freshness(false);
    assert!(fresh);
    assert_eq!(epoch, 1);
    // Every scope sees the write, including the one it was not made in: the
    // retained pages the agent holds are process-wide.
    assert_eq!(
        state
            .for_profile("other.example")
            .unwrap()
            .catalog_read_freshness(false),
        (true, 1)
    );

    state.note_fresh_catalog_read(epoch);
    assert_eq!(state.catalog_read_freshness(false), (false, 1));

    // A write that lands while a fresh listing runs is not covered by it.
    let (fresh, epoch) = state.catalog_read_freshness(false);
    assert!(!fresh);
    profile.invalidate_catalog();
    state.note_fresh_catalog_read(epoch);
    assert_eq!(state.catalog_read_freshness(false), (true, 2));

    // An ambiguous write stays unreconciled until an authoritative catalog
    // read accepts it, and every listing until then is read fresh.
    state.note_fresh_catalog_read(2);
    assert_eq!(state.catalog_read_freshness(false), (false, 2));
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    assert_eq!(state.catalog_read_freshness(false), (true, 2));
}

/// A profile's own listing settles that profile: without this, every
/// per-profile listing after any write read through as if stale, because only
/// a listing of every profile advanced the epoch it was judged against.
#[test]
fn a_profile_listing_settles_that_profile_and_no_other() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let work = state.for_profile("work.example").unwrap();
    let home = state.for_profile("home.example").unwrap();
    work.invalidate_catalog_items(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));

    let (fresh, epoch) = work.catalog_read_freshness(false);
    assert!(fresh);
    assert_eq!(epoch, 1);
    work.note_fresh_catalog_read(epoch);
    // The profile that was read is settled; its next listing is served from
    // what the agent still holds.
    assert_eq!(work.catalog_read_freshness(false), (false, 1));
    // A second view of the same profile sees the same settlement.
    assert_eq!(
        state
            .for_profile("work.example")
            .unwrap()
            .catalog_read_freshness(false),
        (false, 1)
    );
    // The other profile was not read, so it is still read fresh, and so is a
    // listing of every profile.
    assert_eq!(home.catalog_read_freshness(false), (true, 1));
    assert_eq!(state.catalog_read_freshness(false), (true, 1));

    // A listing of every profile covers each of them.
    state.note_fresh_catalog_read(1);
    assert_eq!(home.catalog_read_freshness(false), (false, 1));
    assert_eq!(state.catalog_read_freshness(false), (false, 1));

    // The next write retires both again, and the profile read settles only
    // the profile it covered.
    home.invalidate_catalog_items(&CatalogStoreRef::Account(account_ref(
        "home.example",
        "home",
    )));
    let (fresh, epoch) = home.catalog_read_freshness(false);
    assert!(fresh);
    assert_eq!(epoch, 2);
    home.note_fresh_catalog_read(epoch);
    assert_eq!(home.catalog_read_freshness(false), (false, 2));
    assert_eq!(work.catalog_read_freshness(false), (true, 2));
    assert_eq!(state.catalog_read_freshness(false), (true, 2));
    // A settled profile is still read fresh when the user asks for it.
    assert_eq!(home.catalog_read_freshness(true), (true, 2));
}

#[test]
fn mutation_gate_refuses_a_second_write_until_the_first_finishes() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let first = state.begin_mutation().unwrap();
    assert_eq!(
        state.begin_mutation().unwrap_err().code,
        "mutation-in-flight"
    );
    assert_eq!(
        state.begin_catalog_load_checked().unwrap_err().code,
        "mutation-in-flight"
    );
    drop(first);
    assert!(state.begin_mutation().is_ok());
}

#[test]
fn ambiguous_mutations_require_a_fresh_catalog_before_another_write() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    let error = state.begin_mutation().unwrap_err();
    assert_eq!(error.code, "ambiguous");
    assert!(error.ambiguous);
    let initialization = state.begin_initialization().unwrap();
    assert_eq!(
        state.begin_initialization().unwrap_err().code,
        "mutation-in-flight"
    );
    drop(initialization);
    assert!(state.mutation_requires_refresh.load(Ordering::Acquire));
    let (generation, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(generation, CatalogSnapshot::default()));
    assert_eq!(state.begin_mutation().unwrap_err().code, "ambiguous");
    let empty = Arc::new(ProfileListTransport {
        value: serde_json::json!([]),
    });
    let (generation, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(
        generation,
        foks_desktop::load_stores(empty.clone()).unwrap()
    ));
    assert_eq!(state.begin_mutation().unwrap_err().code, "ambiguous");
    let (generation, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(generation, foks_desktop::load_catalog(empty).unwrap()));
    assert!(state.begin_mutation().is_ok());
}

#[test]
fn catalog_progress_has_distinct_generations_and_retires_on_root_and_profile_mutation() {
    for profile_mutation in [false, true] {
        let state = phase_four_state(vec![]);
        let (load, _) = state.begin_catalog_load_checked().unwrap();
        let mut published = Vec::new();
        assert!(
            state.publish_catalog(load, CatalogSnapshot::default(), |generation| published
                .push(generation))
        );
        assert!(
            state.publish_catalog(load, CatalogSnapshot::default(), |generation| published
                .push(generation))
        );
        assert!(published[1] > published[0]);
        assert!(state.catalog_at(Some(published[0])).is_err());
        let target = if profile_mutation {
            state.for_profile("work.example").unwrap()
        } else {
            state.clone()
        };
        let permit = target.begin_mutation().unwrap();
        target.invalidate_catalog();
        drop(permit);
        assert!(
            !state.publish_catalog(load, CatalogSnapshot::default(), |_| panic!(
                "retired progress published"
            ))
        );
    }
}

#[test]
fn unrelated_progress_preserves_profile_generation_and_authorization_facts() {
    let state = phase_four_state(vec![]);
    let healthy = state.for_profile("healthy").unwrap();
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    let mut snapshot = CatalogSnapshot {
        profiles: vec!["healthy".into(), "slow".into()],
        ..Default::default()
    };
    snapshot
        .inventory
        .push(foks_desktop::CatalogInventoryState {
            profile: "healthy".into(),
            accounts_complete: true,
            teams_complete: true,
        });
    assert!(state.publish_catalog(load, snapshot.clone(), |_| {}));
    let healthy_generation = healthy.catalog_at(None).unwrap().0;
    let account = AccountDto {
        local_alias: None,
        store: "healthy-store".into(),
        profile: "healthy".into(),
        alias: "alice".into(),
        username: "alice".into(),
    };
    healthy
        .retain_accounts(healthy_generation, &[account])
        .unwrap();
    snapshot
        .inventory
        .push(foks_desktop::CatalogInventoryState {
            profile: "slow".into(),
            accounts_complete: false,
            teams_complete: false,
        });
    assert!(state.publish_catalog(load, snapshot, |_| {}));
    assert_eq!(healthy.catalog_at(None).unwrap().0, healthy_generation);
    assert!(state.accounts.lock().unwrap().contains_key("healthy-store"));
}

#[test]
fn scoped_full_read_preserves_evidence_when_replacing_a_local_only_seed() {
    let state = phase_four_state(vec![]);
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(
        load,
        CatalogSnapshot {
            profiles: vec!["healthy".into(), "slow".into()],
            ..Default::default()
        },
        |_| {}
    ));
    let healthy = state.for_profile("healthy").unwrap();
    let (load, _) = healthy.begin_catalog_load_checked().unwrap();
    assert!(healthy.accept_catalog(load, complete_profile_catalog("healthy")));
    assert_eq!(
        state.catalog_at(None).unwrap().1.unwrap().full_item_reads,
        Some(vec!["healthy".into()])
    );
}

#[test]
fn cancelled_catalog_progress_does_not_replace_the_accepted_snapshot() {
    let state = phase_four_state(vec![]);
    let before = state.catalog_at(None).unwrap();
    let (load, token) = state.begin_catalog_load_checked().unwrap();
    token.cancel();
    assert!(
        !state.publish_catalog(load, CatalogSnapshot::default(), |_| panic!(
            "cancelled publication"
        ))
    );
    assert_eq!(state.catalog_at(None).unwrap(), before);
}

#[test]
fn partial_progress_only_reconciles_profiles_with_authoritative_item_reads() {
    let state = phase_four_state(vec![]);
    let healthy = state.for_profile("healthy").unwrap();
    let slow = state.for_profile("slow").unwrap();
    healthy
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    slow.mutation_requires_refresh
        .store(true, Ordering::Release);
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    let mut partial = CatalogSnapshot {
        profiles: vec!["healthy".into(), "slow".into()],
        inventory: vec![foks_desktop::CatalogInventoryState {
            profile: "healthy".into(),
            accounts_complete: true,
            teams_complete: true,
        }],
        full_item_reads: Some(vec![]),
        ..CatalogSnapshot::default()
    };
    assert!(state.publish_catalog(load, partial.clone(), |_| {}));
    assert!(healthy.mutation_requires_refresh.load(Ordering::Acquire));
    partial.full_item_reads = Some(vec!["healthy".into()]);
    assert!(state.publish_catalog(load, partial, |_| {}));
    assert!(!healthy.mutation_requires_refresh.load(Ordering::Acquire));
    assert!(slow.mutation_requires_refresh.load(Ordering::Acquire));
}

#[test]
fn incomplete_catalog_does_not_release_an_ambiguous_mutation() {
    let state = phase_four_state(vec![]);
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    let (generation, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(
        generation,
        CatalogSnapshot {
            profiles: vec!["work.example".into()],
            inventory: vec![foks_desktop::CatalogInventoryState {
                profile: "work.example".into(),
                accounts_complete: false,
                teams_complete: true,
            }],
            ..CatalogSnapshot::default()
        }
    ));
    assert_eq!(state.begin_mutation().unwrap_err().code, "ambiguous");
}

#[test]
fn profile_mutations_and_local_aliases_are_independent_but_root_is_exclusive() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("work.example").unwrap();
    let b = state.for_profile("home.example").unwrap();
    let aliases = state.for_local_aliases().unwrap();
    let first = a.begin_mutation().unwrap();
    assert_eq!(a.begin_mutation().unwrap_err().code, "mutation-in-flight");
    let second = b.begin_mutation().unwrap();
    let local = aliases.begin_mutation().unwrap();
    assert_eq!(
        aliases.begin_mutation().unwrap_err().code,
        "mutation-in-flight"
    );
    assert_eq!(
        state.begin_mutation().unwrap_err().code,
        "mutation-in-flight"
    );
    assert_eq!(
        state.begin_initialization().unwrap_err().code,
        "mutation-in-flight"
    );
    drop((first, second, local));
    let root = state.begin_mutation().unwrap();
    assert_eq!(a.begin_mutation().unwrap_err().code, "mutation-in-flight");
    assert_eq!(
        aliases.begin_mutation().unwrap_err().code,
        "mutation-in-flight"
    );
    drop(root);
    crate::commands::execution::ambiguous_worker_failure(&a, "unknown remote outcome");
    assert_eq!(a.begin_mutation().unwrap_err().code, "ambiguous");
    assert_eq!(state.begin_mutation().unwrap_err().code, "ambiguous");
    assert_eq!(state.begin_initialization().unwrap_err().code, "ambiguous");
    assert!(b.begin_mutation().is_ok());
    assert!(aliases.begin_mutation().is_ok());
    assert!(a.mutation_requires_refresh.load(Ordering::Acquire));
}

#[test]
fn profile_catalog_changes_preserve_other_profiles_and_local_account_binding() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("work.example").unwrap();
    let b = state.for_profile("home.example").unwrap();
    let b_store = store_id(&CatalogStoreRef::Account(account_ref(
        "home.example",
        "personal",
    )));
    let a_store = store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .stores
        .push(CatalogStoreSummary::Account {
            store: account_ref("home.example", "personal"),
        });
    state
        .catalog
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .full_item_reads = Some(vec!["work.example".into(), "home.example".into()]);
    let before = b.catalog_at(None).unwrap().0;
    let permit = a.begin_mutation().unwrap();
    a.invalidate_catalog();
    assert_eq!(
        state.catalog_at(None).unwrap().1.unwrap().full_item_reads,
        Some(vec!["home.example".into()])
    );
    assert!(a.selected_store(&a_store).is_err());
    assert!(state.local_account(&a_store).is_ok());
    assert!(b.selected_store(&b_store).is_ok());
    assert!(b.catalog_at(Some(before)).is_ok());
    assert!(b.retain_roster(before, b_store.clone(), &[]).is_ok());
    drop(permit);
    let (generation, _) = a.begin_catalog_load_checked().unwrap();
    assert!(a.accept_catalog(generation, complete_profile_catalog("work.example")));
    assert!(b.selected_store(&b_store).is_ok());
    assert!(b.catalog_at(Some(before)).is_ok());
    assert!(b.rosters.lock().unwrap().contains_key(&b_store));
    assert_eq!(
        state.catalog_at(None).unwrap().1.unwrap().full_item_reads,
        Some(vec!["home.example".into(), "work.example".into()])
    );
    let (generation, _) = a.begin_catalog_load_checked().unwrap();
    assert!(a.accept_catalog(
        generation,
        CatalogSnapshot {
            full_item_reads: None,
            ..complete_profile_catalog("work.example")
        }
    ));
    assert_eq!(
        state.catalog_at(None).unwrap().1.unwrap().full_item_reads,
        Some(vec!["home.example".into()])
    );
    assert!(b.catalog_at(Some(before)).is_ok());
}

fn complete_profile_catalog(profile: &str) -> CatalogSnapshot {
    CatalogSnapshot {
        profiles: vec![profile.into()],
        full_item_reads: Some(vec![profile.into()]),
        inventory: vec![foks_desktop::CatalogInventoryState {
            profile: profile.into(),
            accounts_complete: true,
            teams_complete: true,
        }],
        ..CatalogSnapshot::default()
    }
}

#[test]
fn ambiguity_is_released_only_by_complete_catalog_for_that_profile() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("work.example").unwrap();
    let b = state.for_profile("home.example").unwrap();
    crate::commands::execution::ambiguous_worker_failure(&a, "unknown remote outcome");
    let (generation, _) = b.begin_catalog_load_checked().unwrap();
    assert!(b.accept_catalog(generation, complete_profile_catalog("home.example")));
    assert_eq!(a.begin_mutation().unwrap_err().code, "ambiguous");
    for incomplete in [
        CatalogSnapshot::default(),
        CatalogSnapshot {
            profiles: vec!["work.example".into()],
            ..CatalogSnapshot::default()
        },
        CatalogSnapshot {
            blocked_profiles: vec!["work.example".into()],
            ..complete_profile_catalog("work.example")
        },
        CatalogSnapshot {
            profile_overviews: vec![foks_agent_proto::ProfileOverview {
                profile: "work.example".into(),
                accounts: foks_agent_proto::ResponseResult::Success {
                    value: serde_json::json!([]),
                },
                teams: foks_agent_proto::ResponseResult::Success {
                    value: serde_json::json!([]),
                },
                server_status: foks_agent_proto::ResponseResult::Error {
                    code: foks_agent_proto::ErrorCode::Busy,
                    message: "status unavailable".into(),
                    fields: Default::default(),
                },
            }],
            ..complete_profile_catalog("work.example")
        },
        CatalogSnapshot {
            failures: vec![foks_desktop::CatalogFailure {
                scope: foks_desktop::CatalogFailureScope::Store(CatalogStoreRef::Account(
                    account_ref("work.example", "personal"),
                )),
                error: foks_desktop::AgentError::Transport("KV unavailable".into()),
            }],
            ..complete_profile_catalog("work.example")
        },
    ] {
        let (generation, _) = state.begin_catalog_load_checked().unwrap();
        assert!(state.accept_catalog(generation, incomplete));
        assert_eq!(a.begin_mutation().unwrap_err().code, "ambiguous");
    }
    let (generation, _) = a.begin_catalog_load_checked().unwrap();
    assert!(!a.accept_catalog(generation, complete_profile_catalog("home.example")));
    assert!(!a.accept_catalog(
        generation,
        CatalogSnapshot {
            full_item_reads: Some(vec!["home.example".into()]),
            ..complete_profile_catalog("work.example")
        }
    ));
    assert_eq!(a.begin_mutation().unwrap_err().code, "ambiguous");
    assert!(a.accept_catalog(generation, complete_profile_catalog("work.example")));
    assert!(a.begin_mutation().is_ok());
    assert!(state.begin_mutation().is_ok());
}

#[test]
fn retired_account_bindings_only_authorize_local_aliases_until_root_invalidation() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("work.example").unwrap();
    let account = store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    a.invalidate_catalog();
    let (generation, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(
        generation,
        CatalogSnapshot {
            profiles: vec!["work.example".into()],
            ..CatalogSnapshot::default()
        }
    ));
    assert!(state.local_account(&account).is_ok());
    assert!(state.selected_account(&account).is_err());
    state.invalidate_catalog();
    assert!(state.local_account(&account).is_err());
}

#[test]
fn root_publication_never_reuses_a_profile_fact_generation() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("work.example").unwrap();
    let permit = a.begin_mutation().unwrap();
    a.invalidate_catalog();
    let old = a.catalog_at(None).unwrap().0;
    drop(permit);
    let (generation, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(generation, complete_profile_catalog("work.example")));
    assert!(a.retain_roster(old, "stale".into(), &[]).is_err());
    assert!(a.catalog_at(Some(old)).is_err());
}

#[test]
fn root_reservation_retires_private_profile_refreshes() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("work.example").unwrap();
    let (generation, _) = a.begin_catalog_load_checked().unwrap();
    let _permit = state.begin_mutation().unwrap();
    assert!(!a.accept_catalog(generation, complete_profile_catalog("work.example")));
}

#[test]
fn mutation_scope_bookkeeping_is_bounded_without_evicting_ambiguity() {
    let state = phase_four_state(vec![]);
    let a = state.for_profile("uncertain").unwrap();
    crate::commands::execution::ambiguous_worker_failure(&a, "unknown remote outcome");
    drop(a);
    let views: Vec<_> = (0..255)
        .map(|index| state.for_profile(&format!("profile-{index}")).unwrap())
        .collect();
    assert!(state.for_profile("overflow").is_err());
    assert!(state.for_local_aliases().unwrap().begin_mutation().is_ok());
    drop(views);
    for index in 255..600 {
        assert!(state
            .for_profile(&format!("profile-{index}"))
            .unwrap()
            .begin_mutation()
            .is_ok());
    }
    assert_eq!(
        state
            .for_profile("uncertain")
            .unwrap()
            .begin_mutation()
            .unwrap_err()
            .code,
        "ambiguous"
    );
    assert_eq!(state.begin_initialization().unwrap_err().code, "ambiguous");
}

#[test]
fn renderer_can_only_consume_paths_from_the_latest_native_drop_once() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let first = PathBuf::from("/tmp/first");
    let second = PathBuf::from("/tmp/second");
    assert_eq!(
        state.record_drop_paths(std::slice::from_ref(&first)),
        vec!["/tmp/first"]
    );
    assert_eq!(state.take_drop_path("/tmp/first").unwrap(), first);
    assert_eq!(
        state.take_drop_path("/tmp/first").unwrap_err().code,
        "drop-not-authorized"
    );
    state.record_drop_paths(&[second]);
    assert_eq!(
        state.take_drop_path("/tmp/first").unwrap_err().code,
        "drop-not-authorized"
    );
    let third = PathBuf::from("/tmp/third");
    let fourth = PathBuf::from("/tmp/fourth");
    assert_eq!(
        state.record_drop_paths(&[third, fourth]),
        vec!["/tmp/third", "/tmp/fourth"]
    );
    for path in ["/tmp/third", "/tmp/fourth"] {
        assert_eq!(
            state.take_drop_path(path).unwrap_err().code,
            "drop-not-authorized"
        );
    }
}

#[test]
fn known_store_metadata_never_authorizes_an_account_operation() {
    let state = phase_four_state(vec![]);
    let account = account_ref("work.example", "personal");
    *state.catalog.lock().unwrap() = Some(CatalogSnapshot {
        profiles: vec!["work.example".to_owned()],
        known_stores: vec![CatalogStoreSummary::Account {
            store: account.clone(),
        }],
        ..CatalogSnapshot::default()
    });
    assert_eq!(
        state
            .selected_account(&store_id(&CatalogStoreRef::Account(account)))
            .unwrap_err()
            .code,
        "store-not-found"
    );
}

#[test]
fn catalog_reads_bound_to_a_generation_fail_once_it_is_replaced() {
    let state = phase_four_state(vec![]);
    let (generation, catalog) = state.catalog_at(None).unwrap();
    assert!(catalog.is_some());
    assert!(state.catalog_at(Some(generation)).is_ok());
    // A replacement is private until publication; the accepted token and
    // facts stay valid while a refresh is running or fails.
    let (next, _token) = state.begin_catalog_load_checked().unwrap();
    assert!(next > generation);
    assert!(state.catalog_at(Some(generation)).unwrap().1.is_some());
    assert!(state.catalog_at(Some(next)).is_err());
    assert!(state.accept_catalog(next, CatalogSnapshot::default()));
    let stale = state.catalog_at(Some(generation)).unwrap_err();
    assert_eq!(stale.code, "catalog-required");
    assert!(stale.retryable);
    assert_eq!(state.catalog_at(Some(next)).unwrap().0, next);
}

#[test]
fn a_catalog_load_that_lost_its_generation_is_not_accepted() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let (first, _first_token) = state.begin_catalog_load_checked().unwrap();
    let (second, _second_token) = state.begin_catalog_load_checked().unwrap();
    assert!(!state.accept_catalog(first, CatalogSnapshot::default()));
    assert!(state.catalog.lock().unwrap().is_none());
    assert!(state.accept_catalog(second, CatalogSnapshot::default()));
    assert!(state.catalog.lock().unwrap().is_some());
}

#[test]
fn invitation_history_does_not_retire_catalog_or_compete_with_mutations() {
    use foks_agent_proto::invitations::InvitationAction;
    let state = phase_four_state(vec![]);
    let before = state.catalog_at(None).unwrap().0;
    let writer = state.begin_mutation().unwrap();
    for action in [
        InvitationAction::List,
        InvitationAction::PendingApprovals {
            team_alias: "team".into(),
        },
    ] {
        assert!(state
            .begin_catalog_action(action.changes_catalog())
            .unwrap()
            .is_none());
        assert_eq!(state.catalog_at(None).unwrap().0, before);
        assert!(state.catalog_at(None).unwrap().1.is_some());
    }
    drop(writer);
    let writer = state
        .begin_catalog_action(
            InvitationAction::Create {
                team_alias: "team".into(),
            }
            .changes_catalog(),
        )
        .unwrap();
    assert!(writer.is_some());
    assert!(state.catalog_at(None).unwrap().1.is_none());
}

#[test]
fn failed_refresh_preserves_store_selection_but_mutation_retires_it() {
    let state = phase_four_state(vec![]);
    let before = state.catalog_at(None).unwrap();
    // Use a real fixture target instead of depending on its display label.
    let target = before.1.as_ref().unwrap().stores[0].store_ref();
    let target_id = store_id(&target);
    let (_loading, token) = state.begin_catalog_load_checked().unwrap();
    token.cancel();
    assert!(state.selected_store(&target_id).is_ok());
    assert_eq!(state.catalog_at(None).unwrap(), before);
    state.invalidate_catalog();
    assert_eq!(
        state.selected_store(&target_id).unwrap_err().code,
        "catalog-required"
    );
}

#[test]
fn accepted_catalog_facts_survive_refresh_until_a_replacement_is_published() {
    let state = phase_four_state(vec![]);
    let (generation, catalog) = state.catalog_at(None).unwrap();
    let catalog = catalog.unwrap();
    let account = AccountDto {
        local_alias: None,
        store: store_id(&CatalogStoreRef::Account(account_ref(
            "work.example",
            "personal",
        ))),
        profile: "work.example".into(),
        alias: "personal".into(),
        username: "alice".into(),
    };
    let device = DeviceDto {
        id: format!("04{}", "11".repeat(32)),
        name: Some("Other Mac".into()),
        role: "owner",
        current: false,
    };
    let group = store_id(&CatalogStoreRef::Team(team_ref(
        "work.example",
        "personal",
        "engineering",
    )));
    let retain_facts = |generation| {
        state.retain_accounts(generation, std::slice::from_ref(&account))?;
        state.retain_devices(
            generation,
            account.store.clone(),
            std::slice::from_ref(&device),
        )?;
        // An empty response is still a retained fact. A missing map entry
        // requires a read before an operation can use this group.
        state.retain_group_details(generation, group.clone(), Some(&[]), Some(&[]))
    };
    let assert_retained_facts = || {
        assert_eq!(state.selected_account_dto(&account.store).unwrap(), account);
        assert!(state
            .selected_device_target(&account.store, &device.id)
            .is_ok());
        assert_eq!(state.rosters.lock().unwrap().get(&group), Some(&vec![]));
        assert_eq!(state.federations.lock().unwrap().get(&group), Some(&vec![]));
    };
    let assert_no_facts = || {
        assert!(state.accounts.lock().unwrap().is_empty());
        assert!(state.devices.lock().unwrap().is_empty());
        assert!(state.rosters.lock().unwrap().is_empty());
        assert!(state.federations.lock().unwrap().is_empty());
    };
    retain_facts(generation).unwrap();
    let (failed, failed_token) = state.begin_catalog_load_checked().unwrap();
    assert_retained_facts();
    failed_token.cancel();
    assert_retained_facts();
    assert_eq!(state.catalog_at(None).unwrap().0, generation);

    let (replacement, _token) = state.begin_catalog_load_checked().unwrap();
    assert!(!state.accept_catalog(failed, CatalogSnapshot::default()));
    assert_retained_facts();
    assert!(state.accept_catalog(replacement, catalog));
    assert_no_facts();
    assert_eq!(
        state.selected_account_dto(&account.store).unwrap_err().code,
        "accounts-required"
    );
    assert_eq!(
        state
            .selected_device_target(&account.store, &device.id)
            .unwrap_err()
            .code,
        "devices-required"
    );

    // Replies from before publication cannot reintroduce retired facts,
    // whether the caller fetched individual sections or combined details.
    assert_eq!(
        retain_facts(generation).unwrap_err().code,
        "catalog-required"
    );
    assert!(state
        .retain_devices(
            generation,
            account.store.clone(),
            std::slice::from_ref(&device)
        )
        .is_err());
    assert!(state.retain_roster(generation, group.clone(), &[]).is_err());
    assert!(state
        .retain_federation(generation, group.clone(), &[])
        .is_err());
    assert!(state
        .retain_group_details(generation, group.clone(), Some(&[]), Some(&[]))
        .is_err());
    assert_no_facts();
    retain_facts(replacement).unwrap();
    assert_retained_facts();
}

#[test]
fn mutation_cancels_a_private_catalog_load_and_rejects_its_late_reply() {
    let state = phase_four_state(vec![]);
    let (published, catalog) = state.catalog_at(None).unwrap();
    let (loading, token) = state.begin_catalog_load_checked().unwrap();
    assert!(loading > published);
    let transport = Arc::new(ProfileListTransport {
        value: serde_json::json!([]),
    });
    assert!(
        foks_desktop::load_catalog_cancellable(transport.clone(), token.clone(), false).is_ok()
    );
    let mutation = state.begin_catalog_action(true).unwrap().unwrap();
    // The same transport returned a valid response before invalidation.
    assert!(matches!(
        foks_desktop::load_catalog_cancellable(transport, token, false),
        Err(foks_desktop::AgentError::Cancelled)
    ));
    let (invalidated, catalog_after_mutation) = state.catalog_at(None).unwrap();
    assert!(invalidated > loading);
    assert!(catalog_after_mutation.is_none());
    assert!(!state.accept_catalog(loading, catalog.clone().unwrap()));
    assert!(state.catalog_at(None).unwrap().1.is_none());
    assert!(state.begin_catalog_load_checked().is_err());
    drop(mutation);

    let (replacement, _token) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(replacement, catalog.unwrap()));
    assert_eq!(state.catalog_at(None).unwrap().0, replacement);
    assert!(state.catalog_at(None).unwrap().1.is_some());
}

#[test]
fn an_unrefreshed_profile_reports_catalog_required_rather_than_a_missing_store() {
    let state = phase_four_state(vec![]).for_profile("chat").unwrap();
    let catalog = chat_catalog();
    let id = store_id(&catalog.stores[0].store_ref());
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, catalog.clone(), |_| {}));
    assert!(state.selected_chat(&id).is_ok());
    // The profile's overview read failed: no stores were read, and the read
    // recorded its inventory as incomplete, as every read does. The team is
    // still shown from the previous read, so the answer names the
    // unrefreshed vault and invites a retry.
    let failed = CatalogSnapshot {
        profiles: vec!["chat".into()],
        known_stores: catalog.known_stores.clone(),
        inventory: vec![foks_desktop::CatalogInventoryState {
            profile: "chat".into(),
            accounts_complete: false,
            teams_complete: false,
        }],
        failures: vec![foks_desktop::CatalogFailure {
            scope: foks_desktop::CatalogFailureScope::Profile {
                profile: "chat".into(),
                source: "profile overview".into(),
            },
            error: foks_desktop::AgentError::Transport("unavailable".into()),
        }],
        ..Default::default()
    };
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, failed, |_| {}));
    let error = state.selected_chat(&id).unwrap_err();
    assert_eq!(error.code, "catalog-required");
    assert!(error.retryable);
    // The accounts were read but the teams were not.
    let mut partial = complete_profile_catalog("chat");
    partial.inventory[0].teams_complete = false;
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, partial, |_| {}));
    let error = state.selected_chat(&id).unwrap_err();
    assert_eq!(error.code, "catalog-required");
    assert!(error.retryable);
    // A complete read that does not list the team: it is gone.
    let (load, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.publish_catalog(load, complete_profile_catalog("chat"), |_| {}));
    let error = state.selected_chat(&id).unwrap_err();
    assert_eq!(error.code, "store-not-found");
    assert!(!error.retryable);
}
