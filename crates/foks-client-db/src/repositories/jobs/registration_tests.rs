use super::*;
use crate::registration_test_support::observe;
use std::cell::RefCell;

thread_local! {
    static BETWEEN: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}
pub(super) fn between_phases() {
    let hook = BETWEEN.with(|s| s.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
fn fixture() -> (tempfile::TempDir, HardStateStore, ScheduledJob) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = HardStateStore::open(&dir.path().join("hard.sqlite")).unwrap();
    let verified = foks_verify::verify_public_host(
        "foks.app",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
        )),
    )
    .unwrap();
    store.accept_verified_host(&verified.snapshot).unwrap();
    let job = ScheduledJob {
        job_id: [1; 16],
        kind: ScheduledJobKind::UserRefresh,
        host_id: verified.snapshot.host_id().to_vec(),
        scope_id: vec![7; 33],
        interval_micros: 100,
        next_run_at: 10,
        failure_count: 0,
        lease_until: None,
        last_completed_at: None,
        last_error: None,
        updated_at: 1,
    };
    (dir, store, job)
}

#[test]
fn batch_is_atomic_and_existing_state_is_unchanged_without_writer_acquisition() {
    let (_dir, mut store, job) = fixture();
    let mut jobs = (1..=4)
        .map(|id| {
            let mut j = job.clone();
            j.job_id = [id; 16];
            j
        })
        .collect::<Vec<_>>();
    let (report, work) = observe(|| store.register_scheduled_jobs_if_missing(&jobs));
    assert_eq!(
        report.unwrap(),
        JobRegistrationReport {
            existing: 0,
            inserted: 4
        }
    );
    assert_eq!((work.read_transactions, work.write_transactions), (1, 1));
    let claimed = store.claim_due_scheduled_jobs(10, 50, 3).unwrap();
    store
        .fail_scheduled_job(&claimed[0].job_id, 50, 70, 20, "offline")
        .unwrap();
    store
        .complete_scheduled_job(&claimed[1].job_id, 50, 80, 20)
        .unwrap();
    // The third remains leased; the fourth has an explicit custom interval.
    let mut custom = jobs[3].clone();
    custom.interval_micros = 777;
    store.register_scheduled_job(&custom).unwrap();
    let rows = jobs
        .iter()
        .map(|j| store.scheduled_job(&j.job_id).unwrap().unwrap())
        .collect::<Vec<_>>();
    let metadata = store.metadata().unwrap();
    let changes = store.connection.total_changes();
    for j in &mut jobs {
        j.interval_micros = 999;
        j.next_run_at = 1000;
        j.updated_at = 900;
    }
    let (report, work) = observe(|| store.register_scheduled_jobs_if_missing(&jobs));
    assert_eq!(
        report.unwrap(),
        JobRegistrationReport {
            existing: 4,
            inserted: 0
        }
    );
    assert_eq!((work.read_transactions, work.write_transactions), (1, 0));
    assert_eq!(store.connection.total_changes(), changes);
    assert_eq!(store.metadata().unwrap(), metadata);
    for (j, row) in jobs.iter().zip(&rows) {
        assert_eq!(store.scheduled_job(&j.job_id).unwrap().as_ref(), Some(row));
    }
    store.remove_scheduled_job(&jobs[0].job_id).unwrap();
    let (report, work) = observe(|| store.register_scheduled_jobs_if_missing(&jobs));
    assert_eq!(
        report.unwrap(),
        JobRegistrationReport {
            existing: 3,
            inserted: 1
        }
    );
    assert_eq!(work.write_transactions, 1);
    assert_eq!(
        store.scheduled_job(&jobs[0].job_id).unwrap(),
        Some(jobs[0].clone())
    );
    for (j, row) in jobs[1..].iter().zip(&rows[1..]) {
        assert_eq!(store.scheduled_job(&j.job_id).unwrap().as_ref(), Some(row));
    }
}

