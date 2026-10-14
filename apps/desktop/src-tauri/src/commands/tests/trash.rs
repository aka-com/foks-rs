use crate::commands::execution::MutationKind;
use crate::commands::trash::{
    decode_original_path, empty_trash_plan, encode_original_path, execute_trash_plan, in_trash,
    parse_trash_name, path_read_role, removal_plan, removal_without_trash_plan, restore_plan,
    role_tag, trash_entry_name, trash_plan, utc_stamp, TrashName, TrashStep, TRASH_UNAVAILABLE,
};
use foks_agent_proto::{KvPrecondition, KvRole, KvStoreRef, Operation};
use foks_desktop::{CatalogItem, CatalogStoreRef};
use std::sync::Mutex;

/// 2023-11-14T22:13:20Z.
const NOW: u64 = 1_700_000_000;
const STAMP: &str = "20231114T221320";

fn account() -> CatalogStoreRef {
    CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
    })
}

fn team() -> CatalogStoreRef {
    CatalogStoreRef::Team(foks_agent_proto::TeamStoreRef {
        profile: "foks.example".to_owned(),
        account_alias: "personal".to_owned(),
        team_alias: "household".to_owned(),
        team_id: "03".repeat(33),
    })
}

fn kv(store: &CatalogStoreRef) -> KvStoreRef {
    match store {
        CatalogStoreRef::Account(store) => KvStoreRef::Account(store.clone()),
        CatalogStoreRef::Team(store) => KvStoreRef::Team(store.clone()),
    }
}

fn entry(
    store: &CatalogStoreRef,
    path: &str,
    node_type: &str,
    read_role: KvRole,
    write_role: KvRole,
    id: u8,
) -> CatalogItem {
    CatalogItem {
        store: store.clone(),
        metadata: foks_agent_proto::KvEntryMetadata {
            dirent_id: [id; 16],
            path: path.to_owned(),
            node_type: node_type.to_owned(),
            version: u64::from(id) + 1,
            size: None,
            read_role,
            write_role,
            shape: None,
        },
    }
}

fn secret(store: &CatalogStoreRef, path: &str, id: u8) -> CatalogItem {
    entry(store, path, "small-file", KvRole::Owner, KvRole::Owner, id)
}

fn folder(store: &CatalogStoreRef, path: &str, read_role: KvRole, id: u8) -> CatalogItem {
    entry(store, path, "directory", read_role, read_role, id)
}

fn mkdir(
    store: &CatalogStoreRef,
    path: &str,
    read_role: KvRole,
    write_role: KvRole,
    mkdir_p: bool,
) -> Operation {
    Operation::MkdirKv {
        store: kv(store),
        path: path.to_owned(),
        read_role,
        write_role,
        precondition: KvPrecondition::Create,
        mkdir_p,
    }
}

fn moved(item: &CatalogItem, destination: &str) -> Operation {
    Operation::MoveKv {
        dirent_id: item.metadata.dirent_id,
        store: kv(&item.store),
        path: item.metadata.path.clone(),
        destination: destination.to_owned(),
        version: item.metadata.version,
    }
}

fn removed(item: &CatalogItem, recursive: bool) -> Operation {
    Operation::RemoveKv {
        expected_dirent_id: Some(item.metadata.dirent_id),
        store: kv(&item.store),
        path: item.metadata.path.clone(),
        recursive,
        precondition: KvPrecondition::ExactVersion {
            version: item.metadata.version,
        },
    }
}

fn operations(steps: &[TrashStep]) -> Vec<Operation> {
    steps.iter().map(|step| step.operation.clone()).collect()
}

