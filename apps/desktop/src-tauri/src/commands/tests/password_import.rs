use super::support::{account_ref, team_ref};
use crate::agent::AgentError;
use crate::commands::password_import::{
    csv_records, destination_folder, import_path, login_text, parse_export, path_component,
    write_entries, ImportEntry, ImportOutcomeDto, ImportTarget, PasswordImports, SkippedRowDto,
    MAXIMUM_IMPORT_ENTRIES,
};
use foks_agent_proto::{KvPrecondition, KvRole, KvStoreRef, Operation};
use foks_client_app::{login_field, LoginField};
use foks_desktop::CatalogStoreRef;
use std::sync::Mutex;
use zeroize::Zeroizing;

fn fields(records: &[Vec<Zeroizing<String>>]) -> Vec<Vec<&str>> {
    records
        .iter()
        .map(|record| record.iter().map(|field| field.as_str()).collect())
        .collect()
}

fn names(entries: &[ImportEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| {
            let mut path = entry.folder.clone();
            path.push(entry.name.clone());
            path.join("/")
        })
        .collect()
}

#[test]
fn csv_records_follow_rfc_4180_quoting_and_line_endings() {
    let text = "a,\"b,c\",\"say \"\"hi\"\"\"\r\n\"two\nlines\",,x\n\nlast,\n";
    assert_eq!(
        fields(&csv_records(text, b',').unwrap()),
        vec![
            vec!["a", "b,c", "say \"hi\""],
            vec!["two\nlines", "", "x"],
            vec![""],
            vec!["last", ""],
        ]
    );
    // No final line break, a delimiter at the very end, and a lone `\r`.
    assert_eq!(
        fields(&csv_records("a;b\rc;", b';').unwrap()),
        vec![vec!["a", "b"], vec!["c", ""]]
    );
    assert!(csv_records("", b',').unwrap().is_empty());

    let open = csv_records("name,password\n\"github,x\n", b',')
        .err()
        .unwrap();
    assert_eq!(open.code, "import-file");
    assert!(open.message.contains("Row 2"), "{}", open.message);
    let trailing = csv_records("\"a\"b,c\n", b',').err().unwrap();
    assert_eq!(trailing.code, "import-file");
    assert!(trailing.message.contains("Row 1"), "{}", trailing.message);
}

#[test]
fn path_components_are_encoded_as_the_files_list_decodes_them() {
    assert_eq!(path_component("  GitHub  ").as_deref(), Some("GitHub"));
    assert_eq!(
        path_component("50% off / sale").as_deref(),
        Some("50%25 off %2F sale")
    );
    assert_eq!(path_component("a\tb\u{7}c").as_deref(), Some("a bc"));
    for unusable in ["", "   ", ".", "..", "\u{1}"] {
        assert_eq!(path_component(unusable), None, "{unusable:?}");
    }
    // Cut at a character boundary, below the 255-byte component limit.
    let long = path_component(&"é".repeat(200)).unwrap();
    assert_eq!(long.len(), 240);
    assert!(long.chars().all(|character| character == 'é'));
    let encoded = path_component(&"%".repeat(100)).unwrap();
    assert_eq!(encoded, "%25".repeat(80));
}

#[test]
fn a_login_is_written_in_the_layout_the_new_password_sheet_writes() {
    let text = login_text("satoshi", " pass word ", "https://github.com/login", "");
    assert_eq!(
        text.as_str(),
        "user: satoshi\npassword:  pass word \nurl: https://github.com/login"
    );
    assert_eq!(
        login_field(&text, LoginField::Password),
        Some(" pass word ")
    );

    // Each line of the notes becomes a field line, so the text stays a login.
    let text = login_text("", "hunter2", "", "first\r\n\r\n  indented\rlast\n\n");
    assert_eq!(
        text.as_str(),
        "user: \npassword: hunter2\nurl: \nnote: first\nnote:\nnote:   indented\nnote: last"
    );
    for line in text.split('\n') {
        let (key, _) = line.split_once(':').expect("a field line");
        assert!(!key.is_empty() && !key.contains(char::is_whitespace));
    }
    assert_eq!(login_field(&text, LoginField::Password), Some("hunter2"));
    assert_eq!(login_field(&text, LoginField::Username), None);
    assert_eq!(
        login_text("u", "p", "w", " \n ").as_str(),
        "user: u\npassword: p\nurl: w"
    );
}

