use crate::agent::AgentHandle;
use crate::commands::context::AppState;
use crate::commands::servers::{ProfileProtocolSummary, ProfileSummary, ProfileTrustSummary};
use foks_desktop::{CatalogSnapshot, CatalogStoreSummary};
use std::sync::Arc;

pub(super) fn test_profile(name: impl Into<String>) -> ProfileSummary {
    ProfileSummary {
        name: name.into(),
        probe: "foks.example".to_owned(),
        protocol: ProfileProtocolSummary::V019,
        trust: ProfileTrustSummary::WebPki,
    }
}

pub(super) fn test_profile_value(name: impl Into<String>) -> serde_json::Value {
    serde_json::to_value(test_profile(name)).unwrap()
}

pub(super) fn account_ref(profile: &str, alias: &str) -> foks_agent_proto::AccountStoreRef {
    foks_agent_proto::AccountStoreRef {
        profile: profile.to_owned(),
        account_alias: alias.to_owned(),
    }
}

pub(super) fn team_ref(
    profile: &str,
    account_alias: &str,
    team_alias: &str,
) -> foks_agent_proto::TeamStoreRef {
    foks_agent_proto::TeamStoreRef {
        profile: profile.to_owned(),
        account_alias: account_alias.to_owned(),
        team_alias: team_alias.to_owned(),
        team_id: format!("03{team_alias}"),
    }
}

pub(super) fn phase_four_catalog(blocked_profiles: Vec<String>) -> CatalogSnapshot {
    CatalogSnapshot {
        profiles: vec!["work.example".to_owned(), "home.example".to_owned()],
        stores: vec![
            CatalogStoreSummary::Account {
                store: account_ref("work.example", "personal"),
            },
            CatalogStoreSummary::Team {
                store: team_ref("work.example", "personal", "engineering"),
                kind: "named".to_owned(),
                name: Some("Engineering".to_owned()),
                active: true,
            },
            CatalogStoreSummary::Team {
                store: team_ref("home.example", "home", "homelab"),
                kind: "named".to_owned(),
                name: Some("Homelab".to_owned()),
                active: true,
            },
        ],
        blocked_profiles,
        ..CatalogSnapshot::default()
    }
}

pub(super) fn phase_four_state(blocked_profiles: Vec<String>) -> AppState {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    *state.catalog.lock().unwrap() = Some(phase_four_catalog(blocked_profiles));
    state
}
