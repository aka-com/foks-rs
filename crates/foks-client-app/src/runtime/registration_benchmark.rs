//! The same fixture runs on the parent revision: no new batch/observer APIs.
use super::registration_fixture::*;
use super::*;
use foks_keystore::EncryptedFileSecretStore;
use std::time::Instant;

#[test]
#[ignore = "release-mode encrypted-vault registration benchmark; see docs/default-refresh-job-registration-plan.md"]
fn default_refresh_registration_benchmark() {
    for users in [1u8, 10, 50] {
        for due in [0u8, 1, 10, 16] {
            for present in [false, true] {
                let f = Fixture::new(true);
                let checked = crate::CheckedProfileSession {
                    session: &f.session,
                };
                let store = EncryptedFileSecretStore::open(
                    f.dir.path().join("bench-vault"),
                    zeroize::Zeroizing::new([0x42; 32]),
                )
                .unwrap();
                let mut secrets = CountingStore::new(store);
                for id in 1..=users {
                    for alias in [format!("user{id}"), format!("duplicate{id}")] {
                        AccountVault::new(&mut secrets)
                            .commit_created(&alias, "synthetic", &credential(id))
                            .unwrap();
                    }
                }
                if present {
                    checked
                        .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
                        .unwrap();
                }
                let scheduler = FoksScheduler::new(
                    &f.session.paths.hard_database,
                    SchedulerConfig {
                        claim_limit: 1,
                        jitter_percent: 0,
                        ..Default::default()
                    },
                )
                .unwrap();
                for id in 0..due {
                    scheduler
                        .register(ScheduledJobRegistration {
                            job_id: [0xa0 + id; 16],
                            kind: ScheduledJobKind::MutationReconcile,
                            host_id: f.host.as_bytes().to_vec(),
                            scope_id: vec![id; 16],
                            interval_micros: 10000,
                            first_run_at: 100,
                            registered_at: 1,
                        })
                        .unwrap();
                }
                let host_start = Instant::now();
                checked.pinned_host().unwrap();
                let host_us = host_start.elapsed().as_micros();
                secrets.reset();
                let mut slices = 0;
                let mut repair_us = 0;
                for _ in 0..16 {
                    let start = Instant::now();
                    checked
                        .ensure_default_refresh_jobs(&mut AccountVault::new(&mut secrets), 100)
                        .unwrap();
                    repair_us += start.elapsed().as_micros();
                    slices += 1;
                    // Same fixed cutoff and single claim as the resident slices;
                    // omit network handlers and exclude claims from repair timing.
                    if scheduler.run_due(100, |_| Ok(())).unwrap().runs.is_empty() {
                        break;
                    }
                }
                println!("DEFAULT_REPAIR users={users} aliases={} due={due} present={present} slices={slices} scans={} reads={} account_reads={} expected_jobs={} repair_us={repair_us} host_sample_us={host_us}",users as usize*2,secrets.scans,secrets.reads,secrets.account_reads,users as usize*2);
            }
        }
    }
}