const BITWARDEN: &str = "\u{feff}folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
Work/Email,,login,Mail,\"line one\nline two\",,0,https://mail.example.com,ana,s3cret,JBSWY3DP\n\
,,note,Wi-Fi codes,secret note,,0,,,,\n\
,1,login,GitHub,,,0,https://github.com,ana,gh-pass,\n\
,,login,GitHub,,,0,https://github.com,ana-work,gh-work,\n\
,,login,Broken,,,0,https://x.example,ana,\"two\nlines\",\n\
,,login,,,,0,,,,\n\
,,,,,,,,,,\n";

#[test]
fn a_bitwarden_export_imports_its_logins_and_names_what_it_leaves_out() {
    let parsed = parse_export(BITWARDEN.as_bytes()).unwrap();
    assert_eq!(
        names(&parsed.entries),
        vec!["Work/Email/Mail", "GitHub", "GitHub (2)"]
    );
    let mail = &parsed.entries[0];
    assert_eq!(
        mail.content.as_str(),
        "user: ana\npassword: s3cret\nurl: https://mail.example.com\nnote: line one\nnote: line two"
    );
    assert_eq!(
        login_field(&parsed.entries[2].content, LoginField::Username),
        Some("ana-work")
    );
    assert_eq!(
        parsed.skipped,
        vec![
            SkippedRowDto {
                row: 3,
                name: Some("Wi-Fi codes".to_owned()),
                reason: "A note entry, not a login.".to_owned(),
            },
            SkippedRowDto {
                row: 6,
                name: Some("Broken".to_owned()),
                reason: "Its password contains a line break, which a login field cannot hold."
                    .to_owned(),
            },
            SkippedRowDto {
                row: 7,
                name: None,
                reason: "It has no user name, password or website.".to_owned(),
            },
        ]
    );
    assert_eq!(
        parsed.ignored_columns,
        vec!["favorite", "fields", "reprompt", "login_totp"]
    );
}

#[test]
fn keepass_firefox_and_one_password_exports_are_read_by_their_headers() {
    let keepassxc = "\"Group\",\"Title\",\"Username\",\"Password\",\"URL\",\"Notes\",\"TOTP\",\"Icon\",\"Last Modified\",\"Created\"\n\
\"Root\",\"Bank\",\"ana\",\"pw1\",\"https://bank.example\",\"\",\"\",\"0\",\"\",\"\"\n\
\"Root/Internet/Shops\",\"Shop\",\"ana\",\"pw2\",\"\",\"\",\"\",\"0\",\"\",\"\"\n";
    let parsed = parse_export(keepassxc.as_bytes()).unwrap();
    assert_eq!(names(&parsed.entries), vec!["Bank", "Internet/Shops/Shop"]);
    assert_eq!(
        parsed.ignored_columns,
        vec!["TOTP", "Icon", "Last Modified", "Created"]
    );

    // Firefox has no title column: a login is named by its website's host,
    // else its user name.
    let firefox = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\n\
\"https://accounts.example.com\",\"ana\",\"pw\",,\"\",\"{1}\",\"1\",\"1\",\"1\"\n\
\"accounts.example.com\",\"bob\",\"pw\",,\"\",\"{2}\",\"1\",\"1\",\"1\"\n\
\"\",\"carol\",\"pw\",,\"\",\"{3}\",\"1\",\"1\",\"1\"\n\
\"\",\"\",\"pw\",,\"\",\"{4}\",\"1\",\"1\",\"1\"\n";
    let parsed = parse_export(firefox.as_bytes()).unwrap();
    assert_eq!(
        names(&parsed.entries),
        vec![
            "accounts.example.com",
            "accounts.example.com (2)",
            "carol",
            "Login"
        ]
    );

    let one_password = "Title,Url,Username,Password,OTPAuth,Favorite,Archived,Tags,Notes\r\n\
Router,http://192.168.1.1,admin,\"a,b\"\"c\",,false,false,home,Reset: hold 10 s\r\n";
    let parsed = parse_export(one_password.as_bytes()).unwrap();
    assert_eq!(names(&parsed.entries), vec!["Router"]);
    assert_eq!(
        parsed.entries[0].content.as_str(),
        "user: admin\npassword: a,b\"c\nurl: http://192.168.1.1\nnote: Reset: hold 10 s"
    );

    // A semicolon-separated export, as some locales write.
    let keepass =
        "Account;Login Name;Password;Web Site;Comments\nMail;ana;pw;https://mail.example;\n";
    let parsed = parse_export(keepass.as_bytes()).unwrap();
    assert_eq!(names(&parsed.entries), vec!["Mail"]);
}

