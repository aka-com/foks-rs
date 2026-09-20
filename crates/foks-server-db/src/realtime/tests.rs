use super::*;
use crate::{Config, ReadDatabase};
#[test]
fn channel_policy_covers_discovery_read_write_and_redaction() {
    let f = Fixture::new();
    for tier in [RtChannelTier::Bottom, RtChannelTier::Admin] {
        let mut md = f.create.metadata.clone();
        md.tier = tier;
        md.roles.read = Role::ADMIN;
        md.roles.write = Role::OWNER;
        md.mtime = 99;
        let policy = ChannelPolicy::new(&md).unwrap();
        for role in [MIN_ROLE, Role::member(0), Role::ADMIN, Role::OWNER] {
            let visible = tier == RtChannelTier::Bottom || role >= Role::ADMIN;
            assert_eq!(policy.can_discover(role), visible);
            assert_eq!(policy.require_read(role).is_ok(), role >= Role::ADMIN);
            assert_eq!(policy.require_write(role).is_ok(), role == Role::OWNER);
            let projected = policy.project(role, md.clone());
            assert_eq!(projected.is_some(), visible);
            if let Some(projected) = projected {
                assert_eq!(projected.unreadable, role < Role::ADMIN);
                assert_eq!(projected.description.is_some(), role >= Role::ADMIN);
                assert_eq!(
                    projected.mtime,
                    if role < Role::ADMIN { md.ctime } else { 99 }
                );
            }
        }
    }
}

