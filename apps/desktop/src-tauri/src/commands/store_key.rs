//! Vault store identities exchanged with the renderer.
//!
//! The renderer receives each store's identity as a JSON object string, echoes
//! it back unchanged in later commands, and parses it in `chat-contract.ts`.
//! The string has the keys `accountAlias`, `kind`, `profile` and, for a team,
//! `teamAlias` and `teamId`, in that sorted order, with no whitespace: the
//! form `serde_json::json!` produced for these objects before this module
//! existed, so identities a running renderer already holds stay valid.

use foks_agent_proto::{AccountStoreRef, TeamStoreRef};
use foks_desktop::CatalogStoreRef;
use serde::{Deserialize, Serialize};

/// A store identity parsed from the renderer's string. Lookups compare it
/// with catalog entries field by field instead of serializing each candidate.
#[derive(Debug)]
pub(super) struct StoreKey(CatalogStoreRef);

/// The wire object. Fields are declared in sorted key order, which is the
/// order serde writes them in.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireRef<'a> {
    account_alias: &'a str,
    kind: &'static str,
    profile: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_alias: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Wire {
    account_alias: String,
    kind: String,
    profile: String,
    team_alias: Option<String>,
    team_id: Option<String>,
}

impl StoreKey {
    /// Parses a store identity. Only the exact string [`store_id`] produces
    /// for some store is accepted, so each store has one identity and a
    /// parsed key can be compared with the string keys of the fact caches.
    pub(super) fn parse(id: &str) -> Option<Self> {
        let wire: Wire = serde_json::from_str(id).ok()?;
        let store = match (wire.kind.as_str(), wire.team_alias, wire.team_id) {
            ("account", None, None) => CatalogStoreRef::Account(AccountStoreRef {
                profile: wire.profile,
                account_alias: wire.account_alias,
            }),
            ("team", Some(team_alias), Some(team_id)) => CatalogStoreRef::Team(TeamStoreRef {
                profile: wire.profile,
                account_alias: wire.account_alias,
                team_alias,
                team_id,
            }),
            _ => return None,
        };
        (store_id(&store) == id).then_some(Self(store))
    }

    pub(super) fn store(&self) -> &CatalogStoreRef {
        &self.0
    }

    pub(super) fn profile(&self) -> &str {
        self.0.profile()
    }

    /// The account store this key names, if it names one.
    pub(super) fn account(&self) -> Option<&AccountStoreRef> {
        match &self.0 {
            CatalogStoreRef::Account(store) => Some(store),
            CatalogStoreRef::Team(_) => None,
        }
    }

    /// The team store this key names, if it names one.
    pub(super) fn team(&self) -> Option<&TeamStoreRef> {
        match &self.0 {
            CatalogStoreRef::Account(_) => None,
            CatalogStoreRef::Team(store) => Some(store),
        }
    }
}

/// The renderer's identity string for `store`.
pub(super) fn store_id(store: &CatalogStoreRef) -> String {
    let wire = match store {
        CatalogStoreRef::Account(store) => WireRef {
            account_alias: &store.account_alias,
            kind: "account",
            profile: &store.profile,
            team_alias: None,
            team_id: None,
        },
        CatalogStoreRef::Team(store) => WireRef {
            account_alias: &store.account_alias,
            kind: "team",
            profile: &store.profile,
            team_alias: Some(&store.team_alias),
            team_id: Some(&store.team_id),
        },
    };
    serde_json::to_string(&wire).expect("a struct of strings serializes")
}