#[test]
fn a_login_never_takes_the_name_of_a_folder_other_logins_are_written_in() {
    // A login named like a folder that other rows use, before and after
    // those rows: written as a file there, it would leave no folder for
    // them, and their writes would fail.
    let text = "folder,name,login_username,login_password\n\
,Work,ana,pw1\n\
Work,Mail,ana,pw2\n\
Work/Email,Inbox,ana,pw3\n\
Work,Email,ana,pw4\n\
,Work,bob,pw5\n";
    let parsed = parse_export(text.as_bytes()).unwrap();
    assert_eq!(
        names(&parsed.entries),
        vec![
            "Work (2)",
            "Work/Mail",
            "Work/Email/Inbox",
            "Work/Email (2)",
            "Work (3)"
        ]
    );
}

#[test]
fn lastpass_secure_notes_are_left_out_and_bitwarden_uris_are_split() {
    let lastpass = "url,username,password,totp,extra,name,grouping,fav\n\
http://sn,,,,\"NoteType:Credit Card\nNumber:4111\",Visa,Cards,0\n\
https://mail.example,ana,pw,,,Mail,Email,0\n";
    let parsed = parse_export(lastpass.as_bytes()).unwrap();
    assert_eq!(names(&parsed.entries), vec!["Email/Mail"]);
    assert_eq!(
        parsed.skipped,
        vec![SkippedRowDto {
            row: 2,
            name: Some("Visa".to_owned()),
            reason: "A secure note, not a login.".to_owned(),
        }]
    );

    // Bitwarden joins a login's URIs with commas: the first is its website
    // and the others are kept as notes. One URL with a comma stays whole.
    let bitwarden = "name,login_uri,login_username,login_password,notes\n\
Shop,\"https://shop.example, https://m.shop.example\",ana,pw,keep this\n\
,\"https://search.example/?q=a,b\",ana,pw,\n";
    let parsed = parse_export(bitwarden.as_bytes()).unwrap();
    assert_eq!(names(&parsed.entries), vec!["Shop", "search.example"]);
    assert_eq!(
        parsed.entries[0].content.as_str(),
        "user: ana\npassword: pw\nurl: https://shop.example\nnote: https://m.shop.example\nnote: keep this"
    );
    assert_eq!(
        login_field(&parsed.entries[1].content, LoginField::Url),
        Some("https://search.example/?q=a,b")
    );
}

#[test]
fn a_file_without_a_password_column_or_in_another_encoding_is_refused() {
    for bytes in [
        b"name,url,username\ngithub,https://github.com,ana\n".as_slice(),
        b"",
        &[0xff, 0xfe, b'n', 0, b'a', 0],
    ] {
        let error = parse_export(bytes).err().unwrap();
        assert_eq!(error.code, "import-file", "{bytes:?}");
        assert!(!error.retryable);
    }
}

#[test]
fn a_file_with_too_many_logins_is_refused() {
    let mut text = String::from("name,password\n");
    for index in 0..=MAXIMUM_IMPORT_ENTRIES {
        text.push_str(&format!("site{index},pw\n"));
    }
    let error = parse_export(text.as_bytes()).err().unwrap();
    assert_eq!(error.code, "import-file");
    text.truncate(text.trim_end().rfind('\n').unwrap() + 1);
    assert_eq!(
        parse_export(text.as_bytes()).unwrap().entries.len(),
        MAXIMUM_IMPORT_ENTRIES
    );
}

