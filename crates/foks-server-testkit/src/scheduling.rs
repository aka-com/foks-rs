use std::sync::mpsc::{Receiver, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub struct WriterQueuePressure {
    release: Option<SyncSender<()>>,
    blocker: Option<JoinHandle<foks_server::Result<()>>>,
    queued: Option<JoinHandle<foks_server::Result<()>>>,
    handle: foks_server::WriterHandle,
}

impl WriterQueuePressure {
    pub(crate) fn start(handle: foks_server::WriterHandle) -> foks_server::Result<Self> {
        let (entered_sender, entered_receiver) = std::sync::mpsc::sync_channel(1);
        let (release_sender, release_receiver) = std::sync::mpsc::sync_channel(1);
        let blocker_handle = handle.clone();
        let blocker = std::thread::spawn(move || {
            blocker_handle.call(move |_| {
                entered_sender
                    .send(())
                    .map_err(|_| foks_server::Error::Thread)?;
                await_release(release_receiver)?;
                Ok(())
            })
        });
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| foks_server::Error::Thread)?;
        let queued_handle = handle.clone();
        let queued = std::thread::spawn(move || queued_handle.call(|_| Ok(())));
        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.metrics().pending != 2 {
            if Instant::now() >= deadline {
                let _ = release_sender.try_send(());
                let _ = blocker.join();
                let _ = queued.join();
                return Err(foks_server::Error::WriterQueue);
            }
            std::thread::yield_now();
        }
        Ok(Self {
            release: Some(release_sender),
            blocker: Some(blocker),
            queued: Some(queued),
            handle,
        })
    }

    pub fn metrics(&self) -> foks_server::WriterMetrics {
        self.handle.metrics()
    }

    pub fn drain(mut self) -> foks_server::Result<foks_server::WriterMetrics> {
        self.release_and_join()?;
        Ok(self.handle.metrics())
    }

    fn release_and_join(&mut self) -> foks_server::Result<()> {
        if let Some(release) = self.release.take() {
            release.send(()).map_err(|_| foks_server::Error::Thread)?;
        }
        for thread in [&mut self.blocker, &mut self.queued] {
            if let Some(thread) = thread.take() {
                thread.join().map_err(|_| foks_server::Error::Thread)??;
            }
        }
        Ok(())
    }
}

impl Drop for WriterQueuePressure {
    fn drop(&mut self) {
        let _ = self.release_and_join();
    }
}

fn await_release(receiver: Receiver<()>) -> foks_server::Result<()> {
    receiver.recv().map_err(|_| foks_server::Error::Thread)
}
