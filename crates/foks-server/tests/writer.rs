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
                    let token = [index as u8; 32];
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