#[test]
fn the_preview_names_entries_and_the_commit_takes_only_the_held_import() {
    let imports = PasswordImports::default();
    let mut text = String::from(BITWARDEN);
    text.push_str(&format!(
        ",,login,Long,\"{}\",,0,,ana,pw,\n",
        "x".repeat(2100)
    ));
    let preview = imports
        .hold(
            4,
            "export.csv".to_owned(),
            parse_export(text.as_bytes()).unwrap(),
        )
        .unwrap();
    assert_eq!(preview.file_name, "export.csv");
    assert_eq!(preview.id.len(), 32);
    let listed: Vec<_> = preview
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.folder.as_str(), entry.document))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("Mail", "Work/Email", false),
            ("GitHub", "", false),
            ("GitHub (2)", "", false),
            ("Long", "", true),
        ]
    );
    // The preview carries names, folders and reasons, and no field value.
    let json = serde_json::to_string(&preview).unwrap();
    for value in ["s3cret", "gh-pass", "ana", "mail.example.com", "line one"] {
        assert!(!json.contains(value), "{value} reached the preview");
    }

    let selected = imports.selected(&preview.id, 4, &[2, 0]).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|(index, entry)| (*index, entry.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(2, "GitHub (2)"), (0, "Mail")]
    );
    for (id, generation) in [("other", 4), (preview.id.as_str(), 5)] {
        assert_eq!(
            imports.selected(id, generation, &[0]).err().unwrap().code,
            "import-expired"
        );
    }
    for indexes in [&[][..], &[4], &[1, 1]] {
        assert_eq!(
            imports
                .selected(&preview.id, 4, indexes)
                .err()
                .unwrap()
                .code,
            "invalid-request",
            "{indexes:?}"
        );
    }

    // Another id leaves the import held; its own id or the lock drops it.
    imports.discard(Some("other"));
    assert!(imports.selected(&preview.id, 4, &[0]).is_ok());
    imports.discard(Some(&preview.id));
    assert_eq!(
        imports.selected(&preview.id, 4, &[0]).err().unwrap().code,
        "import-expired"
    );
    let preview = imports
        .hold(
            4,
            String::new(),
            parse_export(BITWARDEN.as_bytes()).unwrap(),
        )
        .unwrap();
    imports.discard(None);
    assert!(imports.selected(&preview.id, 4, &[0]).is_err());
}

#[test]
fn the_destination_is_an_absolute_folder_outside_trash() {
    for folder in ["/", "/logins", "/imported/2024"] {
        assert_eq!(destination_folder(folder.to_owned()).unwrap(), folder);
    }
    for folder in [
        "",
        "logins",
        "/logins/",
        "//a",
        "/a/../b",
        "/.trash",
        "/.trash/x",
    ] {
        assert_eq!(
            destination_folder(folder.to_owned()).err().unwrap().code,
            "invalid-request",
            "{folder}"
        );
    }
    let entry = ImportEntry {
        folder: vec!["Work".to_owned(), "Email".to_owned()],
        name: "Mail".to_owned(),
        content: Zeroizing::new(String::new()),
    };
    assert_eq!(import_path("/", &entry), "/Work/Email/Mail");
    assert_eq!(import_path("/logins", &entry), "/logins/Work/Email/Mail");
}

/// Accepts writes and fails the calls named in `failures`, by their
/// position, with the given error.
struct ImportTransport {
    writes: Mutex<Vec<Operation>>,
    failures: Vec<(usize, fn() -> foks_desktop::AgentError)>,
}

impl ImportTransport {
    fn new(failures: Vec<(usize, fn() -> foks_desktop::AgentError)>) -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            failures,
        }
    }
}

impl foks_desktop::AgentTransport for ImportTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        let mut writes = self.writes.lock().unwrap();
        writes.push(operation);
        match self
            .failures
            .iter()
            .find(|(position, _)| *position == writes.len())
        {
            Some((_, error)) => Err(error()),
            None => Ok(serde_json::Value::Null),
        }
    }
}

fn occupied() -> foks_desktop::AgentError {
    foks_desktop::AgentError::Protocol {
        code: foks_agent_proto::ErrorCode::Conflict,
        message: "precondition failed".to_owned(),
        fields: Box::default(),
    }
}

fn lost() -> foks_desktop::AgentError {
    foks_desktop::AgentError::Ambiguous("reply lost".to_owned())
}

fn refused() -> foks_desktop::AgentError {
    foks_desktop::AgentError::Protocol {
        code: foks_agent_proto::ErrorCode::InvalidRequest,
        message: "denied".to_owned(),
        fields: Box::default(),
    }
}

fn team_target() -> ImportTarget {
    ImportTarget {
        store: CatalogStoreRef::Team(team_ref("work.example", "personal", "engineering")),
        folder: "/logins".to_owned(),
        read_role: KvRole::Member { visibility: 0 },
        write_role: KvRole::Admin,
    }
}

fn run(
    transport: &ImportTransport,
    target: &ImportTarget,
    permitted: &dyn Fn() -> Result<(), AgentError>,
) -> (Vec<ImportOutcomeDto>, Vec<ImportOutcomeDto>) {
    let parsed = parse_export(BITWARDEN.as_bytes()).unwrap();
    let entries = parsed
        .entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| (index as u32, entry))
        .collect();
    let reported = Mutex::new(Vec::new());
    let outcomes = write_entries(transport, target, entries, permitted, &|outcome| {
        reported.lock().unwrap().push(outcome.clone())
    });
    (outcomes, reported.into_inner().unwrap())
}

