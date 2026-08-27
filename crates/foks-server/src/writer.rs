use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use foks_server_db::Database;

use crate::{Error, Result};

trait Task: Send {
    fn run(self: Box<Self>, database: &mut Database);
}

struct Call<F, T> {
    operation: F,
    response: SyncSender<Result<T>>,
}

impl<F, T> Task for Call<F, T>
where
    F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    fn run(self: Box<Self>, database: &mut Database) {
        let Self {
            operation,
            response,
        } = *self;
        let _ = response.send(operation(database));
    }
}

enum Message {
    Task(Box<dyn Task>),
    Shutdown,
}

pub struct Writer {
    handle: WriterHandle,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct WriterHandle {
    queue: Arc<WriterQueue>,
}

struct WriterQueue {
    sender: SyncSender<Message>,
    accepting: Mutex<bool>,
}

impl Writer {
    pub fn start(
        database_path: PathBuf,
        database_config: foks_server_db::Config,
        maximum_pending: usize,
    ) -> Result<Self> {
        if maximum_pending == 0 {
            return Err(Error::Config("zero writer queue limit"));
        }
        let (sender, receiver) = mpsc::sync_channel(maximum_pending);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let database = Database::open(database_path, database_config);
            match database {
                Ok(mut database) => {
                    if startup_sender.send(Ok(())).is_ok() {
                        run(&mut database, &receiver);
                    }
                }
                Err(error) => {
                    let _ = startup_sender.send(Err(error));
                }
            }
        });
        match startup_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                handle: WriterHandle {
                    queue: Arc::new(WriterQueue {
                        sender,
                        accepting: Mutex::new(true),
                    }),
                },
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error.into())
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::Thread)
            }
        }
    }

    pub fn call<F, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.handle.call(operation)
    }

    pub fn handle(&self) -> WriterHandle {
        self.handle.clone()
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> Result<()> {
        let mut accepting = self
            .handle
            .queue
            .accepting
            .lock()
            .map_err(|_| Error::Thread)?;
        if *accepting {
            *accepting = false;
            let _ = self.handle.queue.sender.send(Message::Shutdown);
        }
        drop(accepting);
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| Error::Thread)?;
        }
        Ok(())
    }
}

impl WriterHandle {
    pub fn call<F, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let (response, receiver) = mpsc::sync_channel(1);
        let accepting = self
            .queue
            .accepting
            .lock()
            .map_err(|_| Error::WriterQueue)?;
        if !*accepting {
            return Err(Error::WriterQueue);
        }
        self.queue
            .sender
            .try_send(Message::Task(Box::new(Call {
                operation,
                response,
            })))
            .map_err(|_| Error::WriterQueue)?;
        drop(accepting);
        receiver.recv().map_err(|_| Error::WriterQueue)?
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run(database: &mut Database, receiver: &Receiver<Message>) {
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Task(task) => task.run(database),
            Message::Shutdown => break,
        }
    }
}