#[test]
fn legacy_methods_isolate_extended_channels_without_starving_basic_pages() {
    let mut f = Fixture::new();
    f.db.rt_create_channel_with_format(&f.owner, &f.create, 100, 2)
        .unwrap();
    let extended = f.create.metadata.id;
    let mut basic = f.create.clone();
    basic.metadata.id = RtChannelId([2; 16]);
    basic.metadata.updated_at = 2;
    basic.set_version = 2;
    f.db.rt_create_channel(&f.owner, &basic, 200).unwrap();
    let reader = f.reader();
    let snapshot = reader.snapshot().unwrap();
    let channels = snapshot
        .rt_list_channels(
            &f.member,
            &RtListChannelsArgument {
                team: basic.metadata.team.clone(),
                app: RtAppId::Chat,
                last: 0,
            },
            300,
        )
        .unwrap();
    assert_eq!(channels.version, 2);
    assert_eq!(
        channels.channels.iter().map(|c| c.id).collect::<Vec<_>>(),
        [basic.metadata.id]
    );
    let delta = snapshot
        .rt_changed_threads(
            &f.member,
            &RtGetChangedThreadsArgument {
                query: RtChangedThreads {
                    app: RtAppId::Chat,
                    since: 0,
                    maximum: 1,
                },
            },
            300,
        )
        .unwrap();
    assert_eq!(delta.channels.len(), 1);
    assert_eq!(delta.channels[0].metadata.id, basic.metadata.id);
    assert_eq!(delta.channels[0].inbox_version, 2);
    assert_eq!(delta.inbox_version, 2);
    assert!(matches!(
        snapshot.rt_recents(
            &f.member,
            &RtRecentsArgument {
                channel: extended,
                stop_at: 0,
                limit: 1
            },
            300
        ),
        Err(Error::RtUnsupportedFormat)
    ));
    assert!(matches!(
        snapshot.rt_thread(
            &f.member,
            &RtGetThreadArgument {
                query: RtThreadQuery {
                    channel: extended,
                    ranges: vec![],
                    sequences: vec![1]
                }
            },
            300
        ),
        Err(Error::RtUnsupportedFormat)
    ));
    let mut expired = f.member.clone();
    expired.certificate_expires_at = 299;
    assert!(matches!(
        snapshot.rt_recents(
            &expired,
            &RtRecentsArgument {
                channel: extended,
                stop_at: 0,
                limit: 1
            },
            300
        ),
        Err(Error::AuthorizationChanged)
    ));
    drop(snapshot);
    let send = f.send(9);
    assert!(matches!(
        f.db.rt_send(&f.member, &send, 300),
        Err(Error::RtUnsupportedFormat)
    ));
    assert!(matches!(
        f.db.rt_read_through(
            &f.member,
            &RtReadThroughArgument {
                read: RtReadThrough {
                    channel: extended,
                    sequence: 1
                }
            },
            300
        ),
        Err(Error::RtUnsupportedFormat)
    ));
    assert_eq!(f.count("rt_messages"), 0);
    assert!(f
        .db
        .connection
        .execute(
            "UPDATE rt_channels SET format=1 WHERE channel_id=?1",
            params![extended.0.as_slice()]
        )
        .is_err());
    assert!(f
        .db
        .connection
        .execute(
            "UPDATE rt_channels SET format=2 WHERE channel_id=?1",
            params![basic.metadata.id.0.as_slice()]
        )
        .is_err());
}
fn id(kind: u8, tag: u8) -> Vec<u8> {
    let mut b = vec![tag; 33];
    b[0] = kind;
    b
}
struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    owner: RealtimeActor,
    member: RealtimeActor,
    create: RtCreateChannelArgument,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("db"), Config::default()).unwrap();
        let host = id(ENTITY_HOST, 2);
        let team = id(ENTITY_NAMED_TEAM, 3);
        let actor = |tag| RealtimeActor {
            host: host.clone(),
            uid: id(ENTITY_USER, tag),
            credential: id(ENTITY_DEVICE, tag),
            certificate_expires_at: 1_000_000_000,
        };
        let owner = actor(7);
        let member = actor(8);
        for (a, kind) in [(&owner, 3), (&member, 1)] {
            let name = a.uid.clone();
            db.connection
                .execute(
                    "INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES (?1,1,?2)",
                    params![name, a.uid],
                )
                .unwrap();
            db.connection.execute("INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES (?1,?2,?2,1,zeroblob(16),0)",params![a.uid,name]).unwrap();
            db.connection.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES (?1,?2,1,3,0,zeroblob(32),zeroblob(17),X'00',X'00')",params![a.credential,a.uid]).unwrap();
            if kind == 3 {
                db.connection.execute("INSERT INTO team_names(normalized_name,reservation_sequence,team_id) VALUES (?1,1,?1)",params![team]).unwrap();
                db.connection.execute("INSERT INTO teams(team_id,team_kind,host_id,normalized_name,team_name_utf8,team_name_sequence,team_name_commitment_key,member_load_floor_type,member_load_floor_visibility,created_at) VALUES (?1,3,?2,?1,?1,1,zeroblob(16),1,0,0)",params![team,host]).unwrap();
            }
            db.connection.execute("INSERT INTO team_members(team_id,party_id,source_role_type,source_visibility,role_type,visibility,generation,verify_key,hepk_fingerprint) VALUES (?1,?2,3,0,?3,0,1,zeroblob(33),zeroblob(32))",params![team,a.uid,kind]).unwrap();
        }
        for (kind, visibility) in [(1, -0x4000), (1, 0), (2, 0), (3, 0)] {
            db.connection.execute("INSERT INTO team_shared_keys(team_id,role_type,visibility,generation,verify_key,exact_hepk) VALUES (?1,?2,?3,1,zeroblob(33),X'00')",params![team,kind,visibility]).unwrap();
        }
        let boxed = |role| RtBox {
            key: RoleAndGeneration {
                role,
                generation: 1,
            },
            boxed: SecretBox {
                nonce: [1; 16],
                ciphertext: vec![5; 20],
            },
        };
        let create = RtCreateChannelArgument {
            set_version: 1,
            metadata: RtChannelMetadata {
                id: RtChannelId([1; 16]),
                team: RtTeamId::new(EntityId::from_bytes(team).unwrap()).unwrap(),
                app: RtAppId::Chat,
                sequence: 1,
                name: boxed(MIN_ROLE),
                description: Some(boxed(Role::member(0))),
                roles: RtRolePair {
                    read: Role::member(0),
                    write: Role::member(0),
                },
                last_message: None,
                ctime: 0,
                mtime: 0,
                updated_at: 1,
                tier: RtChannelTier::Bottom,
                unreadable: false,
            },
        };
        Self {
            _dir: dir,
            db,
            owner,
            member,
            create,
        }
    }
    fn send(&self, tag: u8) -> RtSendArgument {
        RtSendArgument {
            send: RtSend {
                metadata: RtMessageMetadata {
                    id: RtMessageId([tag; 16]),
                    previous_id: RtMessageId([0; 16]),
                    previous_sequence: 0,
                    send_time: 1000,
                    kind: RtMessageType::Basic,
                    further_user_attribution: None,
                },
                channel: self.create.metadata.id.short(),
                wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                    key: RoleAndGeneration {
                        role: Role::member(0),
                        generation: 1,
                    },
                    ciphertext: RtCiphertext(vec![tag; 20]),
                }),
                expected_previous_sequence: 0,
            },
        }
    }
    fn reader(&self) -> ReadDatabase {
        ReadDatabase::open(&self.db.path, Config::default()).unwrap()
    }
    fn count(&self, table: &str) -> i64 {
        self.db
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
#[test]
fn exact_replay_conflict_order_history_and_restart() {
    let mut f = Fixture::new();
    assert_eq!(
        f.db.rt_create_channel(&f.owner, &f.create, 1_000_000)
            .unwrap()
            .wake
            .len(),
        2
    );
    let first = f.send(2);
    let receipt = f.db.rt_send(&f.owner, &first, 1_001_000).unwrap();
    assert_eq!(receipt.value.sequence, 1);
    let replay = f.db.rt_send(&f.owner, &first, 1_100_000).unwrap();
    assert_eq!(receipt.value, replay.value);
    assert!(replay.wake.is_empty());
    assert_eq!(f.count("rt_messages"), 1);
    assert!(matches!(
        f.db.rt_send(&f.member, &first, 1_200_000),
        Err(Error::ReceiptConflict)
    ));
    let mut changed = first.clone();
    changed.send.metadata.send_time += 1;
    assert!(matches!(
        f.db.rt_send(&f.owner, &changed, 1_200_000),
        Err(Error::ReceiptConflict)
    ));
    let mut second = f.send(3);
    second.send.expected_previous_sequence = 2;
    assert!(matches!(
        f.db.rt_send(&f.member, &second, 1_200_000),
        Err(Error::RtMessageOrder)
    ));
    second.send.expected_previous_sequence = 1;
    second.send.metadata.previous_id = first.send.metadata.id;
    second.send.metadata.previous_sequence = 1;
    assert_eq!(
        f.db.rt_send(&f.member, &second, 1_200_000)
            .unwrap()
            .value
            .sequence,
        2
    );
    let snapshot = f.reader();
    let snapshot = snapshot.snapshot().unwrap();
    let recent = snapshot
        .rt_recents(
            &f.member,
            &RtRecentsArgument {
                channel: f.create.metadata.id,
                stop_at: 1,
                limit: 10,
            },
            1_300_000,
        )
        .unwrap();
    assert_eq!(
        recent
            .messages
            .iter()
            .map(|m| m.sequence)
            .collect::<Vec<_>>(),
        [2]
    );
    let page = snapshot
        .rt_thread(
            &f.member,
            &RtGetThreadArgument {
                query: RtThreadQuery {
                    channel: f.create.metadata.id,
                    ranges: vec![
                        RtThreadRange { start: 2, end: 1 },
                        RtThreadRange { start: 1, end: 2 },
                    ],
                    sequences: vec![99, 1, 1],
                },
            },
            1_300_000,
        )
        .unwrap();
    assert_eq!(
        page.ranges[0]
            .messages
            .iter()
            .map(|m| m.sequence)
            .collect::<Vec<_>>(),
        [2, 1]
    );
    assert_eq!(page.ranges[1].messages[0].sequence, 1);
    assert_eq!(page.sequences.len(), 1);
    drop(snapshot);
    let path = f.db.path.clone();
    drop(f.db);
    f.db = Database::open_existing(path, Config::default()).unwrap();
    assert_eq!(
        f.db.rt_send(&f.owner, &first, 1_400_000).unwrap().value,
        receipt.value
    );
    let read: i64 =
        f.db.connection
            .query_row(
                "SELECT read_through FROM rt_user_channels WHERE uid=?1",
                params![f.member.uid],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(read, 2);
}
#[test]
fn current_membership_credentials_and_generation_control_every_operation() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    let first = f.send(2);
    f.db.rt_send(&f.owner, &first, 100).unwrap();
    f.db.connection.execute("INSERT INTO team_shared_keys SELECT team_id,role_type,visibility,2,verify_key,exact_hepk,start_epoch FROM team_shared_keys WHERE role_type=1 AND visibility=0",[]).unwrap();
    assert!(f.db.rt_send(&f.owner, &first, 200).is_ok());
    assert!(matches!(
        f.db.rt_send(&f.owner, &f.send(3), 200),
        Err(Error::RtRace)
    ));
    f.db.connection
        .execute(
            "DELETE FROM team_members WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    assert!(matches!(
        f.db.rt_send(&f.member, &first, 200),
        Err(Error::AuthorizationChanged)
    ));
    assert!(matches!(
        f.reader().snapshot().unwrap().rt_recents(
            &f.member,
            &RtRecentsArgument {
                channel: f.create.metadata.id,
                stop_at: 0,
                limit: 1
            },
            200
        ),
        Err(Error::AuthorizationChanged)
    ));
    f.db.connection
        .execute(
            "UPDATE devices SET active=0 WHERE device_id=?1",
            params![f.owner.credential],
        )
        .unwrap();
    assert!(matches!(
        f.db.rt_send(&f.owner, &first, 200),
        Err(Error::AuthorizationChanged)
    ));
    assert_eq!(f.count("rt_messages"), 1);
}
#[test]
fn tier_visibility_cas_identity_collisions_and_limits() {
    let mut f = Fixture::new();
    let mut admin = f.create.clone();
    admin.metadata.tier = RtChannelTier::Admin;
    admin.metadata.roles = RtRolePair {
        read: Role::ADMIN,
        write: Role::ADMIN,
    };
    admin.metadata.name.key.role = Role::ADMIN;
    admin.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
    assert!(matches!(
        f.db.rt_create_channel(&f.member, &admin, 100),
        Err(Error::AuthorizationChanged)
    ));
    f.db.rt_create_channel(&f.owner, &admin, 100).unwrap();
    let mut restricted = f.create.clone();
    restricted.metadata.id.0 = [2; 16];
    restricted.set_version = 2;
    restricted.metadata.updated_at = 2;
    restricted.metadata.roles = RtRolePair {
        read: Role::ADMIN,
        write: Role::ADMIN,
    };
    restricted.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
    f.db.rt_create_channel(&f.owner, &restricted, 100).unwrap();
    let mut list = RtListChannelsArgument {
        team: restricted.metadata.team.clone(),
        app: RtAppId::Chat,
        last: 0,
    };
    let page = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_list_channels(&f.member, &list, 200)
        .unwrap();
    assert_eq!(page.channels.len(), 1);
    assert!(page.channels[0].unreadable);
    assert!(page.channels[0].description.is_none());
    list.last = 2;
    assert!(f
        .reader()
        .snapshot()
        .unwrap()
        .rt_list_channels(&f.owner, &list, 200)
        .unwrap()
        .channels
        .is_empty());
    let mut collision = restricted.clone();
    collision.metadata.id.0[15] ^= 1;
    assert!(matches!(
        f.db.rt_create_channel(&f.owner, &collision, 200),
        Err(Error::Duplicate(_))
    ));
    collision.metadata.id.0 = [3; 16];
    assert!(matches!(
        f.db.rt_create_channel(&f.owner, &collision, 200),
        Err(Error::RtRace)
    ));
    let mut wrong = f.owner.clone();
    wrong.host[1] ^= 1;
    assert!(matches!(
        f.db.rt_create_channel(&wrong, &collision, 200),
        Err(Error::AuthorizationChanged)
    ));
    wrong = f.owner.clone();
    wrong.certificate_expires_at = 200;
    assert!(matches!(
        f.db.rt_create_channel(&wrong, &collision, 200),
        Err(Error::AuthorizationChanged)
    ));
    assert!(f
        .reader()
        .snapshot()
        .unwrap()
        .rt_recents(
            &f.owner,
            &RtRecentsArgument {
                channel: restricted.metadata.id,
                stop_at: u64::MAX,
                limit: 1
            },
            200
        )
        .is_err());
}
#[test]
fn fanout_failure_rolls_back_message_sequence_and_receipt() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    f.db.connection
        .execute(
            "UPDATE rt_user_inboxes SET version=?1 WHERE uid=?2",
            params![i64::MAX, f.member.uid],
        )
        .unwrap();
    let send = f.send(2);
    assert!(matches!(
        f.db.rt_send(&f.owner, &send, 200),
        Err(Error::IntegerRange)
    ));
    assert_eq!(f.count("rt_messages"), 0);
    let last: i64 =
        f.db.connection
            .query_row("SELECT last_sequence FROM rt_channels", [], |r| r.get(0))
            .unwrap();
    assert_eq!(last, 0);
    f.db.connection
        .execute("UPDATE rt_user_inboxes SET version=1", [])
        .unwrap();
    assert_eq!(
        f.db.rt_send(&f.owner, &send, 200).unwrap().value.sequence,
        1
    );
}
#[test]
fn contended_writes_recheck_authority_and_allocate_contiguous_sequences() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    let path = f.db.path.clone();
    let mut queued = Database::open_existing(&path, Config::default()).unwrap();
    let tx =
        f.db.connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
    tx.execute(
        "UPDATE team_members SET role_type=1,visibility=-1 WHERE party_id=?1",
        params![f.member.uid],
    )
    .unwrap();
    let actor = f.member.clone();
    let send = RtSendArgument {
        send: RtSend {
            metadata: RtMessageMetadata {
                id: RtMessageId([9; 16]),
                previous_id: RtMessageId([0; 16]),
                previous_sequence: 0,
                send_time: 1,
                kind: RtMessageType::Basic,
                further_user_attribution: None,
            },
            channel: f.create.metadata.id.short(),
            wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                key: RoleAndGeneration {
                    role: Role::member(0),
                    generation: 1,
                },
                ciphertext: RtCiphertext(vec![1; 20]),
            }),
            expected_previous_sequence: 0,
        },
    };
    let (ready, started) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        queued.rt_send(&actor, &send, 200)
    });
    started.recv().unwrap();
    tx.commit().unwrap();
    assert!(matches!(
        worker.join().unwrap(),
        Err(Error::AuthorizationChanged)
    ));
    let mut workers = Vec::new();
    for tag in 2..10 {
        let mut db = Database::open_existing(&path, Config::default()).unwrap();
        let actor = f.owner.clone();
        let send = f.send(tag);
        workers.push(std::thread::spawn(move || {
            db.rt_send(&actor, &send, 200).unwrap().value.sequence
        }));
    }
    let mut seqs: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    seqs.sort();
    assert_eq!(seqs, (1..=8).collect::<Vec<_>>());
    assert_eq!(f.count("rt_messages"), 8);
}
#[test]
fn restricted_activity_is_hidden_and_new_direct_members_can_discover() {
    let mut f = Fixture::new();
    let mut restricted = f.create.clone();
    restricted.metadata.roles = RtRolePair {
        read: Role::ADMIN,
        write: Role::ADMIN,
    };
    restricted.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
    f.db.rt_create_channel(&f.owner, &restricted, 1000).unwrap();
    let mut send = f.send(2);
    if let RtMessageWrapper::Encrypted(boxed) = &mut send.send.wrapper {
        boxed.key.role = Role::ADMIN;
    }
    f.db.rt_send(&f.owner, &send, 5000).unwrap();
    let list = RtListChannelsArgument {
        team: restricted.metadata.team.clone(),
        app: RtAppId::Chat,
        last: 0,
    };
    let hidden = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_list_channels(&f.member, &list, 6000)
        .unwrap();
    assert!(hidden.channels[0].unreadable);
    assert_eq!(hidden.channels[0].mtime, hidden.channels[0].ctime);
    assert!(hidden.channels[0].last_message.is_none());
    f.db.connection
        .execute(
            "UPDATE team_members SET role_type=2 WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    assert_eq!(
        f.db.connection
            .query_row(
                "SELECT COUNT(*) FROM rt_user_channels WHERE uid=?1",
                params![f.member.uid],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let visible = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_list_channels(&f.member, &list, 6000)
        .unwrap();
    assert!(!visible.channels[0].unreadable);
    assert_eq!(
        visible.channels[0].last_message.as_ref().unwrap().sequence,
        1
    );
    assert_eq!(
        f.reader()
            .snapshot()
            .unwrap()
            .rt_recents(
                &f.member,
                &RtRecentsArgument {
                    channel: restricted.metadata.id,
                    stop_at: 0,
                    limit: 1
                },
                6000
            )
            .unwrap()
            .messages
            .len(),
        1
    );
    f.db.connection
        .execute(
            "UPDATE team_members SET source_role_type=1 WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    assert!(matches!(
        f.reader()
            .snapshot()
            .unwrap()
            .rt_list_channels(&f.member, &list, 6000),
        Err(Error::AuthorizationChanged)
    ));
    let mut recovery = f.owner.clone();
    recovery.credential[0] = ENTITY_BACKUP_KEY;
    assert!(matches!(
        f.db.rt_send(&recovery, &send, 6000),
        Err(Error::AuthorizationChanged)
    ));
}
#[test]
fn maximum_channel_metadata_can_accept_a_message_and_replay_ignores_precondition() {
    let mut f = Fixture::new();
    let mut create = f.create.clone();
    create.metadata.ctime = 1;
    create.metadata.mtime = 1;
    create
        .metadata
        .name
        .boxed
        .ciphertext
        .resize(Limits::CHANNEL_CONFIGURATION_BYTES - 500, 4);
    let size = create.metadata.encoded().unwrap().len();
    create.metadata.name.boxed.ciphertext.resize(
        create.metadata.name.boxed.ciphertext.len() + Limits::CHANNEL_CONFIGURATION_BYTES - size,
        4,
    );
    assert_eq!(
        create.metadata.encoded().unwrap().len(),
        Limits::CHANNEL_CONFIGURATION_BYTES
    );
    f.db.rt_create_channel(&f.owner, &create, 1000).unwrap();
    let mut send = f.send(2);
    let receipt = f.db.rt_send(&f.owner, &send, 2000).unwrap().value;
    let stored: Vec<u8> =
        f.db.connection
            .query_row(
                "SELECT metadata FROM rt_channels WHERE channel_id=?1",
                params![create.metadata.id.0.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(stored, create.metadata.encoded().unwrap());
    let projected = channel(&f.db.connection, &create.metadata.id.0).unwrap();
    assert_eq!(projected.mtime, 2);
    assert_eq!(projected.last_message.unwrap().sequence, receipt.sequence);
    send.send.expected_previous_sequence = u64::MAX;
    assert_eq!(f.db.rt_send(&f.owner, &send, 3000).unwrap().value, receipt);
    let mut too_large = create;
    too_large.metadata.id.0 = [3; 16];
    too_large.metadata.name.boxed.ciphertext.push(4);
    assert!(matches!(
        f.db.rt_create_channel(&f.owner, &too_large, 1000),
        Err(Error::Capacity(_))
    ));
}
#[test]
fn concurrent_channel_creates_have_one_cas_winner() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for tag in [20, 21] {
        let mut db = Database::open_existing(&f.db.path, Config::default()).unwrap();
        let actor = f.owner.clone();
        let mut arg = f.create.clone();
        arg.metadata.id = RtChannelId([tag; 16]);
        arg.set_version = 2;
        arg.metadata.updated_at = 2;
        let gate = gate.clone();
        workers.push(std::thread::spawn(move || {
            gate.wait();
            db.rt_create_channel(&actor, &arg, 200)
        }));
    }
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(Error::RtRace)))
            .count(),
        1
    );
    assert_eq!(f.count("rt_channels"), 2);
}
#[test]
fn inbox_delta_read_through_and_membership_reconciliation_are_monotonic() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    f.db.rt_send(&f.owner, &f.send(2), 200).unwrap();
    let query = |since| RtGetChangedThreadsArgument {
        query: RtChangedThreads {
            app: RtAppId::Chat,
            since,
            maximum: 100,
        },
    };
    let initial = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(&f.member, &query(0), 300)
        .unwrap();
    assert_eq!(initial.inbox_version, 2);
    assert_eq!(initial.channels.len(), 1);
    assert_eq!(initial.channels[0].read_through, 0);
    let marked =
        f.db.rt_read_through(
            &f.member,
            &RtReadThroughArgument {
                read: RtReadThrough {
                    channel: f.create.metadata.id,
                    sequence: 1,
                },
            },
            400,
        )
        .unwrap();
    assert_eq!(marked.wake.len(), 1);
    let stale =
        f.db.rt_read_through(
            &f.member,
            &RtReadThroughArgument {
                read: RtReadThrough {
                    channel: f.create.metadata.id,
                    sequence: 1,
                },
            },
            500,
        )
        .unwrap();
    assert!(stale.wake.is_empty());
    let changed = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(&f.member, &query(2), 600)
        .unwrap();
    assert_eq!(changed.inbox_version, 3);
    assert_eq!(changed.channels[0].read_through, 1);
    assert!(matches!(
        f.db.rt_read_through(
            &f.member,
            &RtReadThroughArgument {
                read: RtReadThrough {
                    channel: f.create.metadata.id,
                    sequence: 2,
                },
            },
            700
        ),
        Err(Error::NotFound(_))
    ));

    let mut restricted = f.create.clone();
    restricted.metadata.id = RtChannelId([2; 16]);
    restricted.metadata.updated_at = 2;
    restricted.set_version = 2;
    restricted.metadata.roles = RtRolePair {
        read: Role::ADMIN,
        write: Role::ADMIN,
    };
    restricted.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
    f.db.rt_create_channel(&f.owner, &restricted, 800).unwrap();
    assert_eq!(
        f.db.connection
            .query_row(
                "SELECT COUNT(*) FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
                params![f.member.uid, restricted.metadata.id.0.as_slice()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    f.db.connection
        .execute(
            "UPDATE team_members SET role_type=2,visibility=0 WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    let reconciled =
        f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 900)
            .unwrap();
    assert_eq!(reconciled.wake.len(), 1);
    let after = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(&f.member, &query(3), 1000)
        .unwrap();
    assert_eq!(after.channels.len(), 1);
    assert_eq!(after.channels[0].metadata.id, restricted.metadata.id);
    let mut restricted_send = f.send(3);
    restricted_send.send.channel = restricted.metadata.id.short();
    let RtMessageWrapper::Encrypted(boxed) = &mut restricted_send.send.wrapper else {
        panic!()
    };
    boxed.key.role = Role::ADMIN;
    f.db.rt_send(&f.owner, &restricted_send, 1010).unwrap();
    f.db.rt_read_through(
        &f.member,
        &RtReadThroughArgument {
            read: RtReadThrough {
                channel: restricted.metadata.id,
                sequence: 1,
            },
        },
        1020,
    )
    .unwrap();
    f.db.connection
        .execute(
            "UPDATE team_members SET role_type=1,visibility=0 WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    let reconciled =
        f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 1050)
            .unwrap();
    assert_eq!(reconciled.wake.len(), 1);
    let filtered = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(&f.member, &query(0), 1100)
        .unwrap();
    assert_eq!(filtered.inbox_version, 7);
    assert_eq!(filtered.channels.len(), 1);
    assert_eq!(filtered.channels[0].metadata.id, f.create.metadata.id);
    assert_eq!(
        f.db.connection
            .query_row(
                "SELECT read_through,accessible FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
                params![f.member.uid, restricted.metadata.id.0.as_slice()],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
        (1, 0)
    );
    f.db.connection
        .execute(
            "UPDATE team_members SET role_type=2,visibility=0 WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
    f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 1150)
        .unwrap();
    let restored = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(&f.member, &query(7), 1200)
        .unwrap();
    assert_eq!(restored.inbox_version, 8);
    assert_eq!(restored.channels.len(), 1);
    assert_eq!(restored.channels[0].metadata.id, restricted.metadata.id);
    assert_eq!(restored.channels[0].read_through, 1);
}

#[test]
fn inbox_reconciliation_ignores_foreign_host_channels_above_the_work_bound() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    let foreign_team = id(ENTITY_NAMED_TEAM, 9);
    f.db.connection
        .execute(
            "INSERT INTO team_names(normalized_name,reservation_sequence,team_id) VALUES (?1,1,?1)",
            params![foreign_team],
        )
        .unwrap();
    f.db.connection
        .execute(
            "INSERT INTO teams(team_id,team_kind,host_id,normalized_name,team_name_utf8,team_name_sequence,team_name_commitment_key,member_load_floor_type,member_load_floor_visibility,created_at) VALUES (?1,3,?2,?1,?1,1,zeroblob(16),1,0,0)",
            params![foreign_team, f.owner.host],
        )
        .unwrap();
    f.db.connection
        .execute(
            "INSERT INTO rt_channel_sets(team_id,app_id,version,mtime) VALUES (?1,1,1,0)",
            params![foreign_team],
        )
        .unwrap();
    let tx = f.db.connection.transaction().unwrap();
    for index in 0..=Limits::INBOX_RECONCILE_CHANNELS {
        let mut channel = [0xfe; 16];
        channel[..8].copy_from_slice(&(index as u64).to_be_bytes());
        tx.execute(
            "INSERT INTO rt_channels(channel_id,short_id,team_id,app_id,metadata) VALUES (?1,?2,?3,1,X'00')",
            params![
                channel.as_slice(),
                1_000_000i64 + index as i64,
                foreign_team
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let reconciled =
        f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 200)
            .unwrap();
    assert!(reconciled.wake.is_empty());
}

#[test]
fn read_state_converges_across_two_devices_of_one_user() {
    let mut f = Fixture::new();
    let mut other_device = f.member.clone();
    other_device.credential = id(ENTITY_DEVICE, 9);
    f.db.connection
        .execute(
            "INSERT INTO devices(device_id,uid,active,role_type,visibility,hepk_fingerprint,self_token,exact_hepk,exact_name)
             VALUES (?1,?2,1,3,0,zeroblob(32),?3,X'00',X'00')",
            params![other_device.credential, other_device.uid, vec![9u8; 17]],
        )
        .unwrap();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    f.db.rt_send(&f.owner, &f.send(2), 200).unwrap();
    let query = |since| RtGetChangedThreadsArgument {
        query: RtChangedThreads {
            app: RtAppId::Chat,
            since,
            maximum: 100,
        },
    };
    let before = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(&other_device, &query(0), 300)
        .unwrap();
    assert_eq!(before.channels[0].read_through, 0);
    f.db.rt_read_through(
        &f.member,
        &RtReadThroughArgument {
            read: RtReadThrough {
                channel: f.create.metadata.id,
                sequence: 1,
            },
        },
        400,
    )
    .unwrap();
    let after = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(&other_device, &query(2), 500)
        .unwrap();
    assert_eq!(after.inbox_version, 3);
    assert_eq!(after.channels[0].read_through, 1);
}

#[test]
fn inbox_page_scans_past_inaccessible_rows() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    let mut second = f.create.clone();
    second.metadata.id = RtChannelId([2; 16]);
    second.metadata.updated_at = 2;
    second.set_version = 2;
    f.db.rt_create_channel(&f.owner, &second, 200).unwrap();
    let mut hidden = f.create.metadata.clone();
    hidden.roles = RtRolePair {
        read: Role::ADMIN,
        write: Role::ADMIN,
    };
    hidden.description.as_mut().unwrap().key.role = Role::ADMIN;
    f.db.connection
        .execute(
            "UPDATE rt_channels SET metadata=?2 WHERE channel_id=?1",
            params![hidden.id.0.as_slice(), hidden.encoded().unwrap()],
        )
        .unwrap();
    let delta = f
        .reader()
        .snapshot()
        .unwrap()
        .rt_changed_threads(
            &f.member,
            &RtGetChangedThreadsArgument {
                query: RtChangedThreads {
                    app: RtAppId::Chat,
                    since: 0,
                    maximum: 1,
                },
            },
            300,
        )
        .unwrap();
    assert_eq!(delta.channels.len(), 1);
    assert_eq!(delta.channels[0].metadata.id, second.metadata.id);
    assert_eq!(delta.channels[0].inbox_version, 2);
}

#[test]
fn response_byte_limits_fail_explicitly_without_truncating_history() {
    let mut f = Fixture::new();
    f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
    for tag in 2..11 {
        let mut send = f.send(tag);
        if let RtMessageWrapper::Encrypted(boxed) = &mut send.send.wrapper {
            boxed.ciphertext.0.resize(RT_MAX_BODY_BYTES, tag);
        }
        f.db.rt_send(&f.owner, &send, 200).unwrap();
    }
    let reader = f.reader();
    let snapshot = reader.snapshot().unwrap();
    assert!(matches!(
        snapshot.rt_recents(
            &f.owner,
            &RtRecentsArgument {
                channel: f.create.metadata.id,
                stop_at: 0,
                limit: 9
            },
            300
        ),
        Err(Error::Capacity(_))
    ));
    assert_eq!(
        snapshot
            .rt_recents(
                &f.owner,
                &RtRecentsArgument {
                    channel: f.create.metadata.id,
                    stop_at: 0,
                    limit: 8
                },
                300
            )
            .unwrap()
            .messages
            .len(),
        8
    );
    assert!(snapshot
        .rt_thread(
            &f.owner,
            &RtGetThreadArgument {
                query: RtThreadQuery {
                    channel: f.create.metadata.id,
                    ranges: vec![RtThreadRange { start: 1, end: 9 }],
                    sequences: vec![]
                }
            },
            300
        )
        .is_err());
}

mod reconciliation;
