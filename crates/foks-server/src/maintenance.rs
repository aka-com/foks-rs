use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{Error, Result, WriterHandle};

const INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const ABANDONED_UPLOAD_AGE_MICROS: u64 = 24 * 60 * 60 * 1_000_000;

pub(crate) struct Maintenance {
    stop: SyncSender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Maintenance {
    pub(crate) fn start(writer: WriterHandle, clock: Arc<dyn foks_server_db::Clock>) -> Self {
        let (stop, receiver) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || loop {
            match receiver.recv_timeout(INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    let _ = writer.call_with_current_time(Arc::clone(&clock), |database, now| {
                        let cutoff = now.saturating_sub(ABANDONED_UPLOAD_AGE_MICROS);
                        database.run_maintenance(now, cutoff)?;
                        database.checkpoint()?;
                        Ok(())
                    });
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn shutdown(mut self) -> Result<()> {
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| Error::Thread)?;
        }
        Ok(())
    }
}

impl Drop for Maintenance {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