#[test]
fn entry_names_carry_the_utc_deletion_time_and_the_encoded_original_path() {
    assert_eq!(utc_stamp(0), "19700101T000000");
    assert_eq!(utc_stamp(NOW), STAMP);
    assert_eq!(utc_stamp(951_782_400 + 3_661), "20000229T010101");
    assert_eq!(utc_stamp(4_102_444_799), "20991231T235959");

    let original = "/logins/50% off~deals/\u{e9}t\u{e9}\tnote";
    let encoded = encode_original_path(original);
    assert_eq!(
        encoded,
        "%2Flogins%2F50%25 off~deals%2F\u{e9}t\u{e9}%09note"
    );
    assert!(!encoded.contains('/'));
    assert_eq!(decode_original_path(&encoded).as_deref(), Some(original));

    let (name, origin) = trash_entry_name(STAMP, original);
    assert_eq!(origin, None);
    assert_eq!(name, format!("{STAMP}~{encoded}"));
    assert_eq!(
        parse_trash_name(&name),
        Some(TrashName::Path {
            stamp: STAMP.to_owned(),
            original: original.to_owned(),
        })
    );
    for foreign in [
        "github.com",
        "2023111T221320~%2Fa",
        "20231114X221320~%2Fa",
        "20231114T221320~a",
        "20231114T221320~%2",
        "20231114T221320~%FF",
        "20231114T221320~#short",
    ] {
        assert_eq!(parse_trash_name(foreign), None, "{foreign}");
    }
}

#[test]
fn names_past_the_component_limit_are_hashed_with_an_origin_file() {
    let original = format!("/{}", "a".repeat(236));
    let (name, origin) = trash_entry_name(STAMP, &original);
    assert_eq!(name.len(), 255);
    assert_eq!(origin, None);

    let original = format!("/{}", "a".repeat(237));
    let (name, origin) = trash_entry_name(STAMP, &original);
    let origin = origin.unwrap();
    assert!(name.starts_with(&format!("{STAMP}~#")));
    assert_eq!(name.len(), STAMP.len() + 2 + 32);
    assert_eq!(origin, format!("{name}.origin"));
    assert_eq!(
        parse_trash_name(&name),
        Some(TrashName::Hashed {
            stamp: STAMP.to_owned()
        })
    );
    assert_eq!(parse_trash_name(&origin), Some(TrashName::Origin));
    assert_eq!(trash_entry_name(STAMP, &original).0, name);
}

#[test]
fn role_tags_name_each_read_role() {
    assert_eq!(role_tag(KvRole::Owner), "owner");
    assert_eq!(role_tag(KvRole::Admin), "admin");
    assert_eq!(role_tag(KvRole::Member { visibility: 0 }), "member-0");
    assert_eq!(role_tag(KvRole::Member { visibility: -2 }), "member--2");
    assert!(in_trash("/.trash"));
    assert!(in_trash("/.trash/owner/x"));
    assert!(!in_trash("/.trashcan"));
    assert!(!in_trash("/logins/.trash"));
}

#[test]
fn a_first_deletion_creates_trash_and_moves_the_item_into_it() {
    let store = account();
    let item = secret(&store, "/logins/github.com", 1);
    let plan = trash_plan(std::slice::from_ref(&item), &item, NOW).unwrap();
    let destination = format!("/.trash/owner/{STAMP}~%2Flogins%2Fgithub.com");
    assert_eq!(plan.path, destination);
    assert_eq!(
        operations(&plan.steps),
        vec![
            mkdir(&store, "/.trash", KvRole::Owner, KvRole::Owner, false),
            mkdir(&store, "/.trash/owner", KvRole::Owner, KvRole::Owner, false),
            moved(&item, &destination),
        ]
    );
    assert_eq!(
        plan.steps.iter().map(|step| step.kind).collect::<Vec<_>>(),
        vec![
            MutationKind::Create,
            MutationKind::Create,
            MutationKind::Guarded
        ]
    );

    let items = vec![
        item.clone(),
        folder(&store, "/.trash", KvRole::Owner, 2),
        folder(&store, "/.trash/owner", KvRole::Owner, 3),
    ];
    let plan = trash_plan(&items, &item, NOW).unwrap();
    assert_eq!(operations(&plan.steps), vec![moved(&item, &destination)]);
}

