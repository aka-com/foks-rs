#![cfg(feature = "celld-proof")]
//! Exact native service oracle with a real issued certificate; no hosted route.
use foks_metadata_core::{
    native::NativeSql,
    personal::{authenticate_peer, read_personal_list, ListCursor, ListRequest, PersonalError},
};
use foks_proto::Role;
use foks_rpc::{KvAuth, KvListCursor, RpcStatus, DEFAULT_MAX_FRAME_LENGTH};
use foks_server::hosted_personal;
use foks_server_db::Clock;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use rusqlite::{params, Connection};
use rustls_pki_types::CertificateDer;
use std::{io::Cursor, sync::Arc};
struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(self.0)
    }
}
#[test]
fn issued_certificate_list_domain_matches_exact_native_service_and_corrupt_type_status() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "list-domain").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("listdomain", 0x76))
        .unwrap();
    let root = account.kv_projection[0].root_directory_id;
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .user_kv_write_session(
            &host,
            &account.credential,
            &account.authenticated.verified,
            &account.authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    for n in 0..16 {
        session
            .put_file(
                root,
                &format!("entry-{n}.txt"),
                &mut Cursor::new(b"real native optional body"),
                foks_client::KvWriteOptions {
                    read_role: Role::OWNER,
                    write_role: Role::OWNER,
                    overwrite: false,
                    expected_version: None,
                },
            )
            .unwrap();
    }
    drop(session);
    server.shutdown().unwrap();
    let now = environment.advance_clock(0);
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(now));
    let certificate = CertificateDer::from(account.credential.certificate_chain[0].as_slice());
    let peer = hosted_personal::peer_certificate(&certificate).unwrap();
    let reader = environment.read_database().unwrap();
    let c = Connection::open(environment.database_path()).unwrap();
    let compare = |parent: [u8; 16], cursor: KvListCursor, count: u64, load: bool| {
        let own_cursor = match &cursor {
            KvListCursor::None => ListCursor::Initial,
            KvListCursor::Mac(m) => ListCursor::Mac(*m),
            KvListCursor::Time(t) => ListCursor::Time(*t),
        };
        let request =
            foks_rpc::encode_kv_list_request(KvAuth::User, &parent, cursor, count, load).unwrap();
        let call =
            foks_rpc::read_call(&mut Cursor::new(request), DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let native =
            hosted_personal::native_list(&certificate, &reader, &clock, call.argument()).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let mut sql = NativeSql::new(&tx).unwrap();
        let credential = authenticate_peer(&mut sql, &peer, now).unwrap();
        let actual = read_personal_list(
            &mut sql,
            &credential,
            &ListRequest::new(parent, own_cursor, count, load),
            now,
            16 * 1024 * 1024,
        );
        tx.rollback().unwrap();
        match (native, actual) {
            (Ok(expected), Ok(actual)) => assert_eq!(actual, expected),
            (Err(RpcStatus::TransactionRetry), Err(PersonalError::Corrupt)) => {}
            (Err(RpcStatus::KvNoEnt), Err(PersonalError::Missing)) => {}
            (Err(RpcStatus::PermissionDenied(_)), Err(PersonalError::Permission)) => {}
            pair => panic!("native/domain mismatch: {pair:?}"),
        }
    };
    for count in [0, 1, 15, 16, 17, 4096, 4097] {
        for load in [false, true] {
            compare(root, KvListCursor::None, count, load);
            compare(root, KvListCursor::Time(0), count, load);
            compare(root, KvListCursor::Mac([0; 32]), count, load);
        }
    }
    compare(root, KvListCursor::Time(u64::MAX), 4096, true);
    compare([0xf1; 16], KvListCursor::Time(u64::MAX), 4096, true);
    let uid = account.credential.uid.as_bytes();
    let node: Vec<u8> = c
        .query_row(
            "SELECT node_id FROM kv_nodes WHERE uid=?1 AND node_type=3 LIMIT 1",
            params![uid],
            |r| r.get(0),
        )
        .unwrap();
    c.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    for kind in [-1i64, 0, 4] {
        c.execute(
            "UPDATE kv_nodes SET node_type=?1 WHERE uid=?2 AND node_id=?3",
            params![kind, uid, node],
        )
        .unwrap();
        compare(root, KvListCursor::None, 4096, true);
        compare(root, KvListCursor::None, 4096, false);
    }
    c.execute(
        "UPDATE kv_nodes SET node_type=3,exact_node=X'C1',ciphertext_bytes=NULL WHERE uid=?1 AND node_id=?2",
        params![uid, node],
    )
    .unwrap();
    compare(root, KvListCursor::None, 4096, true);
    compare(root, KvListCursor::None, 4096, false);
}
