use a::{Number as N, Request as R};
use foks_metadata_core::{
    authority as a, native::NativeSql, personal as p, wire, Limits, Mutation, Statement, Value,
};
use foks_proto::{KvDirent, Role, RoleAndGeneration};
use foks_server::hosted_personal;
use foks_server_db::{Clock, Config};
use foks_server_testkit::TestEnvironment;
use rusqlite::{params, types::ValueRef, Connection};
use rustls_pki_types::CertificateDer;
use serde::Serialize;
use std::sync::Arc;

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
#[derive(Clone, Serialize)]
struct Table {
    name: String,
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}
#[derive(Clone, Serialize)]
struct Object {
    kind: String,
    sql: String,
}
fn schema(c: &Connection) -> Vec<Object> {
    c.prepare("SELECT type,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name").unwrap().query_map([],|r|Ok(Object{kind:r.get(0)?,sql:r.get(1)?})).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
fn snapshot(c: &Connection) -> Vec<Table> {
    let names:Vec<String>=c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut query = c
                .prepare(&format!("SELECT * FROM {}", quote(&name)))
                .unwrap();
            let columns = query
                .column_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let count = columns.len();
            let mut rows = query
                .query_map([], |row| {
                    (0..count)
                        .map(|i| {
                            Ok(match row.get_ref(i)? {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(n) => Value::Integer(n),
                                ValueRef::Text(t) => {
                                    Value::Text(std::str::from_utf8(t).unwrap().to_owned())
                                }
                                ValueRef::Blob(b) => Value::Blob(b.to_vec()),
                                ValueRef::Real(_) => panic!("unexpected native REAL"),
                            })
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort_by_cached_key(|row| serde_json::to_string(row).unwrap());
            Table {
                name,
                columns,
                rows,
            }
        })
        .collect()
}

struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_micros(&self) -> foks_server_db::Result<u64> {
        Ok(self.0)
    }
}
pub fn compare_native_keys(environment: &TestEnvironment, der: &[u8], now: u64) {
    let cert = CertificateDer::from(der);
    let peer = hosted_personal::peer_certificate(&cert).unwrap();
    let requirements: Vec<_> = [
        Role::NONE,
        Role::member(-1),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ]
    .into_iter()
    .flat_map(|role| {
        [0, 1, 2].into_iter().flat_map(move |generation| {
            [p::KeyAccess::Read, p::KeyAccess::Write]
                .into_iter()
                .map(move |access| p::KeyRequirement {
                    key: RoleAndGeneration { role, generation },
                    access,
                })
        })
    })
    .collect();
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(now));
    let expected = hosted_personal::native_key_decisions(
        &cert,
        &environment.read_database().unwrap(),
        &clock,
        &requirements,
    )
    .unwrap();
    let c = Connection::open(environment.database_path()).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let current = p::authenticate_peer(&mut sql, &peer, now).unwrap();
    p::require_personal_profile(&mut sql, &current).unwrap();
    let actual: Vec<_> = requirements
        .into_iter()
        .map(|r| p::require_key(&mut sql, &current, r).is_ok())
        .collect();
    assert_eq!(actual, expected, "unchanged native personal key decisions");
}
fn limits() -> Limits {
    let c = Config::default();
    Limits {
        blob_bytes: c.maximum_blob_bytes,
        dirent_bytes: c.maximum_kv_dirent_bytes,
        namespace_bytes: c.maximum_kv_namespace_bytes,
        namespace_objects: c.maximum_kv_namespace_objects,
        directories: c.maximum_kv_directories,
        dirents: c.maximum_kv_dirents,
    }
}
fn encoded(value: &impl Serialize) -> serde_json::Value {
    serde_json::to_value(value).unwrap()
}
#[derive(Serialize)]
struct Vector {
    name: String,
    command: String,
    before: Vec<Table>,
    after: Vec<Table>,
    outcome: serde_json::Value,
    reply: Option<serde_json::Value>,
    authority: bool,
    personal: &'static str,
}
struct Harness {
    c: Connection,
    peer: p::PeerCertificate,
    fence: a::Fence,
    now: i64,
    vectors: Vec<Vector>,
}
impl Harness {
    fn new(environment: &TestEnvironment, der: &[u8], now: u64) -> Self {
        let c = Connection::open(environment.database_path()).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c.execute_batch(a::SCHEMA).unwrap();
        // A real-personal invocation must never consult this synthetic table.
        c.execute_batch("DROP TABLE metadata_principals").unwrap();
        let peer = hosted_personal::peer_certificate(&CertificateDer::from(der)).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let current = p::authenticate_peer(&mut NativeSql::new(&tx).unwrap(), &peer, now).unwrap();
        tx.rollback().unwrap();
        let host: Vec<u8> = c
            .query_row(
                "SELECT host_id FROM kv_namespaces WHERE namespace_id=?1",
                [current.uid().as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        let fence = a::Fence {
            identity: a::Identity {
                installation: [1; 16],
                tenant: [2; 16],
                host,
                cell: [4; 32],
            },
            activation: a::Activation {
                version: N(1),
                schema: N(foks_server_db::SCHEMA_VERSION),
                schema_hash: [5; 32],
                kernel_hash: [6; 32],
                catalog_hash: [7; 32],
                key_generation: N(1),
                key_digest: [8; 32],
                storage_namespace: [9; 16],
                storage_placement: [10; 32],
                limits: limits(),
            },
            owner: [1; 16],
            epoch: N(1),
        };
        let now = i64::try_from(now / 1000).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        a::run(
            &mut NativeSql::new(&tx).unwrap(),
            a::Invocation {
                caller: a::Caller::Controller {},
                now: N(now),
                deadline: N(now + 1000),
                request: R::Initialize {
                    identity: fence.identity.clone(),
                    activation: fence.activation.clone(),
                    owner: fence.owner,
                    max_rows: N(32),
                    max_bytes: N(32 * a::INTENT_CHARGE),
                },
            },
        )
        .unwrap();
        tx.commit().unwrap();
        Self {
            c,
            peer,
            fence,
            now,
            vectors: vec![],
        }
    }
    fn revision(&self) -> N {
        N(self
            .c
            .query_row("SELECT revision FROM metadata_authority", [], |r| r.get(0))
            .unwrap())
    }
    fn invoke(&mut self, name: &str, request: R) -> p::Result<a::Reply> {
        let invocation = a::PersonalInvocation {
            peer: self.peer.clone(),
            now: N(self.now),
            deadline: N(self.now + 1000),
            request,
        };
        let command = serde_json::to_string(&invocation).unwrap();
        let before = snapshot(&self.c);
        let tx = self.c.transaction().unwrap();
        let result = a::run_personal(&mut NativeSql::new(&tx).unwrap(), invocation);
        if result.is_ok() {
            tx.commit().unwrap();
        } else {
            tx.rollback().unwrap();
        }
        self.record(name, command, before, &result, "authority");
        result
    }
    fn point(&mut self, name: &str, point: p::PersonalPoint) -> p::Result<wire::Reply> {
        let invocation = a::PersonalPointInvocation {
            peer: self.peer.clone(),
            now: N(self.now),
            deadline: N(self.now + 1000),
            fence: self.fence.clone(),
            revision: self.revision(),
            point,
        };
        let command = serde_json::to_string(&invocation).unwrap();
        let before = snapshot(&self.c);
        let tx = self.c.transaction().unwrap();
        let result = a::read_personal_point(&mut NativeSql::new(&tx).unwrap(), invocation);
        if result.is_ok() {
            tx.commit().unwrap();
        } else {
            tx.rollback().unwrap();
        }
        self.record(name, command, before, &result, "point");
        result
    }
    fn record<T: Serialize>(
        &mut self,
        name: &str,
        command: String,
        before: Vec<Table>,
        result: &p::Result<T>,
        personal: &'static str,
    ) {
        let after = snapshot(&self.c);
        if result.is_err() {
            assert_eq!(
                encoded(&before),
                encoded(&after),
                "failed invocation changed SQL state: {name}"
            );
        }
        self.vectors.push(Vector {
            name: name.into(),
            command,
            before,
            after,
            outcome: result.as_ref().err().map_or_else(
                || serde_json::json!("ok"),
                |e| encoded(&a::PersonalErrorPayload::from(e)),
            ),
            reply: result.as_ref().ok().map(encoded),
            authority: false,
            personal,
        });
    }
    fn export(&self, suite: &str) {
        if let Some(root) = std::env::var_os("FOKS_PERSONAL_VECTORS") {
            let path = std::path::Path::new(&root).join(suite);
            std::fs::create_dir_all(&path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let data = serde_json::json!({"schema_version":foks_server_db::SCHEMA_VERSION,"objects":schema(&self.c),"vectors":self.vectors,"provenance":suite,"scope":"actual native signup and encrypted KV setup; private personal SQL resolver; scalar bounded profile; no persistent production adapter"});
            std::fs::write(
                path.join("vectors.json"),
                serde_json::to_vec(&data).unwrap(),
            )
            .unwrap();
            std::fs::write(
                path.join("catalog.json"),
                serde_json::to_vec(Statement::ALL.map(Statement::spec).as_slice()).unwrap(),
            )
            .unwrap();
        }
    }
}
fn mutation(d: &KvDirent, version: u64) -> Mutation {
    let changed = KvDirent::new(
        d.parent,
        d.id,
        d.value,
        version,
        d.directory_version,
        d.write_role,
        d.name_mac,
        d.name_box.clone(),
        d.directory_status,
        d.binding_mac,
        d.creation_time,
    )
    .unwrap();
    Mutation {
        parent: changed.parent,
        id: changed.id,
        version: changed.version,
        directory_version: changed.directory_version,
        node_id: changed.value.0,
        name_mac: changed.name_mac,
        creation_time: changed.creation_time,
        exact: changed.encoded().to_vec(),
    }
}
fn command(uid: &[u8], m: Mutation, precondition: Option<Vec<u8>>) -> String {
    serde_json::to_string(&wire::Command::PutDirents {
        uid: uid.to_vec(),
        role: Role::OWNER.encoded().unwrap(),
        mutations: vec![m],
        precondition,
        limits: limits(),
        normalize_at: None,
    })
    .unwrap()
}
fn intent(h: &Harness, sequence: i64, command: String) -> a::Intent {
    a::Intent {
        epoch: h.fence.epoch,
        sequence: N(sequence),
        revision: h.revision(),
        expires: N(h.now + 500),
        command,
    }
}
fn register(h: &mut Harness, name: &str, i: &a::Intent) -> [u8; 32] {
    match h
        .invoke(
            name,
            R::Register {
                fence: h.fence.clone(),
                intent: i.clone(),
            },
        )
        .unwrap()
    {
        a::Reply::Registered { digest } => digest,
        other => panic!("unexpected {other:?}"),
    }
}

pub fn qualify(
    environment: &TestEnvironment,
    suite: &str,
    der: &[u8],
    uid: &[u8],
    credential: &[u8],
    renewed: Option<&[u8]>,
    now: u64,
) {
    let mut h = Harness::new(environment, der, now);
    {
        let tx = h.c.unchecked_transaction().unwrap();
        let mut sql = NativeSql::new(&tx).unwrap();
        let current = p::authenticate_peer(&mut sql, &h.peer, now).unwrap();
        assert_eq!(current.uid().as_slice(), uid);
        assert_eq!(current.credential_id(), credential);
    }
    let exact:Vec<u8>=h.c.query_row("SELECT d.exact_dirent FROM kv_dirent_heads h JOIN kv_dirents d USING(uid,parent_id,dirent_id,version) WHERE d.uid=?1 AND substr(d.node_id,1,1)=X'03' ORDER BY d.dirent_id LIMIT 1",[uid],|r|r.get(0)).unwrap();
    let d = KvDirent::decode(&exact).unwrap();
    let root_exact: Vec<u8> =
        h.c.query_row("SELECT exact_root FROM kv_roots WHERE uid=?1", [uid], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        h.point("root_exact", p::PersonalPoint::Root {})
            .unwrap()
            .values,
        vec![Value::Blob(root_exact)]
    );
    let dir_exact:Vec<u8>=h.c.query_row("SELECT d.exact_directory FROM kv_directory_heads h JOIN kv_directories d USING(uid,directory_id,version) WHERE h.uid=?1 AND h.directory_id=?2",params![uid,d.parent],|r|r.get(0)).unwrap();
    let pair = foks_proto::KvDirectoryPair::from_active(
        foks_proto::KvDirectory::decode(&dir_exact).unwrap(),
    )
    .unwrap();
    assert_eq!(
        h.point(
            "directory_exact_pair",
            p::PersonalPoint::Directory { id: d.parent }
        )
        .unwrap()
        .values,
        vec![Value::Blob(pair.encoded().to_vec())]
    );
    let node_exact: Vec<u8> =
        h.c.query_row(
            "SELECT exact_node FROM kv_nodes WHERE uid=?1 AND node_id=?2",
            params![uid, d.value.0],
            |r| r.get(0),
        )
        .unwrap();
    let node =
        foks_proto::KvNode::SmallFile(foks_proto::KvSmallFileBox::decode(&node_exact).unwrap());
    assert_eq!(
        h.point("small_node_exact", p::PersonalPoint::Node { id: d.value.0 })
            .unwrap()
            .values,
        vec![Value::Blob(node.encoded().unwrap())]
    );
    h.point(
        "namespace_quota_counters_not_public_usage",
        p::PersonalPoint::QuotaCounters {},
    )
    .unwrap();
    let native_usage = || {
        let clock: Arc<dyn Clock> = Arc::new(FixedClock(now));
        hosted_personal::native_usage(
            &CertificateDer::from(der),
            &environment.read_database().unwrap(),
            &clock,
        )
        .unwrap()
    };
    let exact_usage = native_usage().unwrap();
    assert_eq!(
        h.point("public_usage_exact", p::PersonalPoint::PublicUsage {})
            .unwrap()
            .values,
        vec![Value::Blob(exact_usage)]
    );
    // ABI precision fixture, not a supported production counter mutation.
    let original_bytes: i64 =
        h.c.query_row(
            "SELECT usage_small_bytes FROM kv_namespaces WHERE namespace_id=?1",
            [uid],
            |r| r.get(0),
        )
        .unwrap();
    h.c.execute(
        "UPDATE kv_namespaces SET usage_small_bytes=9007199254740993 WHERE namespace_id=?1",
        [uid],
    )
    .unwrap();
    let exact_usage = native_usage().unwrap();
    assert_eq!(
        foks_proto::KvUsage::decode(&exact_usage)
            .unwrap()
            .small
            .bytes,
        9_007_199_254_740_993
    );
    assert_eq!(
        h.point(
            "public_usage_exact_integer_above_js_safe_range",
            p::PersonalPoint::PublicUsage {}
        )
        .unwrap()
        .values,
        vec![Value::Blob(exact_usage)]
    );
    h.c.execute(
        "UPDATE kv_namespaces SET usage_small_bytes=?1 WHERE namespace_id=?2",
        params![original_bytes, uid],
    )
    .unwrap();
    h.c.execute(
        "UPDATE kv_nodes SET exact_node=X'00',ciphertext_bytes=NULL WHERE uid=?1 AND node_id=?2",
        params![uid, d.value.0],
    )
    .unwrap();
    assert_eq!(native_usage(), Err(foks_rpc::RpcStatus::TransactionRetry));
    assert_eq!(
        h.point(
            "public_usage_corrupt_node_never_partial_success",
            p::PersonalPoint::PublicUsage {}
        ),
        Err(p::PersonalError::Corrupt)
    );
    let length =
        i64::try_from(foks_proto::KvSmallFileBox::ciphertext_length(&node_exact).unwrap()).unwrap();
    h.c.execute(
        "UPDATE kv_nodes SET exact_node=?1,ciphertext_bytes=NULL WHERE uid=?2 AND node_id=?3",
        params![node_exact, uid, d.value.0],
    )
    .unwrap();
    h.c.execute(
        "UPDATE kv_nodes SET ciphertext_bytes=?1 WHERE uid=?2 AND node_id=?3 AND exact_node=?4",
        params![length, uid, d.value.0, node_exact],
    )
    .unwrap();
    assert_eq!(
        h.point(
            "public_usage_revalidated_exact_node",
            p::PersonalPoint::PublicUsage {}
        )
        .unwrap()
        .values,
        vec![Value::Blob(native_usage().unwrap())]
    );
    assert_eq!(
        h.point("missing_node", p::PersonalPoint::Node { id: [3; 17] }),
        Err(p::PersonalError::Missing)
    );
    assert_eq!(
        h.point(
            "large_node_unsupported",
            p::PersonalPoint::Node { id: [2; 17] }
        ),
        Err(p::PersonalError::UnsupportedProfile)
    );
    let mut badfence = h.fence.clone();
    badfence.owner = [99; 16];
    assert_eq!(
        h.invoke(
            "stale_owner",
            R::Prove {
                fence: badfence,
                revision: h.revision()
            }
        ),
        Err(p::PersonalError::Kernel(
            foks_metadata_core::Error::Permission
        ))
    );
    assert_eq!(
        h.invoke(
            "stale_revision",
            R::Prove {
                fence: h.fence.clone(),
                revision: N(h.revision().0 + 1)
            }
        ),
        Err(p::PersonalError::Kernel(
            foks_metadata_core::Error::Conflict
        ))
    );
    assert_eq!(
        h.invoke(
            "controller_cannot_be_selected",
            R::Promote {
                fence: h.fence.clone(),
                new_owner: [8; 16]
            }
        ),
        Err(p::PersonalError::Kernel(
            foks_metadata_core::Error::Permission
        ))
    );
    let first = intent(&h, 1, command(uid, mutation(&d, d.version + 1), None));
    let digest = register(&mut h, "register_actual_credential", &first);
    assert_eq!(digest, register(&mut h, "registration_reply_lost", &first));
    if let Some(renewed) = renewed {
        let original = h.peer.clone();
        h.peer = hosted_personal::peer_certificate(&CertificateDer::from(renewed)).unwrap();
        assert_eq!(
            h.invoke(
                "renewal_cannot_execute_original_context",
                R::Execute {
                    fence: h.fence.clone(),
                    intent: first.clone()
                }
            ),
            Err(p::PersonalError::Kernel(
                foks_metadata_core::Error::Conflict
            ))
        );
        h.peer = original;
    }
    let success = h
        .invoke(
            "execute_real_personal_put",
            R::Execute {
                fence: h.fence.clone(),
                intent: first.clone(),
            },
        )
        .unwrap();
    assert!(
        matches!(&success,a::Reply::Outcome{outcome} if outcome.error.is_none() && outcome.revision==N(2))
    );
    assert_eq!(
        h.invoke(
            "lost_execution_reply_exact_replay",
            R::Execute {
                fence: h.fence.clone(),
                intent: first.clone()
            }
        )
        .unwrap(),
        success
    );
    if let Some(renewed) = renewed {
        let original = h.peer.clone();
        h.peer = hosted_personal::peer_certificate(&CertificateDer::from(renewed)).unwrap();
        assert_eq!(
            h.invoke(
                "renewal_can_resolve_original_outcome",
                R::Resolve {
                    fence: h.fence.clone(),
                    epoch: N(1),
                    sequence: N(1),
                    digest
                }
            )
            .unwrap(),
            success
        );
        h.peer = original;
    }
    // Completed receipt reconciliation precedes mutable destination checks.
    // Remove only its target node; current credential/profile still must pass.
    h.c.execute(
        "DELETE FROM kv_nodes WHERE uid=?1 AND node_id=?2",
        params![uid, d.value.0],
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "committed_outcome_after_destination_change",
            R::Execute {
                fence: h.fence.clone(),
                intent: first.clone()
            }
        )
        .unwrap(),
        success
    );
    h.c.execute(
        "INSERT INTO kv_nodes(uid,node_id,node_type,exact_node) VALUES(?1,?2,3,?3)",
        params![uid, d.value.0, node_exact],
    )
    .unwrap();
    let stale = foks_proto::KvPathVersionVector {
        root_version: 0,
        directories: vec![foks_proto::KvDirectoryVersion {
            id: d.parent,
            version: d.directory_version,
            entries: vec![foks_proto::KvDirentVersion {
                id: d.id,
                version: d.version,
            }],
        }],
    };
    let bad = intent(
        &h,
        2,
        command(
            uid,
            mutation(&d, d.version + 2),
            Some(stale.encode().unwrap()),
        ),
    );
    let failure = h
        .invoke(
            "stale_cache_retains_exact_vector",
            R::Register {
                fence: h.fence.clone(),
                intent: bad,
            },
        )
        .unwrap_err();
    let p::PersonalError::StaleCache(vector) = failure else {
        panic!("expected exact stale cache vector, got {failure:?}")
    };
    assert_eq!(vector.directories[0].entries[0].version, d.version + 1);
    assert!(
        matches!(a::PersonalErrorPayload::from(&p::PersonalError::StaleCache(vector)),a::PersonalErrorPayload::StaleCache{exact_hex} if !exact_hex.is_empty())
    );
    let gap = intent(&h, 2, command(uid, mutation(&d, d.version + 3), None));
    register(&mut h, "register_domain_conflict", &gap);
    let rejection = h
        .invoke(
            "personal_nested_domain_rejection",
            R::Execute {
                fence: h.fence.clone(),
                intent: gap,
            },
        )
        .unwrap();
    assert!(
        matches!(rejection,a::Reply::Outcome{outcome} if outcome.error.as_deref()==Some("conflict") && outcome.reply.is_none())
    );
    let head: i64 =
        h.c.query_row(
            "SELECT version FROM kv_dirent_heads WHERE uid=?1 AND parent_id=?2 AND dirent_id=?3",
            params![uid, d.parent, d.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        head,
        i64::try_from(d.version + 1).unwrap(),
        "rejected domain did not publish a new version"
    );
    let pending = intent(&h, 3, command(uid, mutation(&d, d.version + 2), None));
    register(&mut h, "register_before_authority_change", &pending);
    h.c.execute(
        "UPDATE devices SET active=0 WHERE device_id=?1",
        [credential],
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "revoked_execute",
            R::Execute {
                fence: h.fence.clone(),
                intent: pending.clone()
            }
        ),
        Err(p::PersonalError::Unauthenticated)
    );
    assert_eq!(
        h.invoke(
            "revoked_outcome_lookup",
            R::Resolve {
                fence: h.fence.clone(),
                epoch: N(1),
                sequence: N(1),
                digest
            }
        ),
        Err(p::PersonalError::Unauthenticated)
    );
    assert_eq!(
        h.point("revoked_read", p::PersonalPoint::Root {}),
        Err(p::PersonalError::Unauthenticated)
    );
    h.c.execute(
        "UPDATE devices SET active=1,role_type=2 WHERE device_id=?1",
        [credential],
    )
    .unwrap();
    assert_eq!(
        h.invoke(
            "demoted_pending_write",
            R::Execute {
                fence: h.fence.clone(),
                intent: pending.clone()
            }
        ),
        Err(p::PersonalError::Kernel(
            foks_metadata_core::Error::Permission
        ))
    );
    assert_eq!(
        h.point("demoted_owner_root", p::PersonalPoint::Root {}),
        Err(p::PersonalError::Permission)
    );
    h.c.execute(
        "UPDATE devices SET role_type=3 WHERE device_id=?1",
        [credential],
    )
    .unwrap();
    // Domain revision remains the caller-hosted complete mutation barrier. This
    // direct fixture alteration models a committed intervening auth mutation.
    h.c.execute("UPDATE metadata_authority SET revision=revision+1", [])
        .unwrap();
    assert_eq!(
        h.invoke(
            "intervening_authority_revision",
            R::Execute {
                fence: h.fence.clone(),
                intent: pending
            }
        ),
        Err(p::PersonalError::Kernel(
            foks_metadata_core::Error::Conflict
        ))
    );
    h.now += 1000;
    assert_eq!(
        h.invoke(
            "fresh_observation_after_original_expiry",
            R::Resolve {
                fence: h.fence.clone(),
                epoch: N(1),
                sequence: N(1),
                digest
            }
        )
        .unwrap(),
        success
    );
    let original = h.peer.clone();
    let mut wrong_key = *original.public_key();
    wrong_key[0] ^= 1;
    h.peer = p::PeerCertificate::from_frontend_parts(
        original.exact_der().to_vec(),
        original.serial_magnitude().to_vec(),
        wrong_key,
        original.expires_at_micros(),
    )
    .unwrap();
    assert_eq!(
        h.point("wrong_bound_public_key", p::PersonalPoint::Root {}),
        Err(p::PersonalError::Unauthenticated)
    );
    h.peer = original;
    h.now = i64::try_from(h.peer.expires_at_micros() / 1000 + 1).unwrap();
    assert_eq!(
        h.point("expired_certificate_observation", p::PersonalPoint::Root {}),
        Err(p::PersonalError::Unauthenticated)
    );
    let mut forbidden = encoded(&a::PersonalInvocation {
        peer: h.peer.clone(),
        now: N(h.now),
        deadline: N(h.now + 1000),
        request: R::Inspect {
            fence: h.fence.clone(),
        },
    });
    forbidden["caller"] = serde_json::json!({"kind":"controller"});
    assert!(serde_json::from_value::<a::PersonalInvocation>(forbidden).is_err());
    h.export(suite);
}