#[test]
fn team_items_go_to_the_trash_folder_of_their_own_read_role() {
    let store = team();
    let member = KvRole::Member { visibility: 0 };
    let item = entry(
        &store,
        "/wifi/guest",
        "small-file",
        member,
        KvRole::Admin,
        1,
    );
    let plan = trash_plan(std::slice::from_ref(&item), &item, NOW).unwrap();
    let low = KvRole::Member {
        visibility: -0x4000,
    };
    let destination = format!("/.trash/member-0/{STAMP}~%2Fwifi%2Fguest");
    assert_eq!(
        operations(&plan.steps),
        vec![
            mkdir(&store, "/.trash", low, low, false),
            mkdir(&store, "/.trash/member-0", member, member, false),
            moved(&item, &destination),
        ]
    );

    // A folder is filed by its own directory read role.
    let tax = folder(&store, "/tax-2024", KvRole::Owner, 2);
    let items = vec![
        tax.clone(),
        folder(&store, "/.trash", low, 3),
        folder(&store, "/.trash/member-0", member, 4),
    ];
    let plan = trash_plan(&items, &tax, NOW).unwrap();
    let destination = format!("/.trash/owner/{STAMP}~%2Ftax-2024");
    assert_eq!(
        operations(&plan.steps),
        vec![
            mkdir(&store, "/.trash/owner", KvRole::Owner, KvRole::Owner, false),
            moved(&tax, &destination),
        ]
    );
}

#[test]
fn a_role_folder_with_other_read_access_is_never_used() {
    let store = team();
    let item = secret(&store, "/lease.pdf", 1);
    let member = KvRole::Member { visibility: 0 };
    let items = vec![
        item.clone(),
        folder(&store, "/.trash", member, 2),
        folder(&store, "/.trash/owner", member, 3),
    ];
    let plan = trash_plan(&items, &item, NOW).unwrap();
    let destination = format!("/.trash/owner.2/{STAMP}~%2Flease.pdf");
    assert_eq!(
        operations(&plan.steps),
        vec![
            mkdir(
                &store,
                "/.trash/owner.2",
                KvRole::Owner,
                KvRole::Owner,
                false
            ),
            moved(&item, &destination),
        ]
    );
}