fn statuses(outcomes: &[ImportOutcomeDto]) -> Vec<&'static str> {
    outcomes.iter().map(|outcome| outcome.status).collect()
}

#[test]
fn each_login_is_one_create_with_the_target_roles() {
    let transport = ImportTransport::new(Vec::new());
    let (outcomes, reported) = run(&transport, &team_target(), &|| Ok(()));
    assert_eq!(statuses(&outcomes), vec!["created"; 3]);
    assert_eq!(reported, outcomes);
    let writes = transport.writes.lock().unwrap();
    let Operation::PutKv {
        store,
        path,
        content,
        read_role,
        write_role,
        precondition,
        mkdir_p,
    } = &writes[0]
    else {
        panic!("a login is one PutKv: {:?}", writes[0]);
    };
    assert_eq!(
        *store,
        KvStoreRef::Team(team_ref("work.example", "personal", "engineering"))
    );
    assert_eq!(path, "/logins/Work/Email/Mail");
    assert!(content.starts_with(b"user: ana\npassword: s3cret\n"));
    assert_eq!(*read_role, KvRole::Member { visibility: 0 });
    assert_eq!(*write_role, KvRole::Admin);
    assert_eq!(*precondition, KvPrecondition::Create);
    assert!(*mkdir_p);
    let paths: Vec<_> = writes
        .iter()
        .map(|write| match write {
            Operation::PutKv { path, .. } => path.as_str(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        paths,
        vec![
            "/logins/Work/Email/Mail",
            "/logins/GitHub",
            "/logins/GitHub (2)"
        ]
    );

    let personal = ImportTarget {
        store: CatalogStoreRef::Account(account_ref("foks.example", "personal")),
        folder: "/".to_owned(),
        read_role: KvRole::Owner,
        write_role: KvRole::Owner,
    };
    let transport = ImportTransport::new(Vec::new());
    run(&transport, &personal, &|| Ok(()));
    assert!(matches!(
        &transport.writes.lock().unwrap()[1],
        Operation::PutKv { path, read_role: KvRole::Owner, .. } if path == "/GitHub"
    ));
}

#[test]
fn an_occupied_path_is_skipped_and_any_other_failure_stops_the_run() {
    let transport = ImportTransport::new(vec![(1, occupied)]);
    let (outcomes, _) = run(&transport, &team_target(), &|| Ok(()));
    assert_eq!(statuses(&outcomes), vec!["exists", "created", "created"]);
    assert_eq!(transport.writes.lock().unwrap().len(), 3);

    let transport = ImportTransport::new(vec![(2, refused)]);
    let (outcomes, reported) = run(&transport, &team_target(), &|| Ok(()));
    assert_eq!(
        statuses(&outcomes),
        vec!["created", "failed", "not-started"]
    );
    assert_eq!(reported, outcomes);
    assert_eq!(transport.writes.lock().unwrap().len(), 2);
    assert_eq!(
        outcomes[2].message.as_deref(),
        Some("Not started: an earlier login failed.")
    );

    let transport = ImportTransport::new(vec![(1, lost)]);
    let (outcomes, _) = run(&transport, &team_target(), &|| Ok(()));
    assert_eq!(
        statuses(&outcomes),
        vec!["uncertain", "not-started", "not-started"]
    );
    assert_eq!(transport.writes.lock().unwrap().len(), 1);
    assert_eq!(
        outcomes[1].message.as_deref(),
        Some("Not started: the outcome of an earlier login is unknown.")
    );
}

#[test]
fn a_lock_during_the_run_stops_it_before_the_next_write() {
    let transport = ImportTransport::new(Vec::new());
    let calls = Mutex::new(0);
    let (outcomes, _) = run(&transport, &team_target(), &|| {
        let mut calls = calls.lock().unwrap();
        *calls += 1;
        if *calls == 2 {
            Err(AgentError::new(
                "app-locked",
                "The application locked before the change was submitted.",
                false,
            ))
        } else {
            Ok(())
        }
    });
    assert_eq!(
        statuses(&outcomes),
        vec!["created", "failed", "not-started"]
    );
    assert_eq!(transport.writes.lock().unwrap().len(), 1);
}
