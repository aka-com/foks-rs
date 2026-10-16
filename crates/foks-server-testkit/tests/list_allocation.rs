#![cfg(feature = "list-allocation-qualification")]
//! Local issued-certificate fixture export, not a production List endpoint.
use foks_metadata_core::{allocation_trace as trace, native::NativeSql};
use foks_proto::{
    KvDirectory, KvDirectoryStatus, KvDirent, KvNodeId, KvSmallFileBox, Role, RoleAndGeneration,
    SecretBox,
};
use foks_rpc::{KvAuth, KvListCursor, DEFAULT_MAX_FRAME_LENGTH};
use foks_server_db::Clock;
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use rusqlite::{params, types::ValueRef, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(self.0)
    }
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn id(n: usize) -> [u8; 16] {
    let mut b = [0; 16];
    b[8..].copy_from_slice(&(n as u64).to_be_bytes());
    b
}
fn node(n: usize) -> [u8; 17] {
    let mut b = [3; 17];
    b[1..].copy_from_slice(&id(n));
    b
}
fn mac(n: usize) -> [u8; 32] {
    let mut b = [0; 32];
    b[16..].copy_from_slice(&id(n));
    b
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn asset(dir: &Path, b: &[u8]) -> Value {
    let digest = hash(b);
    let name = format!("{digest}.bin");
    let p = dir.join(&name);
    if p.exists() {
        assert_eq!(std::fs::read(&p).unwrap(), b);
    } else {
        std::fs::write(&p, b).unwrap();
    }
    json!({"asset":{"path":name,"sha256":digest,"length":b.len()}})
}
fn snapshot(c: &Connection, dir: &Path) -> Vec<Value> {
    let names:Vec<String>=c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND substr(name,1,7)!='sqlite_' COLLATE NOCASE ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut q = c
                .prepare(&format!("SELECT * FROM {}", quote(&name)))
                .unwrap();
            let columns = q
                .column_names()
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            let mut rows = q
                .query_map([], |r| {
                    Ok((0..columns.len())
                        .map(|i| match r.get_ref(i).unwrap() {
                            ValueRef::Null => json!("null"),
                            ValueRef::Integer(n) => json!({"integer":n.to_string()}),
                            ValueRef::Text(t) => json!({"text":std::str::from_utf8(t).unwrap()}),
                            ValueRef::Blob(b) if b.len() > 1024 => asset(dir, b),
                            ValueRef::Blob(b) => json!({"blob":b}),
                            ValueRef::Real(_) => panic!("unexpected real"),
                        })
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort_by_cached_key(|r| serde_json::to_string(r).unwrap());
            json!({"name":name,"columns":columns,"rows":rows})
        })
        .collect()
}
fn box_bytes(role: Role, generation: u64, cipher: usize) -> Vec<u8> {
    KvSmallFileBox {
        key: RoleAndGeneration { role, generation },
        ciphertext: vec![8; cipher],
    }
    .encode()
    .unwrap()
}
fn entry(root: [u8; 16], n: usize, cipher: usize) -> KvDirent {
    KvDirent::new(
        root,
        id(n),
        KvNodeId(node(n)),
        1,
        1,
        Role::OWNER,
        mac(n),
        SecretBox {
            nonce: [7; 16],
            ciphertext: vec![8; cipher],
        },
        KvDirectoryStatus::Active,
        [9; 32],
        n as u64,
    )
    .unwrap()
}
struct Setup<'a> {
    c: &'a Connection,
    uid: &'a [u8],
    root: [u8; 16],
    parent: KvDirectory,
}
impl Setup<'_> {
    #[allow(clippy::too_many_arguments)] // Explicit independent fixture dimensions.
    fn seed(
        &self,
        rows: usize,
        max_entry: bool,
        body_cipher: usize,
        last_cipher: Option<usize>,
        large_parent: bool,
        roles: bool,
        malformed: bool,
    ) {
        let tx = self.c.unchecked_transaction().unwrap();
        tx.execute(
            "DELETE FROM kv_dirent_heads WHERE uid=?1",
            params![self.uid],
        )
        .unwrap();
        tx.execute("DELETE FROM kv_dirents WHERE uid=?1", params![self.uid])
            .unwrap();
        tx.execute("DELETE FROM kv_nodes WHERE uid=?1", params![self.uid])
            .unwrap();
        let mut parent = self.parent.clone();
        if large_parent {
            parent.seed_ciphertext = vec![1; 16 * 1024 * 1024 - 128];
            let initial = parent.encode().unwrap();
            let overhead = initial.len() - parent.seed_ciphertext.len();
            parent
                .seed_ciphertext
                .resize(16 * 1024 * 1024 - overhead, 1);
            assert_eq!(parent.encode().unwrap().len(), 16 * 1024 * 1024);
        }
        tx.execute(
            "UPDATE kv_directories SET exact_directory=?1 WHERE uid=?2 AND directory_id=?3",
            params![parent.encode().unwrap(), self.uid, self.root],
        )
        .unwrap();
        for n in 0..rows {
            let role = if roles {
                Role::member(i16::MIN + n as i16)
            } else {
                Role::OWNER
            };
            if roles {
                tx.execute("INSERT OR IGNORE INTO shared_keys(uid,role_type,visibility,generation,verify_key,exact_hepk) VALUES(?1,1,?2,1,zeroblob(33),X'01')",params![self.uid,role.visibility().unwrap()]).unwrap();
            }
            let len = if n + 1 == rows {
                last_cipher.unwrap_or(body_cipher)
            } else {
                body_cipher
            };
            let mut body = box_bytes(role, 1, len);
            let malformed = malformed && n + 1 == rows;
            if malformed {
                body = vec![0xc1];
            }
            tx.execute("INSERT INTO kv_nodes(uid,node_id,node_type,exact_node,ciphertext_bytes) VALUES(?1,?2,3,?3,?4)",params![self.uid,node(n),body,if malformed{None}else{Some(len as i64)}]).unwrap();
            let mut e = entry(self.root, n, if max_entry { 3500 } else { 16 });
            if max_entry {
                let overhead = e.encode().unwrap().len() - e.name_box.ciphertext.len();
                e.name_box.ciphertext.resize(4096 - overhead, 8);
                assert_eq!(e.encode().unwrap().len(), 4096);
            }
            tx.execute("INSERT INTO kv_dirents(uid,parent_id,dirent_id,version,directory_version,node_id,name_mac,creation_time,exact_dirent) VALUES(?1,?2,?3,1,1,?4,?5,?6,?7)",params![self.uid,self.root,id(n),node(n),mac(n),n as i64,e.encode().unwrap()]).unwrap();
            tx.execute(
                "INSERT INTO kv_dirent_heads VALUES(?1,?2,?3,1)",
                params![self.uid, self.root, id(n)],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    fn role_generation_edges(&self) {
        self.seed(5, false, 32, None, false, false, false);
        let roles = [
            Role::member(i16::MIN),
            Role::member(i16::MAX),
            Role::member(0),
            Role::member(1),
            Role::member(2),
        ];
        let tx = self.c.unchecked_transaction().unwrap();
        for (n, role) in roles.into_iter().enumerate() {
            let visibility = role.visibility().unwrap();
            tx.execute(
                "DELETE FROM shared_keys WHERE uid=?1 AND role_type=1 AND visibility=?2",
                params![self.uid, visibility],
            )
            .unwrap();
            // Distinct body-generation-zero case: valid current key generation1.
            let body_generation = if n == 4 { 0 } else { 1 };
            tx.execute("UPDATE kv_nodes SET exact_node=?1,ciphertext_bytes=NULL WHERE uid=?2 AND node_id=?3", params![box_bytes(role,body_generation,32),self.uid,node(n)]).unwrap();
            tx.execute(
                "UPDATE kv_nodes SET ciphertext_bytes=32 WHERE uid=?1 AND node_id=?2",
                params![self.uid, node(n)],
            )
            .unwrap();
            if n != 2 {
                // Member(0) is intentionally absent, not generation zero.
                tx.execute("INSERT INTO shared_keys(uid,role_type,visibility,generation,verify_key,exact_hepk) VALUES(?1,1,?2,1,zeroblob(33),X'01')",params![self.uid,visibility]).unwrap();
            }
        }
        // Malformed legacy stored generation0 is a distinct native-parity case.
        // Enable bypass only around this fixed seed mutation; always restore it.
        self.c
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        let changed = tx.execute(
            "UPDATE shared_keys SET generation=0 WHERE uid=?1 AND role_type=1 AND visibility=1",
            params![self.uid],
        );
        self.c
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
        assert_eq!(changed.unwrap(), 1);
        assert_eq!(
            self.c
                .pragma_query_value(None, "ignore_check_constraints", |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(tx
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none());
        tx.commit().unwrap();
    }
}
fn request(
    peer: &foks_metadata_core::personal::PeerCertificate,
    root: [u8; 16],
    now: u64,
    count: u64,
    frame: bool,
    sequence: u64,
    fail: u64,
) -> Value {
    json!({"version":1,"peer":peer,"now_micros":now.to_string(),"parent":root,"cursor":{"kind":"initial"},"count":count.to_string(),"load_small":true,"frame":frame,"rpc_sequence":sequence.to_string(),"fail_at":fail.to_string()})
}
fn check_bounds(r: &trace::Report) {
    let b = trace::formula::bounds().unwrap();
    let bytes = |phase: &str| {
        r.phases
            .iter()
            .find(|p| p.name == phase)
            .unwrap()
            .totals
            .admitted_bytes
    };
    assert!(r.maximum_page_scratch <= b.full_page_scratch);
    assert!(r.maximum_page_charge <= b.full_page_scratch.checked_add(b.retained_growth).unwrap());
    assert!(bytes("page") <= r.page_scopes.checked_mul(b.page_transport).unwrap());
    assert!(bytes("decode") <= 4096u64.checked_mul(b.row_decode).unwrap());
    assert!(bytes("append") <= 4096u64.checked_mul(b.row_append).unwrap());
    assert!(bytes("roles") <= r.page_scopes.checked_mul(b.page_roles).unwrap());
    assert!(bytes("growth") <= b.retained_growth);
    assert!(bytes("finish") <= b.finish);
    assert!(trace::encode_report(r).unwrap().len() <= trace::MAX_OUTPUT);
}
fn without_bodies(mut input: Value) -> Value {
    input["load_small"] = json!(false);
    input
}
fn with_cursor(mut input: Value, cursor: Value) -> Value {
    input["cursor"] = cursor;
    input
}
fn run(c: &Connection, input: &Value) -> trace::Report {
    let tx = c.unchecked_transaction().unwrap();
    let r = trace::run(
        &mut NativeSql::new(&tx).unwrap(),
        &serde_json::to_vec(input).unwrap(),
    )
    .unwrap();
    tx.rollback().unwrap();
    r
}
#[test]
#[ignore = "explicit local issued-certificate/native-Wasm artifact qualification"]
fn export_list_allocation_vectors_with_real_current_certificate() {
    let output = PathBuf::from(
        std::env::var_os("FOKS_LIST_CHARGE_VECTORS").expect("new output directory required"),
    );
    std::fs::create_dir(&output).unwrap();
    let env = TestEnvironment::new().unwrap();
    let server = env.start_server().unwrap();
    let client = TestClient::new(&env, "list-charge").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("listcharge", 0x37))
        .unwrap();
    server.shutdown().unwrap();
    let now = env.advance_clock(0);
    let cert =
        rustls_pki_types::CertificateDer::from(account.credential.certificate_chain[0].as_slice());
    let peer = foks_server::hosted_personal::peer_certificate(&cert).unwrap();
    let root = account.kv_projection[0].root_directory_id;
    let uid = account.credential.uid.as_bytes();
    let c = Connection::open(env.database_path()).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    let exact:Vec<u8>=c.query_row("SELECT exact_directory FROM kv_directories WHERE uid=?1 AND directory_id=?2 ORDER BY version DESC LIMIT 1",params![uid,root],|r|r.get(0)).unwrap();
    let setup = Setup {
        c: &c,
        uid,
        root,
        parent: KvDirectory::decode(&exact).unwrap(),
    };
    let objects:Vec<Value>=c.prepare("SELECT type,sql FROM sqlite_schema WHERE substr(name,1,7)!='sqlite_' COLLATE NOCASE AND sql IS NOT NULL ORDER BY type,name").unwrap().query_map([],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"sql":r.get::<_,String>(1)?}))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(now));
    let reader = env.read_database().unwrap();
    let mut fixtures = Vec::new();
    let mut collect = |name: &str, cases: Vec<Value>| {
        let before = snapshot(&c, &output);
        let mut exported = Vec::new();
        for (index, input) in cases.into_iter().enumerate() {
            let report = run(&c, &input);
            let fail = input["fail_at"].as_str().unwrap().parse::<u64>().unwrap();
            if fail == 0 {
                let count = input["count"].as_str().unwrap().parse::<u64>().unwrap();
                let wire = foks_rpc::encode_kv_list_request(
                    KvAuth::User,
                    &root,
                    match input["cursor"]["kind"].as_str().unwrap() {
                        "initial" => KvListCursor::None,
                        "mac" => KvListCursor::Mac(
                            serde_json::from_value(input["cursor"]["value"].clone()).unwrap(),
                        ),
                        "time" => KvListCursor::Time(
                            input["cursor"]["value"].as_str().unwrap().parse().unwrap(),
                        ),
                        _ => unreachable!("locally constructed fixture"),
                    },
                    count,
                    input["load_small"].as_bool().unwrap(),
                )
                .unwrap();
                let call =
                    foks_rpc::read_call(&mut wire.as_slice(), DEFAULT_MAX_FRAME_LENGTH).unwrap();
                let expected = foks_server::hosted_personal::native_list(
                    &cert,
                    &reader,
                    &clock,
                    call.argument(),
                )
                .unwrap();
                if let Ok(expected) = &expected {
                    if input["frame"] == true {
                        let seq = input["rpc_sequence"]
                            .as_str()
                            .unwrap()
                            .parse::<u64>()
                            .unwrap();
                        match foks_rpc::encode_success_response_at(expected, seq) {
                            Ok(frame) => {
                                assert_eq!(report.outcome, "ok");
                                assert_eq!(
                                    report.frame_sha256.as_deref(),
                                    Some(hash(&frame).as_str())
                                );
                            }
                            Err(foks_rpc::Error::FrameTooLarge { .. }) => {
                                assert_eq!(report.outcome, "frame_too_large")
                            }
                            Err(e) => panic!("unexpected RPC error {e}"),
                        }
                    } else {
                        assert_eq!(report.outcome, "ok");
                    }
                    if report.outcome == "ok" {
                        assert_eq!(report.list_sha256.as_deref(), Some(hash(expected).as_str()));
                    }
                } else {
                    match expected.unwrap_err() {
                        foks_rpc::RpcStatus::TransactionRetry => {
                            assert_eq!(report.outcome, "corrupt")
                        }
                        status => panic!("unplanned native oracle error {status:?}"),
                    }
                }
            } else {
                assert_eq!(report.outcome, "trace_refused");
                assert!(report.list_sha256.is_none() && report.frame_sha256.is_none());
            }
            if report.outcome != "ok" {
                assert!(report.list_sha256.is_none() && report.frame_sha256.is_none());
                assert_eq!((report.response_bytes, report.frame_bytes), (0, 0));
            }
            check_bounds(&report);
            let expected = serde_json::to_value(&report).unwrap();
            exported
                .push(json!({"name":format!("{name}-{index}"),"input":input,"expected":expected}));
        }
        assert_eq!(snapshot(&c, &output), before);
        fixtures.push(json!({"name":name,"before":before,"cases":exported}));
    };
    setup.seed(30, false, 32, None, false, false, false);
    collect(
        "small",
        [0, 1, 14, 15, 16, 30, 4096]
            .into_iter()
            .map(|n| request(&peer, root, now, n, true, 0, 0))
            .chain([
                without_bodies(request(&peer, root, now, 30, true, 0, 0)),
                with_cursor(
                    request(&peer, root, now, 30, true, 0, 0),
                    json!({"kind":"mac","value":mac(14)}),
                ),
                with_cursor(
                    request(&peer, root, now, 30, true, 0, 0),
                    json!({"kind":"time","value":"0"}),
                ),
                with_cursor(
                    request(&peer, root, now, 30, true, 0, 0),
                    json!({"kind":"time","value":u64::MAX.to_string()}),
                ),
            ])
            .collect(),
    );
    let baseline = run(&c, &request(&peer, root, now, 1, true, 0, 0));
    for fail in 1..=baseline.total.attempts {
        let r = run(&c, &request(&peer, root, now, 1, true, 0, fail));
        assert_eq!(r.outcome, "trace_refused");
        assert!(r.list_sha256.is_none());
    }
    assert_eq!(
        run(&c, &request(&peer, root, now, 1, true, 0, 0)).list_sha256,
        baseline.list_sha256
    );
    let overhead = box_bytes(Role::OWNER, 1, 65000).len() - 65000;
    setup.seed(15, true, 65536 - overhead, None, false, false, false);
    collect(
        "maximum_page",
        vec![
            request(&peer, root, now, 15, true, 0, 0),
            without_bodies(request(&peer, root, now, 1, false, 0, 0)),
            request(&peer, root, now, 15, true, 0, 0),
            request(&peer, root, now, 15, false, 0, 1),
            request(&peer, root, now, 15, false, 0, 2),
            request(&peer, root, now, 15, false, 0, 0),
        ],
    );
    setup.seed(15, false, 32, None, false, false, true);
    collect(
        "late_malformed",
        vec![
            request(&peer, root, now, 15, false, 0, 0),
            without_bodies(request(&peer, root, now, 1, false, 0, 0)),
        ],
    );
    // An exact public boundary is computed through the unchanged real encoder.
    for sequence in [
        0,
        127,
        128,
        255,
        256,
        65535,
        65536,
        u32::MAX as u64,
        u32::MAX as u64 + 1,
        u64::MAX,
    ] {
        setup.seed(256, false, 65500, Some(1000), false, false, false);
        let baseline = run(&c, &request(&peer, root, now, 256, false, sequence, 0));
        let empty = foks_proto::KvListResponse::new(vec![], true, vec![]).unwrap();
        let framed = foks_rpc::encode_success_response_at(empty.encoded(), sequence).unwrap();
        let content =
            foks_rpc::read_frame(&mut framed.as_slice(), DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let overhead = content.len() - empty.encoded().len();
        let target = DEFAULT_MAX_FRAME_LENGTH - overhead;
        let last = 1000 + target - baseline.response_bytes as usize;
        assert!(last < 65500);
        for delta in [-1i64, 0, 1] {
            let body = (last as i64 + delta) as usize;
            setup.seed(256, false, 65500, Some(body), false, false, false);
            let name = format!("frame_{sequence}_{delta}");
            collect(
                &name,
                vec![request(&peer, root, now, 256, true, sequence, 0)],
            );
        }
    }
    // Combine maximum parent with near-maximum response in the SAME invocation.
    setup.seed(256, false, 65500, Some(1000), true, false, false);
    collect(
        "large_parent_cycles",
        vec![
            request(&peer, root, now, 256, true, 0, 0),
            without_bodies(request(&peer, root, now, 1, false, 0, 0)),
            request(&peer, root, now, 256, true, 0, 0),
        ],
    );
    setup.seed(4096, false, 32, None, false, true, false);
    collect(
        "distinct_roles",
        vec![
            request(&peer, root, now, 4096, true, 0, 0),
            request(&peer, root, now, 1, true, 0, 0),
            request(&peer, root, now, 4096, true, 0, 0),
        ],
    );
    setup.role_generation_edges();
    collect(
        "role_generation_edges",
        vec![
            request(&peer, root, now, 5, true, 0, 0),
            without_bodies(request(&peer, root, now, 5, true, 0, 0)),
            request(&peer, root, now, 5, true, 0, 0),
        ],
    );
    let manifest = json!({"version":1,"schema_version":foks_server_db::SCHEMA_VERSION,"objects":objects,"fixtures":fixtures,"native_layout":trace::layout_report().unwrap(),"scalar_coverage":"S1 admits requested native binding/result copies; S2 Wasm scalar and S3 domain/host gaps remain incomplete"});
    let bytes = serde_json::to_vec(&manifest).unwrap();
    assert!(bytes.len() < 64 * 1024 * 1024);
    std::fs::write(output.join("vectors.json"), bytes).unwrap();
    std::fs::write(
        output.join("catalog.json"),
        serde_json::to_vec(
            foks_metadata_core::Statement::ALL
                .map(foks_metadata_core::Statement::spec)
                .as_slice(),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("page-catalog.json"),
        foks_metadata_core::page::catalog_artifact().unwrap(),
    )
    .unwrap();
}
