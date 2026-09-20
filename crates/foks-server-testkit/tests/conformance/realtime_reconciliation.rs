use crate::support::Fixture;
use foks_proto::*;
use foks_rpc::{RealtimeRequest as Request, RealtimeResponse as Response};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};
use std::time::{Duration, Instant};

fn fixture(profile: TestProfile) -> Fixture {
    let environment = TestEnvironment::with_profile(profile).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "rt-reconciliation").unwrap();
    let probe = client.probe_and_pin().unwrap();
    Fixture {
        environment,
        server,
        client,
        probe,
    }
}
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
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "readiness deadline exceeded");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn clean_delta_and_idle_poll_never_submit_reconciliation() {
    let f = fixture(TestProfile::Default);
    let account = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("rtidle", 0x33))
        .unwrap();
    let mut connection = f
        .client
        .foks()
        .realtime_connection(f.host(), &account.credential)
        .unwrap();
    connection.call(&delta()).unwrap();
    let before = f.server.metrics().realtime;
    for _ in 0..5 {
        connection.call(&delta()).unwrap();
    }
    let Response::PollResult(result) = connection.call(&poll(0, 2200)).unwrap() else {
        panic!()
    };
    assert!(!result.bumped);
    let after = f.server.metrics().realtime;
    assert_eq!(after.delta.submissions, before.delta.submissions);
    assert_eq!(after.poll.submissions, before.poll.submissions);
    assert_eq!(after.delta.clean_skips - before.delta.clean_skips, 5);
    assert!(after.poll.state_reads - before.poll.state_reads >= 3);
    assert!(after.fallback_wakes - before.fallback_wakes >= 2);
    assert_eq!(after.timed_out_polls - before.timed_out_polls, 1);
}

#[test]
fn dirty_delta_returns_single_reader_lease_while_queued_and_rechecks_expiry() {
    let f = fixture(TestProfile::RealtimeSingleReader);
    let account = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("rtlease", 0x34))
        .unwrap();
    let mut connection = f
        .client
        .foks()
        .realtime_connection(f.host(), &account.credential)
        .unwrap();
    let mut other = f
        .client
        .foks()
        .realtime_connection(f.host(), &account.credential)
        .unwrap();
    let pressure = f.server.saturate_writer_queue().unwrap();
    let waiting = std::thread::spawn(move || connection.call(&delta()));
    wait(|| f.server.writer_handle().metrics().pending == 3);
    // This request must obtain the only reader while the delta is waiting for the writer.
    assert_eq!(
        other
            .call(&Request::GetInboxVersion(RtGetInboxVersionArgument {
                key: RtInboxKey { app: RtAppId::Chat }
            }))
            .unwrap(),
        Response::InboxVersion(0)
    );
    // Expire the presented certificate after the reader check but before execution.
    f.environment.advance_clock(366 * 24 * 60 * 60 * 1_000_000);
    pressure.drain().unwrap();
    assert!(waiting.join().unwrap().is_err());
    let metrics = f.server.metrics().realtime.delta;
    assert_eq!(metrics.submissions, 1);
    assert_eq!(metrics.execution_failures, 1);
    assert_eq!(metrics.execution_buckets[8], 1);
    assert_eq!(metrics.queue_wait_buckets[8], 1);
}

