//! Nonblocking Unix transport with a single absolute request deadline.
use super::{check_upload_cancelled, Error, Result};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use std::io::{ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(super) fn wait(
    stream: Option<&UnixStream>,
    events: PollFlags,
    cancelled: &dyn Fn() -> bool,
    deadline: Instant,
) -> Result<()> {
    check_upload_cancelled(cancelled, deadline)?;
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .min(POLL_INTERVAL);
    let timeout = Timespec::try_from(remaining).map_err(|_| Error::DeadlineExceeded)?;
    let mut descriptors = stream.map(|stream| [PollFd::new(stream, events)]);
    match poll(
        descriptors.as_mut().map_or(&mut [], |fds| &mut fds[..]),
        Some(&timeout),
    ) {
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(Error::Io(error.into())),
    }
}

pub(super) fn connect(
    path: &Path,
    cancelled: &dyn Fn() -> bool,
    deadline: Instant,
) -> Result<UnixStream> {
    use rustix::net::{socket_with, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
    let address = SocketAddrUnix::new(path).map_err(|e| Error::Io(e.into()))?;
    #[cfg(target_os = "linux")]
    let flags = SocketFlags::CLOEXEC | SocketFlags::NONBLOCK;
    #[cfg(not(target_os = "linux"))]
    let flags = SocketFlags::empty();
    let fd = socket_with(AddressFamily::UNIX, SocketType::STREAM, flags, None)
        .map_err(|e| Error::Io(e.into()))?;
    rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC).map_err(|e| Error::Io(e.into()))?;
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(true)?;
    loop {
        match rustix::net::connect(&stream, &address) {
            Ok(()) => return Ok(stream),
            Err(rustix::io::Errno::INPROGRESS | rustix::io::Errno::ALREADY) => loop {
                check_upload_cancelled(cancelled, deadline)?;
                let mut ready = [PollFd::new(&stream, PollFlags::OUT)];
                let timeout = Timespec::try_from(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(POLL_INTERVAL),
                )
                .map_err(|_| Error::DeadlineExceeded)?;
                match poll(&mut ready, Some(&timeout)) {
                    Ok(0) | Err(rustix::io::Errno::INTR) => continue,
                    Ok(_) => {
                        rustix::net::sockopt::socket_error(&stream)
                            .map_err(|e| Error::Io(e.into()))?
                            .map_err(|e| Error::Io(e.into()))?;
                        return Ok(stream);
                    }
                    Err(error) => return Err(Error::Io(error.into())),
                }
            },
            // Linux reports EAGAIN for a full Unix listen backlog. There is no
            // connection in flight in that case; retry without spinning.
            Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => {
                wait(None, PollFlags::empty(), cancelled, deadline)?;
                check_upload_cancelled(cancelled, deadline)?;
            }
            Err(error) => return Err(Error::Io(error.into())),
        }
    }
}

pub(super) fn write_all(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    cancelled: &dyn Fn() -> bool,
    deadline: Instant,
) -> Result<()> {
    stream.set_nonblocking(true)?;
    while !bytes.is_empty() {
        check_upload_cancelled(cancelled, deadline)?;
        match stream.write(bytes) {
            Ok(0) => {
                return Err(Error::Io(std::io::Error::new(
                    ErrorKind::WriteZero,
                    "agent socket stopped accepting bytes",
                )))
            }
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                wait(Some(stream), PollFlags::OUT, cancelled, deadline)?;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => (),
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn nonreading_peer_cannot_hold_a_cancelled_write() {
        let (mut client, _server) = UnixStream::pair().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let error = write_all(
            &mut client,
            &vec![0; 8 * 1024 * 1024],
            &|| cancelled.load(Ordering::Acquire),
            started + Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(2));
        thread.join().unwrap();
    }

    #[test]
    fn slow_progress_does_not_restart_the_write_deadline() {
        use std::io::Read;
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        server.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let _ = server.read(&mut [0; 1024]);
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let started = Instant::now();
        let result = write_all(
            &mut client,
            &vec![0; 8 * 1024 * 1024],
            &|| false,
            started + Duration::from_millis(100),
        );
        stop.store(true, Ordering::Release);
        thread.join().unwrap();
        assert!(matches!(result, Err(Error::DeadlineExceeded)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn saturated_listen_backlog_observes_the_connect_deadline() {
        use std::os::unix::net::UnixListener;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        rustix::net::listen(&listener, 0).unwrap();
        let _occupant = UnixStream::connect(&path).unwrap();
        let started = Instant::now();
        assert!(matches!(
            connect(&path, &|| false, started + Duration::from_millis(100)),
            Err(Error::DeadlineExceeded)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
