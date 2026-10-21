//! A team data write whose view token the server no longer holds activates one
//! replacement view and replays the refused request.

use std::cell::RefCell;
use std::sync::atomic::Ordering;

use crate::kv::TEAM_VIEW_REMINTS;
use crate::test_support::AccountFixture;
use crate::{DataWriteKind, DataWriteSpec, DataWriteStatus, SubmissionHandle};

thread_local! {
    static BEFORE_TEAM_WRITE: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

/// Runs the hook a test installed, after the team session is armed and
/// before its first request.
pub(super) fn before_team_write() {
    if let Some(hook) = BEFORE_TEAM_WRITE.with(|hook| hook.borrow_mut().take()) {
        hook();
    }
}

#[test]
fn team_data_write_replaces_a_reclaimed_view_once() {
    let f = AccountFixture::start();
    let database = f.environment.database_path().to_path_buf();
    let issued_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let body = b"written after the view was reclaimed".to_vec();
    let outcome = f.run(|s, vault, master| {
        s.create_account(
            "owner",
            "remintowner",
            "device",
            "owner@example.test",
            "",
            None,
            vault,
            master,
        )?;
        s.create_named_team("owner", "remintgroup", "remintteam", vault, master)?;
        let handle = SubmissionHandle::new(issued_at, [0x51; 16]);
        let spec = DataWriteSpec {
            kind: DataWriteKind::Put,
            path: "/reminted".into(),
            destination: None,
            team_selector: Some("remintteam".into()),
            overwrite: false,
            mkdir_p: false,
            recursive: false,
            body_length: body.len() as u64,
            body_hash: foks_crypto::kv_adapter_body_hash(&body),
        };
        s.prepare_data_write("owner", handle, spec, vault, master)?;
        // The write's view was activated while loading its context; the
        // server then reclaims every recorded view, as eviction or expiry
        // cleanup would, so the first KV request is refused with
        // TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR.
        BEFORE_TEAM_WRITE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let connection = rusqlite::Connection::open(&database).unwrap();
                let removed = connection
                    .execute("DELETE FROM team_view_tokens", [])
                    .unwrap();
                assert!(removed > 0, "no view token to reclaim");
            }));
        });
        TEAM_VIEW_REMINTS.store(0, Ordering::SeqCst);
        s.execute_data_write("owner", handle, &mut body.as_slice(), vault, master)
    });
    assert_eq!(outcome.status, DataWriteStatus::Committed);
    assert_eq!(TEAM_VIEW_REMINTS.load(Ordering::SeqCst), 1);
    let connection = rusqlite::Connection::open(f.environment.database_path()).unwrap();
    let tokens: i64 = connection
        .query_row("SELECT count(*) FROM team_view_tokens", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tokens, 1, "the replacement view is recorded");
}
