use crate::commands::store_key::{store_id, StoreKey};
use crate::commands::tests::support::{account_ref, phase_four_state, team_ref};
use foks_desktop::CatalogStoreRef;

/// The identity string the renderer has received since store ids were first
/// built with `serde_json::json!`.
fn json_store_id(store: &CatalogStoreRef) -> String {
    match store {
        CatalogStoreRef::Account(store) => serde_json::json!({
            "kind": "account",
            "profile": store.profile,
            "accountAlias": store.account_alias,
        }),
        CatalogStoreRef::Team(store) => serde_json::json!({
            "kind": "team",
            "profile": store.profile,
            "accountAlias": store.account_alias,
            "teamAlias": store.team_alias,
            "teamId": store.team_id,
        }),
    }
    .to_string()
}

fn sample_stores() -> Vec<CatalogStoreRef> {
    let escaped = "q\"b\\s/é\u{1}\u{7f}\u{2028}</script>";
    vec![
        CatalogStoreRef::Account(account_ref("work.example", "personal")),
        CatalogStoreRef::Account(account_ref(escaped, escaped)),
        CatalogStoreRef::Team(team_ref("work.example", "personal", "engineering")),
        CatalogStoreRef::Team(foks_agent_proto::TeamStoreRef {
            profile: escaped.to_owned(),
            account_alias: escaped.to_owned(),
            team_alias: escaped.to_owned(),
            team_id: escaped.to_owned(),
        }),
    ]
}

#[test]
fn store_ids_keep_the_sorted_json_object_form() {
    assert_eq!(
        store_id(&CatalogStoreRef::Account(account_ref(
            "work.example",
            "personal"
        ))),
        r#"{"accountAlias":"personal","kind":"account","profile":"work.example"}"#,
    );
    assert_eq!(
        store_id(&CatalogStoreRef::Team(team_ref(
            "work.example",
            "personal",
            "engineering"
        ))),
        r#"{"accountAlias":"personal","kind":"team","profile":"work.example","teamAlias":"engineering","teamId":"03engineering"}"#,
    );
    for store in sample_stores() {
        assert_eq!(store_id(&store), json_store_id(&store));
    }
}

#[test]
fn store_keys_parse_only_the_identity_a_store_produces() {
    for store in sample_stores() {
        let key = StoreKey::parse(&store_id(&store)).expect("a produced identity parses");
        assert_eq!(key.store(), &store);
        assert_eq!(key.profile(), store.profile());
    }
    for id in [
        "",
        "null",
        "[]",
        r#"{"accountAlias":"personal","kind":"account"}"#,
        // Whitespace and key order other than the produced form.
        r#"{"accountAlias": "personal","kind":"account","profile":"work.example"}"#,
        r#"{"kind":"account","accountAlias":"personal","profile":"work.example"}"#,
        // An escape the produced form does not use.
        r#"{"accountAlias":"person\u0061l","kind":"account","profile":"work.example"}"#,
        // Fields that do not belong to the kind, or are unknown.
        r#"{"accountAlias":"personal","kind":"account","profile":"work.example","teamAlias":"x"}"#,
        r#"{"accountAlias":"personal","kind":"account","profile":"work.example","teamAlias":null}"#,
        r#"{"accountAlias":"personal","kind":"account","profile":"work.example","extra":1}"#,
        r#"{"accountAlias":"personal","kind":"team","profile":"work.example","teamAlias":"engineering"}"#,
        r#"{"accountAlias":"personal","kind":"group","profile":"work.example"}"#,
    ] {
        assert!(StoreKey::parse(id).is_none(), "{id} parsed");
    }
}

#[test]
fn an_identity_selects_only_a_store_of_its_own_kind() {
    let state = phase_four_state(vec![]);
    let account = CatalogStoreRef::Account(account_ref("work.example", "personal"));
    let team = CatalogStoreRef::Team(team_ref("work.example", "personal", "engineering"));
    assert_eq!(
        state.selected_store(&store_id(&account)).unwrap(),
        (account.clone(), None)
    );
    assert_eq!(
        state.selected_store(&store_id(&team)).unwrap(),
        (team.clone(), Some(true))
    );
    assert_eq!(
        state.selected_account(&store_id(&team)).unwrap_err().code,
        "store-not-found"
    );
    assert_eq!(
        state.selected_team(&store_id(&account)).unwrap_err().code,
        "store-not-found"
    );
    // A store with the same profile and aliases but another team id is a
    // different store.
    let mut other = team_ref("work.example", "personal", "engineering");
    other.team_id = "03other".to_owned();
    assert_eq!(
        state
            .selected_store(&store_id(&CatalogStoreRef::Team(other)))
            .unwrap_err()
            .code,
        "store-not-found"
    );
}

#[test]
fn an_identity_in_another_form_selects_nothing_and_scopes_nothing() {
    let state = phase_four_state(vec![]);
    let reordered = r#"{"kind":"account","accountAlias":"personal","profile":"work.example"}"#;
    assert_eq!(
        state.selected_store(reordered).unwrap_err().code,
        "store-not-found"
    );
    assert_eq!(
        state.for_store(reordered).err().unwrap().code,
        "invalid-request"
    );
    let canonical = store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    assert_eq!(
        state.for_store(&canonical).unwrap().mutation_profile(),
        Some("work.example")
    );
}
