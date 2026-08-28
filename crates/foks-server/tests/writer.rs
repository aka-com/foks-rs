use std::sync::{Arc, Barrier};

use foks_server::Writer;

#[test]
fn writer_serializes_calls_and_persists_before_returning() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("foks-server.sqlite");
    let writer =
        Arc::new(Writer::start(path.clone(), foks_server_db::Config::default(), 16).unwrap());
    let barrier = Arc::new(Barrier::new(9));
    let threads = (0..8)
        .map(|index| {
            let writer = Arc::clone(&writer);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                writer.call(move |database| {
                    let token = [index as u8; 17];
                    database.reserve_name(&[index as u8 + 1], &token, 1, 1, 100)?;
                    Ok(())
                })
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for thread in threads {
        thread.join().unwrap().unwrap();
    }
    let writer = Arc::into_inner(writer).unwrap();
    writer.shutdown().unwrap();

    let database =
        foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
    assert!(database.integrity_check().unwrap());
}

#[test]
fn writer_rejects_an_unbounded_configuration() {
    let temporary = tempfile::tempdir().unwrap();
    assert!(Writer::start(
        temporary.path().join("foks-server.sqlite"),
        foks_server_db::Config::default(),
        0,
    )
    .is_err());
}

#[test]
fn writer_handles_fail_promptly_after_shutdown() {
    let temporary = tempfile::tempdir().unwrap();
    let writer = Writer::start(
        temporary.path().join("foks-server.sqlite"),
        foks_server_db::Config::default(),
        1,
    )
    .unwrap();
    let handle = writer.handle();
    writer.shutdown().unwrap();
    assert!(handle.call(|_| Ok(())).is_err());
}

#[test]
fn only_one_writer_process_can_own_a_database() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("foks-server.sqlite");
    let first = Writer::start(path.clone(), foks_server_db::Config::default(), 1).unwrap();
    assert!(Writer::start(path.clone(), foks_server_db::Config::default(), 1).is_err());
    first.shutdown().unwrap();
    Writer::start(path, foks_server_db::Config::default(), 1)
        .unwrap()
        .shutdown()
        .unwrap();
}

#[test]
fn bounded_writer_queue_rejects_promptly_and_recovers_after_drain() {
    let temporary = tempfile::tempdir().unwrap();
    let writer = Arc::new(
        Writer::start(
            temporary.path().join("foks-server.sqlite"),
            foks_server_db::Config::default(),
            1,
        )
        .unwrap(),
    );
    let (entered_sender, entered_receiver) = std::sync::mpsc::sync_channel(1);
    let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);
    let blocker_writer = Arc::clone(&writer);
    let blocker = std::thread::spawn(move || {
        blocker_writer.call(move |_| {
            entered_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
            Ok(())
        })
    });
    entered_receiver.recv().unwrap();

    let queued_writer = Arc::clone(&writer);
    let queued = std::thread::spawn(move || queued_writer.call(|_| Ok(())));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while writer.handle().metrics().pending != 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "writer queue did not fill"
        );
        std::thread::yield_now();
    }
    let before = std::time::Instant::now();
    assert!(matches!(
        writer.call(|_| Ok(())),
        Err(foks_server::Error::WriterQueue)
    ));
    assert!(before.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(writer.handle().metrics().rejected, 1);

    release_sender.send(()).unwrap();
    blocker.join().unwrap().unwrap();
    queued.join().unwrap().unwrap();
    let metrics = writer.handle().metrics();
    assert_eq!(metrics.pending, 0);
    assert_eq!(metrics.queue_wait_observations, 2);
    assert_eq!(metrics.execution_observations, 2);
    assert!(metrics.queue_wait_microseconds_max > 0);
    assert!(metrics.execution_microseconds_max > 0);
    writer.call(|_| Ok(())).unwrap();
    Arc::into_inner(writer).unwrap().shutdown().unwrap();
}
