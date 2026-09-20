//! Same fixture on either revision; isolates fanout lookup from database/RPC work.
use super::*;
use std::time::Instant;
#[test]
#[ignore = "release-mode fanout benchmark"]
fn fixed_recipient_fanout_benchmark() {
    for count in [10u64, 1_000, 10_000] {
        let service = RealtimeService::default();
        let targets = (0..count)
            .map(|id| RealtimeWakeTarget {
                host: vec![2],
                uid: id.to_le_bytes().to_vec(),
                app: foks_proto::RtAppId::Chat,
            })
            .collect::<Vec<_>>();
        let _listeners = targets
            .iter()
            .map(|target| service.inbox_hub.listener(target).unwrap())
            .collect::<Vec<_>>();
        for _ in 0..100 {
            service.notifier.notify(&targets[..2]);
        }
        let mut batches = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            for _ in 0..1_000 {
                service.notifier.notify(&targets[..2]);
            }
            batches.push(start.elapsed().as_nanos() / 1_000);
        }
        batches.sort();
        println!(
            "FANOUT listeners={count} recipients=2 median_ns={} max_batch_mean_ns={}",
            batches[2], batches[4]
        );
    }
}