#[test]
fn fallback_observes_unannounced_commit_and_revocation_on_parked_poll() {
    for revoke in [false, true] {
        let f = fixture(TestProfile::Default);
        let account = f
            .client
            .create_account(f.host(), &TestAccountSpec::new("rtfallback", 0x35))
            .unwrap();
        let mut connection = f
            .client
            .foks()
            .realtime_connection(f.host(), &account.credential)
            .unwrap();
        connection.call(&delta()).unwrap();
        let before = f.server.metrics().realtime.poll.submissions;
        let waiting = std::thread::spawn(move || connection.call(&poll(0, 5000)));
        wait(|| f.server.metrics().active_realtime_polls == 1);
        let path = f.environment.database_path().to_path_buf();
        let uid = account.credential.uid.as_bytes().to_vec();
        f.server
            .writer_handle()
            .call(move |_| {
                // Deliberately bypass notifications while serializing with the owning writer.
                let c = rusqlite::Connection::open(path).unwrap();
                c.execute(
                    if revoke {
                        "UPDATE devices SET active=0 WHERE uid=?1"
                    } else {
                        "UPDATE rt_user_inboxes SET version=version+1 WHERE uid=?1"
                    },
                    [uid],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        let result = waiting.join().unwrap();
        if revoke {
            assert!(result.is_err());
        } else {
            let Response::PollResult(result) = result.unwrap() else {
                panic!()
            };
            assert!(result.bumped);
            assert_eq!(result.inbox_version, 1);
        }
        assert_eq!(f.server.metrics().active_realtime_polls, 0);
        assert_eq!(f.server.metrics().realtime.poll.submissions, before);
    }
}

#[test]
fn stale_concurrent_delta_readers_recheck_clean_state_at_writer_execution() {
    let f = fixture(TestProfile::Default);
    let account = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("rtrace", 0x36))
        .unwrap();
    let connections = (0..2)
        .map(|_| {
            f.client
                .foks()
                .realtime_connection(f.host(), &account.credential)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let pressure = f.server.saturate_writer_queue().unwrap();
    let waiting = connections
        .into_iter()
        .map(|mut c| std::thread::spawn(move || c.call(&delta())))
        .collect::<Vec<_>>();
    wait(|| f.server.writer_handle().metrics().pending == 4);
    pressure.drain().unwrap();
    for thread in waiting {
        thread.join().unwrap().unwrap();
    }
    let s = f.server.metrics().realtime.delta;
    assert_eq!(s.submissions, 2);
    assert_eq!(s.complete, 1);
    assert_eq!(s.already_clean, 1);
    assert_eq!(s.execution_buckets[8], 2);
}

#[test]
fn parked_poll_rechecks_presented_certificate_expiry_without_writer_work() {
    let f = fixture(TestProfile::Default);
    let account = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("rtexpire", 0x38))
        .unwrap();
    let mut connection = f
        .client
        .foks()
        .realtime_connection(f.host(), &account.credential)
        .unwrap();
    connection.call(&delta()).unwrap();
    let before = f.server.metrics().realtime;
    let waiting = std::thread::spawn(move || connection.call(&poll(0, 5000)));
    wait(|| f.server.metrics().active_realtime_polls == 1);
    f.environment.advance_clock(366 * 24 * 60 * 60 * 1_000_000);
    assert!(waiting.join().unwrap().is_err());
    let after = f.server.metrics().realtime;
    assert_eq!(after.poll.submissions, before.poll.submissions);
    assert_eq!(after.failed_polls - before.failed_polls, 1);
    assert_eq!(f.server.metrics().active_realtime_polls, 0);
}

#[test]
fn timed_out_client_disconnect_releases_poll_capacity() {
    let f = fixture(TestProfile::RealtimePollCapacity);
    let account = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("rtcancel", 0x39))
        .unwrap();
    let mut short_client = f.client.foks().clone();
    short_client.set_timeout(Duration::from_millis(500));
    let mut connection = short_client
        .realtime_connection(f.host(), &account.credential)
        .unwrap();
    connection.call(&delta()).unwrap();
    let before = f.server.metrics().realtime.cancelled_polls;
    let waiting = std::thread::spawn(move || connection.call(&poll(0, 5000)));
    wait(|| f.server.metrics().active_realtime_polls == 1);
    assert!(waiting.join().unwrap().is_err());
    wait(|| f.server.metrics().active_realtime_polls == 0);
    assert_eq!(f.server.metrics().realtime.cancelled_polls - before, 1);
    let mut another = f
        .client
        .foks()
        .realtime_connection(f.host(), &account.credential)
        .unwrap();
    another.call(&poll(0, 10)).unwrap();
}

#[test]
fn queued_revocation_is_checked_and_timed_before_reconciliation_body() {
    let f = fixture(TestProfile::Default);
    let account = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("rtqueued", 0x40))
        .unwrap();
    let mut connection = f
        .client
        .foks()
        .realtime_connection(f.host(), &account.credential)
        .unwrap();
    let pressure = f.server.saturate_writer_queue().unwrap();
    let path = f.environment.database_path().to_path_buf();
    let uid = account.credential.uid.as_bytes().to_vec();
    let writer = f.server.writer_handle();
    let revocation = std::thread::spawn(move || {
        writer.call(move |_| {
            rusqlite::Connection::open(path)
                .unwrap()
                .execute("UPDATE devices SET active=0 WHERE uid=?1", [uid])
                .unwrap();
            Ok(())
        })
    });
    wait(|| f.server.writer_handle().metrics().pending == 3);
    let waiting = std::thread::spawn(move || connection.call(&delta()));
    wait(|| f.server.writer_handle().metrics().pending == 4);
    pressure.drain().unwrap();
    revocation.join().unwrap().unwrap();
    assert!(waiting.join().unwrap().is_err());
    let s = f.server.metrics().realtime.delta;
    assert_eq!(s.execution_failures, 1);
    assert_eq!(s.execution_buckets[8], 1);
    assert_eq!(s.complete + s.incomplete + s.already_clean, 0);
}