#[test]
fn trash_refuses_its_own_entries_and_a_top_level_file_named_trash() {
    let store = account();
    let inside = secret(&store, &format!("/.trash/owner/{STAMP}~%2Fa"), 1);
    assert_eq!(
        trash_plan(std::slice::from_ref(&inside), &inside, NOW)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let root = folder(&store, "/.trash", KvRole::Owner, 2);
    assert!(trash_plan(std::slice::from_ref(&root), &root, NOW).is_err());
    let item = secret(&store, "/a", 3);
    let blocker = secret(&store, "/.trash", 4);
    assert_eq!(
        trash_plan(&[item.clone(), blocker], &item, NOW)
            .unwrap_err()
            .code,
        "invalid-request"
    );
}

#[test]
fn long_paths_record_their_origin_before_the_move() {
    let store = account();
    let path = format!("/notes/{}", "n".repeat(240));
    let item = secret(&store, &path, 1);
    let items = vec![
        item.clone(),
        folder(&store, "/.trash", KvRole::Owner, 2),
        folder(&store, "/.trash/owner", KvRole::Owner, 3),
    ];
    let plan = trash_plan(&items, &item, NOW).unwrap();
    let (name, origin) = trash_entry_name(STAMP, &path);
    assert_eq!(plan.path, format!("/.trash/owner/{name}"));
    assert_eq!(
        operations(&plan.steps),
        vec![
            Operation::PutKv {
                store: kv(&store),
                path: format!("/.trash/owner/{}", origin.unwrap()),
                content: path.as_bytes().to_vec(),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
                precondition: KvPrecondition::Create,
                mkdir_p: false,
            },
            moved(&item, &plan.path),
        ]
    );
}

#[test]
fn restore_moves_back_creating_a_missing_folder_with_the_entry_roles() {
    let store = team();
    let member = KvRole::Member { visibility: 0 };
    let trashed = entry(
        &store,
        &format!("/.trash/member-0/{STAMP}~%2Fwifi%2Fguest"),
        "small-file",
        member,
        KvRole::Admin,
        1,
    );
    let plan = restore_plan(std::slice::from_ref(&trashed), &trashed, "/wifi/guest").unwrap();
    assert_eq!(plan.path, "/wifi/guest");
    assert_eq!(
        operations(&plan.steps),
        vec![
            mkdir(&store, "/wifi", member, KvRole::Admin, true),
            moved(&trashed, "/wifi/guest"),
        ]
    );

    let wifi = folder(&store, "/wifi", member, 2);
    let occupant = secret(&store, "/wifi/guest", 3);
    let items = vec![trashed.clone(), wifi.clone(), occupant];
    assert_eq!(
        restore_plan(&items, &trashed, "/wifi/guest")
            .unwrap_err()
            .code,
        "already-exists"
    );
    let plan = restore_plan(&items, &trashed, "/wifi/guest (restored)").unwrap();
    assert_eq!(
        operations(&plan.steps),
        vec![moved(&trashed, "/wifi/guest (restored)")]
    );
    for destination in ["/.trash/x", "/.trash", "relative", "/", "/wifi/"] {
        assert_eq!(
            restore_plan(&items, &trashed, destination)
                .unwrap_err()
                .code,
            "invalid-request",
            "{destination}"
        );
    }
}

#[test]
fn restore_accepts_only_entries_directly_in_a_role_folder() {
    let store = account();
    let foreign = secret(&store, "/.trash/owner/github.com", 1);
    let plan = restore_plan(std::slice::from_ref(&foreign), &foreign, "/github.com").unwrap();
    assert_eq!(
        operations(&plan.steps),
        vec![moved(&foreign, "/github.com")]
    );
    for path in [
        "/logins/github.com",
        "/.trash/owner",
        &format!("/.trash/owner/{STAMP}~#{}.origin", "0".repeat(32)),
        &format!("/.trash/owner/{STAMP}~%2Fa/child"),
    ] {
        let item = secret(&store, path, 1);
        assert_eq!(
            restore_plan(std::slice::from_ref(&item), &item, "/a")
                .unwrap_err()
                .code,
            "invalid-request",
            "{path}"
        );
    }
}

#[test]
fn restoring_or_deleting_a_hashed_entry_removes_its_origin_file() {
    let store = account();
    let path = format!("/{}", "n".repeat(250));
    let (name, origin) = trash_entry_name(STAMP, &path);
    let trashed = secret(&store, &format!("/.trash/owner/{name}"), 1);
    let origin = secret(&store, &format!("/.trash/owner/{}", origin.unwrap()), 2);
    let items = vec![trashed.clone(), origin.clone()];
    let plan = restore_plan(&items, &trashed, &path).unwrap();
    assert_eq!(
        operations(&plan.steps),
        vec![moved(&trashed, &path), removed(&origin, false)]
    );
    let plan = removal_plan(&items, &trashed).unwrap();
    assert_eq!(
        operations(&plan.steps),
        vec![removed(&trashed, false), removed(&origin, false)]
    );
}

#[test]
fn permanent_removal_is_limited_to_trash_and_recursive_for_folders() {
    let store = account();
    let item = secret(&store, "/logins/github.com", 1);
    assert_eq!(
        removal_plan(std::slice::from_ref(&item), &item)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    let root = folder(&store, "/.trash", KvRole::Owner, 2);
    assert!(removal_plan(std::slice::from_ref(&root), &root).is_err());
    let trashed = folder(
        &store,
        &format!("/.trash/owner/{STAMP}~%2Ftax-2024"),
        KvRole::Owner,
        3,
    );
    let plan = removal_plan(std::slice::from_ref(&trashed), &trashed).unwrap();
    assert_eq!(operations(&plan.steps), vec![removed(&trashed, true)]);
    let file = secret(&store, &format!("/.trash/owner/{STAMP}~%2Fa"), 4);
    let plan = removal_plan(std::slice::from_ref(&file), &file).unwrap();
    assert_eq!(operations(&plan.steps), vec![removed(&file, false)]);
}

struct ScriptedTransport {
    calls: Mutex<Vec<Operation>>,
    fail_at: usize,
    code: foks_agent_proto::ErrorCode,
}

impl ScriptedTransport {
    fn failing_at(fail_at: usize) -> Self {
        Self::failing_with(fail_at, foks_agent_proto::ErrorCode::Conflict)
    }

    fn failing_with(fail_at: usize, code: foks_agent_proto::ErrorCode) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail_at,
            code,
        }
    }
}

