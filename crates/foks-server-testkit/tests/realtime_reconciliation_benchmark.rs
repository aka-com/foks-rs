//! Opt-in and baseline-compatible: uses only public APIs that predate the gate.
#[path = "conformance/support.rs"]
mod support;
use foks_proto::*;
use foks_rpc::{RealtimeRequest as Request, RealtimeResponse as Response};
use foks_server_testkit::TestAccountSpec;
use std::time::{Duration, Instant};
use support::Fixture;

fn delta() -> Request {
    Request::GetChangedThreads(RtGetChangedThreadsArgument {
        query: RtChangedThreads {
            app: RtAppId::Chat,
            since: 0,
            maximum: 100,
        },
    })
}
fn poll(since: u64, timeout_milliseconds: u64) -> Request {
    Request::PollInbox(RtPollInboxArgument {
        poll: RtPollInbox {
            app: RtAppId::Chat,
            since,
            timeout_milliseconds,
        },
    })
}
fn head(c: &mut foks_client::RealtimeConnection) -> u64 {
    let Response::InboxVersion(v) = c
        .call(&Request::GetInboxVersion(RtGetInboxVersionArgument {
            key: RtInboxKey { app: RtAppId::Chat },
        }))
        .unwrap()
    else {
        panic!()
    };
    v
}
fn percentile(samples: &mut [u128], percent: usize) -> u128 {
    samples.sort_unstable();
    samples[((samples.len() - 1) * percent) / 100]
}