#[test]
fn invalid_batches_fail_before_mutation_including_existing_integer_ranges() {
    let (_dir, mut store, job) = fixture();
    store.register_scheduled_job_if_missing(&job).unwrap();
    let mut missing = job.clone();
    missing.job_id = [2; 16];
    for case in 0..11 {
        let mut invalid = job.clone();
        match case {
            0 => invalid.kind = ScheduledJobKind::TeamRefresh,
            1 => invalid.scope_id = vec![8; 33],
            2 => invalid.host_id = vec![8; 33],
            3 => invalid.interval_micros = u64::MAX,
            4 => invalid.next_run_at = u64::MAX,
            5 => invalid.updated_at = u64::MAX,
            6 => invalid.failure_count = 1,
            7 => invalid.lease_until = Some(2),
            8 => invalid.last_completed_at = Some(1),
            9 => invalid.last_error = Some("offline".into()),
            _ => invalid.interval_micros = 0,
        }
        assert!(
            store
                .register_scheduled_jobs_if_missing(&[missing.clone(), invalid])
                .is_err(),
            "case {case}"
        );
        assert!(store.scheduled_job(&missing.job_id).unwrap().is_none());
    }
    assert!(store
        .register_scheduled_jobs_if_missing(&[job.clone(), job.clone()])
        .is_err());
    assert!(!store.register_scheduled_job_if_missing(&job).unwrap());
    let (report, work) = observe(|| store.register_scheduled_jobs_if_missing(&[]));
    assert_eq!(report.unwrap(), JobRegistrationReport::default());
    assert_eq!((work.read_transactions, work.write_transactions), (0, 0));
}

#[test]
fn read_fast_path_succeeds_while_another_connection_holds_the_writer() {
    let (dir, mut store, job) = fixture();
    store.register_scheduled_job_if_missing(&job).unwrap();
    let mut other = HardStateStore::open(&dir.path().join("hard.sqlite")).unwrap();
    store
        .connection
        .busy_timeout(std::time::Duration::ZERO)
        .unwrap();
    let _held = other.write_transaction().unwrap();
    let (report, work) =
        observe(|| store.register_scheduled_jobs_if_missing(std::slice::from_ref(&job)));
    assert_eq!(report.unwrap().existing, 1);
    assert_eq!(work.write_transactions, 0);
}

#[test]
fn interleaving_registration_conflict_and_deletion_are_rechecked_under_writer() {
    for case in 0..3 {
        let (dir, mut store, job) = fixture();
        let mut second = job.clone();
        second.job_id = [2; 16];
        store.register_scheduled_job_if_missing(&job).unwrap();
        let mut other = HardStateStore::open(&dir.path().join("hard.sqlite")).unwrap();
        let candidate = second.clone();
        let first = job.clone();
        BETWEEN.with(|s| {
            *s.borrow_mut() = Some(Box::new(move || {
                if case == 2 {
                    other.remove_scheduled_job(&first.job_id).unwrap();
                } else {
                    let mut custom = candidate;
                    custom.interval_micros = 777;
                    if case == 1 {
                        custom.scope_id = vec![9; 33];
                    }
                    other.register_scheduled_job(&custom).unwrap();
                }
            }))
        });
        let result = store.register_scheduled_jobs_if_missing(&[job.clone(), second.clone()]);
        if case == 1 {
            assert!(matches!(
                result,
                Err(Error::InvalidScheduledJob("job ID binding changed"))
            ));
        } else {
            let report = result.unwrap();
            assert_eq!(report.inserted, if case == 2 { 2 } else { 0 });
            assert_eq!(
                store
                    .scheduled_job(&second.job_id)
                    .unwrap()
                    .unwrap()
                    .interval_micros,
                if case == 2 { 100 } else { 777 }
            );
            assert_eq!(store.scheduled_job(&job.job_id).unwrap(), Some(job));
        }
    }
}

#[test]
fn insertion_failure_rolls_back_earlier_rows_and_revision() {
    let (_dir, mut store, job) = fixture();
    let mut second = job.clone();
    second.job_id = [2; 16];
    store.connection.execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON scheduled_jobs WHEN NEW.job_id=X'02020202020202020202020202020202' BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    let metadata = store.metadata().unwrap();
    assert!(store
        .register_scheduled_jobs_if_missing(&[job.clone(), second.clone()])
        .is_err());
    assert!(store.scheduled_job(&job.job_id).unwrap().is_none());
    assert!(store.scheduled_job(&second.job_id).unwrap().is_none());
    assert_eq!(store.metadata().unwrap(), metadata);
}