impl foks_desktop::AgentTransport for ScriptedTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        let mut calls = self.calls.lock().unwrap();
        calls.push(operation);
        if calls.len() == self.fail_at {
            return Err(foks_desktop::AgentError::Protocol {
                code: self.code,
                message: "precondition failed".to_owned(),
                fields: Box::default(),
            });
        }
        Ok(serde_json::Value::Null)
    }
}

#[test]
fn plans_stop_at_the_first_failed_write_and_name_the_step() {
    let store = account();
    let item = secret(&store, "/a", 1);
    let plan = trash_plan(std::slice::from_ref(&item), &item, NOW).unwrap();
    let transport = ScriptedTransport::failing_at(2);
    let error = execute_trash_plan(&transport, plan.steps).unwrap_err();
    assert_eq!(error.code, "already-exists");
    assert!(error
        .message
        .starts_with("The Trash folder for this item's read access could not be created."));
    assert_eq!(transport.calls.lock().unwrap().len(), 2);

    let plan = trash_plan(std::slice::from_ref(&item), &item, NOW).unwrap();
    let transport = ScriptedTransport::failing_at(3);
    let error = execute_trash_plan(&transport, plan.steps).unwrap_err();
    assert_eq!(error.code, "conflict");
    assert_eq!(error.message, "precondition failed");

    let plan = trash_plan(std::slice::from_ref(&item), &item, NOW).unwrap();
    let transport = ScriptedTransport::failing_at(0);
    assert!(!execute_trash_plan(&transport, plan.steps).unwrap());
    assert_eq!(transport.calls.lock().unwrap().len(), 3);
}

#[test]
fn entries_are_filed_by_the_most_restrictive_read_role_on_their_path() {
    let store = team();
    let member = KvRole::Member { visibility: 0 };
    let low = KvRole::Member {
        visibility: -0x4000,
    };
    let payroll = folder(&store, "/Payroll", KvRole::Admin, 1);
    let login = entry(
        &store,
        "/Payroll/bank-login",
        "small-file",
        member,
        member,
        2,
    );
    let items = vec![
        payroll.clone(),
        login.clone(),
        folder(&store, "/.trash", low, 3),
        folder(&store, "/.trash/member-0", member, 4),
    ];
    assert_eq!(path_read_role(&items, &login), KvRole::Admin);
    let plan = trash_plan(&items, &login, NOW).unwrap();
    let destination = format!("/.trash/admin/{STAMP}~%2FPayroll%2Fbank-login");
    assert_eq!(plan.path, destination);
    assert_eq!(
        operations(&plan.steps),
        vec![
            mkdir(&store, "/.trash/admin", KvRole::Admin, KvRole::Admin, false),
            moved(&login, &destination),
        ]
    );

    // A folder beneath a more restrictive one, and a hashed entry's
    // `.origin` file, are filed and sealed the same way.
    let shared = folder(&store, "/Payroll/shared", member, 5);
    let long = format!("/Payroll/shared/{}", "n".repeat(240));
    let note = entry(&store, &long, "small-file", member, member, 6);
    let items = vec![payroll, shared.clone(), note.clone()];
    let plan = trash_plan(&items, &shared, NOW).unwrap();
    assert_eq!(
        plan.path,
        format!("/.trash/admin/{STAMP}~%2FPayroll%2Fshared")
    );
    let plan = trash_plan(&items, &note, NOW).unwrap();
    let (_, origin) = trash_entry_name(STAMP, &long);
    assert_eq!(
        plan.steps[2].operation,
        Operation::PutKv {
            store: kv(&store),
            path: format!("/.trash/admin/{}", origin.unwrap()),
            content: long.as_bytes().to_vec(),
            read_role: KvRole::Admin,
            write_role: member,
            precondition: KvPrecondition::Create,
            mkdir_p: false,
        }
    );

    // A less restrictive ancestor leaves the entry's own role in charge.
    let open = folder(&store, "/open", member, 7);
    let deed = entry(
        &store,
        "/open/deed",
        "small-file",
        KvRole::Owner,
        KvRole::Owner,
        8,
    );
    assert_eq!(path_read_role(&[open, deed.clone()], &deed), KvRole::Owner);
}

