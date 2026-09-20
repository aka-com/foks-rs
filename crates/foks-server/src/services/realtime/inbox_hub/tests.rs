use super::*;
use std::{
    future::Future,
    task::{Context, Poll, Wake, Waker},
};
fn target(id: u64) -> RealtimeWakeTarget {
    RealtimeWakeTarget {
        host: vec![2],
        uid: id.to_le_bytes().to_vec(),
        app: foks_proto::RtAppId::Chat,
    }
}
fn hub() -> Arc<InboxHub> {
    Arc::new(InboxHub::new(Arc::new(crate::ServerMetrics::default())))
}
struct CheckUnlocked(Arc<InboxHub>);
impl Wake for CheckUnlocked {
    fn wake(self: Arc<Self>) {
        assert!(self.0.waiters.try_lock().is_ok());
    }
}
#[test]
fn direct_delivery_deduplicates_and_wakes_all_account_waiters_outside_lock() {
    let hub = hub();
    let a = hub.listener(&target(1)).unwrap();
    assert!(Arc::ptr_eq(&a, &hub.listener(&target(1)).unwrap()));
    let b = hub.listener(&target(2)).unwrap();
    let waker = Waker::from(Arc::new(CheckUnlocked(hub.clone())));
    let mut cx = Context::from_waker(&waker);
    let mut first = std::pin::pin!(a.notified());
    let mut second = std::pin::pin!(a.notified());
    let mut unrelated = std::pin::pin!(b.notified());
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert!(second.as_mut().poll(&mut cx).is_pending());
    assert!(unrelated.as_mut().poll(&mut cx).is_pending());
    hub.notify(&[target(1), target(1), target(999)]);
    assert_eq!(first.as_mut().poll(&mut cx), Poll::Ready(()));
    assert_eq!(second.as_mut().poll(&mut cx), Poll::Ready(()));
    assert!(unrelated.as_mut().poll(&mut cx).is_pending());
    let s = hub.metrics.realtime.snapshot().notification;
    assert_eq!(
        (s.recipient_keys, s.entries_examined, s.listeners_woken),
        (2, 2, 1)
    );
    hub.notify_all();
    assert!(unrelated.as_mut().poll(&mut cx).is_ready());
    assert_eq!(
        hub.metrics.realtime.snapshot().membership.listeners_woken,
        2
    );
}
#[test]
fn replacement_generations_survive_stale_queue_entries_and_cleanup_is_bounded() {
    let hub = hub();
    let live = (0..100)
        .map(|i| hub.listener(&target(i)).unwrap())
        .collect::<Vec<_>>();
    // The oldest cleanup entry still names generation 1, after a targeted removal.
    drop(live);
    hub.notify(&[target(0)]);
    assert_eq!(
        hub.metrics
            .realtime
            .snapshot()
            .notification
            .expired_entries_removed,
        1
    );
    let replacement = hub.listener(&target(0)).unwrap();
    // Force an old queue entry to be visited after the replacement exists.
    hub.waiters
        .lock()
        .unwrap()
        .cleanup
        .push_front((WakeKey::from(&target(0)), 1));
    for _ in 0..100 {
        let before = hub.metrics.realtime.snapshot().registration;
        assert!(Arc::ptr_eq(
            &replacement,
            &hub.listener(&target(0)).unwrap()
        ));
        let after = hub.metrics.realtime.snapshot().registration;
        assert!(after.entries_examined - before.entries_examined <= CLEANUP_BUDGET as u64 + 1);
    }
    let waiters = hub.waiters.lock().unwrap();
    assert_eq!(waiters.by_key.len(), 1);
    assert_eq!(waiters.cleanup.len(), 1);
}
#[test]
fn concurrent_registrations_share_one_live_listener_during_notifications() {
    let hub = hub();
    let listener = hub.listener(&target(1)).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let hub = &hub;
            let barrier = &barrier;
            let listener = &listener;
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..100 {
                    assert!(Arc::ptr_eq(listener, &hub.listener(&target(1)).unwrap()));
                    hub.notify(&[target(1)]);
                }
            });
        }
        barrier.wait();
    });
    assert_eq!(hub.waiters.lock().unwrap().by_key.len(), 1);
    assert_eq!(hub.metrics.realtime.snapshot().notification.calls, 800);
}
#[test]
fn fixed_fanout_work_is_independent_of_unrelated_listener_count() {
    for count in [10, 1_000, 10_000] {
        let hub = hub();
        let _live = (0..count)
            .map(|i| hub.listener(&target(i)).unwrap())
            .collect::<Vec<_>>();
        hub.notify(&[target(0), target(1), target(1)]);
        let s = hub.metrics.realtime.snapshot().notification;
        assert_eq!(
            (s.entries_examined, s.recipient_keys, s.listeners_woken),
            (2, 2, 2)
        );
    }
}