#[test]
#[ignore = "release-mode realtime benchmark; see docs/realtime-inbox-reconciliation-plan.md"]
fn realtime_reconciliation_benchmark() {
    for memberships in [1, 10, 100] {
        for polls in [1, 8] {
            let f = Fixture::start("rt-reconcile-benchmark");
            let account = f
                .client
                .create_account(f.host(), &TestAccountSpec::new("rtbench", 0x37))
                .unwrap();
            let secrets = foks_client::NamedTeamSecrets {
                member_min: SecretSeed::new([0x51; 32]),
                member: SecretSeed::new([0x52; 32]),
                admin: SecretSeed::new([0x53; 32]),
                owner: SecretSeed::new([0x54; 32]),
                removal_key: SecretSeed::new([0x55; 32]),
                team_name_commitment_key: [0x56; 16],
            };
            let team = f
                .client
                .foks()
                .create_single_owner_named_team(f.host(), &account.credential, "rtbench", &secrets)
                .unwrap()
                .team;
            // Seed additional roster projections on the writer. Building 100
            // signed teams hits an unrelated client operation-history depth limit.
            // These extra teams have no channels; they exercise membership size.
            let path = f.environment.database_path().to_path_buf();
            let original = team.as_bytes().to_vec();
            f.server.writer_handle().call(move |_| {
                let mut c = rusqlite::Connection::open(path).unwrap();
                c.pragma_update(None, "foreign_keys", true).unwrap();
                let tx = c.transaction().unwrap();
                for i in 1..memberships {
                    let mut id = original.clone();
                    id[1] ^= 0xff;
                    id[2] = i as u8;
                    let name = format!("rtbench{i}").into_bytes();
                    tx.execute("INSERT INTO team_names(normalized_name,reservation_sequence,team_id) VALUES (?1,1,?2)", rusqlite::params![name, id]).unwrap();
                    tx.execute("INSERT INTO teams SELECT ?1,team_kind,host_id,?2,?2,team_name_sequence,team_name_commitment_key,member_load_floor_type,member_load_floor_visibility,created_at FROM teams WHERE team_id=?3", rusqlite::params![id,name,original]).unwrap();
                    tx.execute("INSERT INTO team_members SELECT ?1,party_id,scoped_host_id,source_role_type,source_visibility,role_type,visibility,generation,verify_key,hepk_fingerprint,removal_key_commitment FROM team_members WHERE team_id=?2", rusqlite::params![id,original]).unwrap();
                }
                tx.commit().unwrap();
                Ok(())
            }).unwrap();
            let name_keys =
                foks_crypto::derive_realtime_keys(&secrets.member_min, RtAppId::Chat).unwrap();
            let data_keys =
                foks_crypto::derive_realtime_keys(&secrets.member, RtAppId::Chat).unwrap();
            let channel = RtChannelId([0x62; 16]);
            let md = RtChannelMetadata {
                id: channel,
                team: RtTeamId::new(team.clone()).unwrap(),
                app: RtAppId::Chat,
                sequence: 1,
                name: RtBox {
                    key: RoleAndGeneration {
                        role: Role::member(-0x4000),
                        generation: 1,
                    },
                    boxed: name_keys
                        .seal_text(RtKeyType::ChannelName, &RtText("bench".into()), [1; 16])
                        .unwrap(),
                },
                description: None,
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
            };
            let mut ordinary = f
                .client
                .foks()
                .realtime_connection(f.host(), &account.credential)
                .unwrap();
            ordinary
                .call(&Request::CreateChannel(RtCreateChannelArgument {
                    metadata: md,
                    set_version: 1,
                }))
                .unwrap();
            ordinary.call(&delta()).unwrap();
            let mut connections = (0..polls)
                .map(|_| {
                    f.client
                        .foks()
                        .realtime_connection(f.host(), &account.credential)
                        .unwrap()
                })
                .collect::<Vec<_>>();
            let before = f.server.writer_handle().metrics();
            let started = Instant::now();
            let since = head(&mut ordinary);
            let waiting = connections
                .drain(..)
                .map(|mut c| {
                    std::thread::spawn(move || {
                        let Response::PollResult(result) = c.call(&poll(since, 2200)).unwrap()
                        else {
                            panic!()
                        };
                        assert!(!result.bumped);
                        c
                    })
                })
                .collect::<Vec<_>>();
            for thread in waiting {
                connections.push(thread.join().unwrap());
            }
            for _ in 0..10 {
                ordinary.call(&delta()).unwrap();
            }
            let after = f.server.writer_handle().metrics();
            let submissions = after.accepted - before.accepted;
            println!("RT_IDLE polls={polls} memberships={memberships} devices=1 elapsed_ms={} writer_submissions={submissions} queue_us={} execution_us={}", started.elapsed().as_millis(), after.queue_wait_microseconds_total-before.queue_wait_microseconds_total, after.execution_microseconds_total-before.execution_microseconds_total);
            if std::env::var_os("FOKS_EXPECT_IDLE_RECONCILE_GATE").is_some() {
                assert_eq!(submissions, 0);
            }

            let mut sequence = 0;
            let mut previous_id = RtMessageId([0; 16]);
            for active in [false, true] {
                let mut sends = Vec::new();
                let mut wakes = Vec::new();
                let before = f.server.writer_handle().metrics();
                for _ in 0..10 {
                    let since = head(&mut ordinary);
                    let waiting = if active {
                        connections
                            .drain(..)
                            .map(|mut c| {
                                std::thread::spawn(move || {
                                    let Response::PollResult(result) =
                                        c.call(&poll(since, 5000)).unwrap()
                                    else {
                                        panic!()
                                    };
                                    assert!(result.bumped);
                                    (c, Instant::now())
                                })
                            })
                            .collect::<Vec<_>>()
                    } else {
                        vec![]
                    };
                    if active {
                        let deadline = Instant::now() + Duration::from_secs(3);
                        while f.server.metrics().active_realtime_polls != polls {
                            assert!(Instant::now() < deadline);
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                    let id = RtMessageId([sequence as u8 + 1; 16]);
                    let noncer = RtMessageNoncer {
                        metadata: RtMessageMetadata {
                            id,
                            previous_id,
                            previous_sequence: sequence,
                            send_time: 1700000000000,
                            kind: RtMessageType::Basic,
                            further_user_attribution: None,
                        },
                        sender: Some(
                            FqParty::new(
                                account.credential.uid.clone(),
                                f.host().host_id().clone(),
                            )
                            .unwrap(),
                        ),
                        app: RtAppId::Chat,
                        team: FqParty::new(team.clone(), f.host().host_id().clone()).unwrap(),
                        channel,
                    };
                    let request = Request::Send(RtSendArgument {
                        send: RtSend {
                            metadata: noncer.metadata.clone(),
                            channel: channel.short(),
                            wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                                key: RoleAndGeneration {
                                    role: Role::member(0),
                                    generation: 1,
                                },
                                ciphertext: data_keys
                                    .seal_basic_message(&noncer, b"benchmark")
                                    .unwrap(),
                            }),
                            expected_previous_sequence: sequence,
                        },
                    });
                    let start = Instant::now();
                    ordinary.call(&request).unwrap();
                    sends.push(start.elapsed().as_micros());
                    for thread in waiting {
                        let (connection, woke) = thread.join().unwrap();
                        wakes.push(woke.duration_since(start).as_micros());
                        connections.push(connection);
                    }
                    sequence += 1;
                    previous_id = id;
                }
                let after = f.server.writer_handle().metrics();
                println!("RT_SEND polls={} memberships={memberships} messages=10 p50_us={} p95_us={} wake_p95_us={} writer_submissions={} queue_us={} execution_us={}", if active { polls } else { 0 }, percentile(&mut sends, 50), percentile(&mut sends, 95), if wakes.is_empty() { 0 } else { percentile(&mut wakes, 95) }, after.accepted-before.accepted, after.queue_wait_microseconds_total-before.queue_wait_microseconds_total, after.execution_microseconds_total-before.execution_microseconds_total);
            }
            // An access edit without a notification exercises durable recovery.
            let path = f.environment.database_path().to_path_buf();
            let uid = account.credential.uid.as_bytes().to_vec();
            let team_id = team.as_bytes().to_vec();
            f.server.writer_handle().call(move |_| {
                let c = rusqlite::Connection::open(path).unwrap();
                c.execute("UPDATE team_members SET scoped_host_id=zeroblob(33) WHERE party_id=?1 AND team_id=?2", rusqlite::params![uid, team_id]).unwrap();
                Ok(())
            }).unwrap();
            let start = Instant::now();
            let Response::InboxDelta(changed) = ordinary.call(&delta()).unwrap() else {
                panic!()
            };
            assert!(changed.channels.is_empty());
            println!(
                "RT_EDIT polls={polls} memberships={memberships} delta_us={}",
                start.elapsed().as_micros()
            );
        }
    }
}