#[test]
fn restore_creates_a_missing_folder_readable_only_under_its_trash_folder_role() {
    let store = team();
    let member = KvRole::Member { visibility: 0 };
    let trashed = entry(
        &store,
        &format!("/.trash/admin/{STAMP}~%2FPayroll%2Fbank-login"),
        "small-file",
        member,
        member,
        1,
    );
    let items = vec![
        trashed.clone(),
        folder(&store, "/.trash/admin", KvRole::Admin, 2),
    ];
    let plan = restore_plan(&items, &trashed, "/Payroll/bank-login").unwrap();
    assert_eq!(
        operations(&plan.steps),
        vec![
            mkdir(&store, "/Payroll", KvRole::Admin, KvRole::Admin, true),
            moved(&trashed, "/Payroll/bank-login"),
        ]
    );
}

#[test]
fn a_name_already_in_trash_is_reported_apart_from_a_changed_item() {
    let store = account();
    let item = secret(&store, "/a", 1);
    let items = vec![
        item.clone(),
        folder(&store, "/.trash", KvRole::Owner, 2),
        folder(&store, "/.trash/owner", KvRole::Owner, 3),
        secret(&store, &format!("/.trash/owner/{STAMP}~%2Fa"), 4),
    ];
    assert_eq!(
        trash_plan(&items, &item, NOW).unwrap_err().code,
        "trash-name-taken"
    );
}

#[test]
fn a_team_member_who_cannot_create_trash_is_told_so_by_code() {
    let store = team();
    let item = entry(
        &store,
        "/wifi/guest",
        "small-file",
        KvRole::Member { visibility: 0 },
        KvRole::Member { visibility: 0 },
        1,
    );
    let refused = |code| {
        let plan = trash_plan(std::slice::from_ref(&item), &item, NOW).unwrap();
        let transport = ScriptedTransport::failing_with(1, code);
        execute_trash_plan(&transport, plan.steps).unwrap_err()
    };
    let error = refused(foks_agent_proto::ErrorCode::OperationFailed);
    assert_eq!(error.code, TRASH_UNAVAILABLE);
    assert!(error
        .message
        .starts_with("Trash could not be created at the top level of this vault."));
    // A transient failure or a concurrent creation keeps its own code.
    assert_eq!(
        refused(foks_agent_proto::ErrorCode::ServerUnavailable).code,
        "server-unavailable"
    );
    assert_eq!(
        refused(foks_agent_proto::ErrorCode::Conflict).code,
        "already-exists"
    );

    // In an account, the owner always can, so the code is never replaced.
    let own = secret(&account(), "/a", 1);
    let plan = trash_plan(std::slice::from_ref(&own), &own, NOW).unwrap();
    let transport =
        ScriptedTransport::failing_with(1, foks_agent_proto::ErrorCode::OperationFailed);
    assert_eq!(
        execute_trash_plan(&transport, plan.steps).unwrap_err().code,
        "operation-failed"
    );
}

#[test]
fn removal_outside_trash_is_limited_to_files_in_a_team_without_trash() {
    let store = team();
    let item = secret(&store, "/wifi/guest", 1);
    let plan = removal_without_trash_plan(std::slice::from_ref(&item), &item, true).unwrap();
    assert_eq!(operations(&plan.steps), vec![removed(&item, false)]);

    let low = KvRole::Member {
        visibility: -0x4000,
    };
    let with_trash = vec![item.clone(), folder(&store, "/.trash", low, 2)];
    assert_eq!(
        removal_without_trash_plan(&with_trash, &item, true)
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert_eq!(
        removal_without_trash_plan(std::slice::from_ref(&item), &item, false)
            .unwrap_err()
            .code,
        "catalog-required"
    );
    let wifi = folder(&store, "/wifi", low, 3);
    assert!(removal_without_trash_plan(std::slice::from_ref(&wifi), &wifi, true).is_err());
    let own = secret(&account(), "/a", 4);
    assert!(removal_without_trash_plan(std::slice::from_ref(&own), &own, true).is_err());
    let trashed = secret(&store, &format!("/.trash/owner/{STAMP}~%2Fa"), 5);
    assert!(removal_without_trash_plan(std::slice::from_ref(&trashed), &trashed, true).is_err());
}

#[test]
fn emptying_trash_plans_every_entry_from_one_catalog_read() {
    let store = account();
    let long = format!("/{}", "n".repeat(250));
    let (hashed_name, origin_name) = trash_entry_name(STAMP, &long);
    let hashed = secret(&store, &format!("/.trash/owner/{hashed_name}"), 1);
    let origin = secret(
        &store,
        &format!("/.trash/owner/{}", origin_name.unwrap()),
        2,
    );
    let tax = folder(
        &store,
        &format!("/.trash/owner/{STAMP}~%2Ftax-2024"),
        KvRole::Owner,
        3,
    );
    let file = secret(&store, &format!("/.trash/owner/{STAMP}~%2Fa"), 4);
    let orphan = secret(
        &store,
        &format!("/.trash/owner/{STAMP}~#{}.origin", "f".repeat(32)),
        5,
    );
    let items = vec![
        hashed.clone(),
        origin.clone(),
        tax.clone(),
        file.clone(),
        orphan.clone(),
    ];
    let entries = vec![hashed.clone(), tax.clone(), file.clone()];
    let plan = empty_trash_plan(&items, &entries, true).unwrap();
    assert_eq!(
        operations(&plan.steps),
        vec![
            removed(&hashed, false),
            removed(&tax, true),
            removed(&file, false),
            removed(&origin, false),
            removed(&orphan, false),
        ]
    );
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.cleanup)
            .collect::<Vec<_>>(),
        vec![false, false, false, true, true]
    );

    // An incomplete catalog may be missing an `.origin` file's entry, so no
    // `.origin` file is taken for an orphan.
    let plan = empty_trash_plan(&items, &entries, false).unwrap();
    assert_eq!(plan.steps.len(), 4);
    // Every entry must be in Trash.
    let outside = secret(&store, "/a", 6);
    assert!(empty_trash_plan(&items, &[file, outside], true).is_err());
}

#[test]
fn a_failed_origin_cleanup_leaves_the_command_applied() {
    let store = account();
    let long = format!("/{}", "n".repeat(250));
    let (name, origin) = trash_entry_name(STAMP, &long);
    let trashed = secret(&store, &format!("/.trash/owner/{name}"), 1);
    let origin = secret(&store, &format!("/.trash/owner/{}", origin.unwrap()), 2);
    let items = vec![trashed.clone(), origin];
    let plan = restore_plan(&items, &trashed, &long).unwrap();
    let transport = ScriptedTransport::failing_at(2);
    assert!(!execute_trash_plan(&transport, plan.steps).unwrap());

    let plan = removal_plan(&items, &trashed).unwrap();
    let transport =
        ScriptedTransport::failing_with(2, foks_agent_proto::ErrorCode::DeadlineExceeded);
    assert!(execute_trash_plan(&transport, plan.steps).unwrap());
}
