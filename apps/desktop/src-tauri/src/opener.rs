//! Hands an external address to the desktop's default handler.
//!
//! Callers validate the address; this module only selects the platform
//! mechanism. On Linux the address goes to the XDG desktop portal's `OpenURI`
//! method on the session bus, which replies as soon as it has accepted the
//! request. A refusal from a running portal is reported as a failure. When no
//! portal is reachable, `/usr/bin/xdg-open` is started; this fallback requires
//! that path, so a system that installs xdg-open elsewhere, such as NixOS or
//! Guix, opens links only through the portal. An exit within
//! [`EARLY_EXIT_WINDOW`] is checked for failure, and after that window the
//! process is reaped on its own thread instead of being waited on, so a handler
//! that stays in the foreground, such as a terminal browser, does not hold the
//! calling worker. On macOS `/usr/bin/open` passes the address to Launch
//! Services and exits once it has been handed off. Both programs are named by
//! absolute path, so a `PATH` entry cannot substitute another program.

//! The step that hands the address to another process (sending the portal's
//! `OpenURI` call, or starting the program) runs inside a caller-supplied
//! [`HandOff`], so a caller can serialize it with application locking.
//! Connecting to the session bus, waiting for the portal's reply and waiting
//! for a started program run outside it, so a slow portal or handler does not
//! hold the gate.

/// Runs one hand-off step, or refuses with `E` without running it.
pub type HandOff<'a, E> = dyn Fn(&mut dyn FnMut()) -> Result<(), E> + 'a;

/// Why [`open_gated`] did not open the address.
#[derive(Debug)]
pub enum Failure<E> {
    /// The [`HandOff`] refused the step.
    Refused(E),
    /// The platform mechanism failed.
    Failed(String),
}

/// Runs `step` through `hand_off` and returns what it produced.
#[cfg_attr(not(unix), allow(dead_code))]
fn gated<T, E>(hand_off: &HandOff<'_, E>, step: impl FnOnce() -> T) -> Result<T, Failure<E>> {
    let mut step = Some(step);
    let mut output = None;
    hand_off(&mut || output = step.take().map(|step| step())).map_err(Failure::Refused)?;
    output.ok_or_else(|| Failure::Failed("The hand-off step did not run.".to_owned()))
}

/// Opens `target` with the desktop's default handler for its scheme, without
/// a hand-off gate.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn open(target: &str) -> Result<(), String> {
    let ungated = |step: &mut dyn FnMut()| -> Result<(), std::convert::Infallible> {
        step();
        Ok(())
    };
    open_gated(target, &ungated).map_err(|failure| match failure {
        Failure::Refused(never) => match never {},
        Failure::Failed(reason) => reason,
    })
}

/// Opens `target` with the desktop's default handler for its scheme.
#[cfg(target_os = "macos")]
pub fn open_gated<E>(target: &str, hand_off: &HandOff<'_, E>) -> Result<(), Failure<E>> {
    let mut child = gated(hand_off, || {
        std::process::Command::new("/usr/bin/open")
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    })?
    .map_err(|error| Failure::Failed(format!("Could not start /usr/bin/open: {error}")))?;
    let status = child
        .wait()
        .map_err(|error| Failure::Failed(format!("Could not wait for /usr/bin/open: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::Failed(format!(
            "/usr/bin/open exited with {status}"
        )))
    }
}

/// Opens `target` with the desktop's default handler for its scheme.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn open_gated<E>(target: &str, hand_off: &HandOff<'_, E>) -> Result<(), Failure<E>> {
    match portal::open_uri(target, hand_off)? {
        Ok(()) => Ok(()),
        // The portal may still act on a request it did not acknowledge in
        // time, so starting xdg-open here could open the address twice.
        Err(portal::Failure::Unanswered) => Err(Failure::Failed(
            "The desktop portal did not answer.".to_owned(),
        )),
        Err(portal::Failure::Rejected(reason)) => Err(Failure::Failed(format!(
            "The desktop portal refused the request: {reason}"
        ))),
        Err(portal::Failure::Unavailable(reason)) => {
            tracing::debug!(%reason, "Desktop portal unavailable; starting xdg-open");
            spawn_detached(std::path::Path::new(XDG_OPEN), target, hand_off)
        }
    }
}

#[cfg(not(unix))]
pub fn open_gated<E>(_target: &str, _hand_off: &HandOff<'_, E>) -> Result<(), Failure<E>> {
    Err(Failure::Failed(
        "Opening links is unavailable on this platform.".to_owned(),
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
const XDG_OPEN: &str = "/usr/bin/xdg-open";

/// How long [`spawn_detached`] watches for an exit status. xdg-open exits
/// within this window when no handler is registered for the scheme or the
/// handler hands the address to a running browser.
#[cfg(all(unix, not(target_os = "macos")))]
const EARLY_EXIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(1);

/// Starts `program` with one argument. A non-zero exit within
/// [`EARLY_EXIT_WINDOW`] is reported as an error; a program still running at
/// the end of the window is treated as started, and a thread waits for it so it
/// does not remain a zombie process. Only starting the program runs inside
/// `hand_off`.
#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_detached<E>(
    program: &std::path::Path,
    argument: &str,
    hand_off: &HandOff<'_, E>,
) -> Result<(), Failure<E>> {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);
    let mut child = gated(hand_off, || {
        std::process::Command::new(program)
            .arg(argument)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    })?
    .map_err(|error| Failure::Failed(format!("Could not start {}: {error}", program.display())))?;
    let deadline = std::time::Instant::now() + EARLY_EXIT_WINDOW;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(Failure::Failed(format!(
                    "{} exited with {status}",
                    program.display()
                )))
            }
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            // Still running at the deadline, or the status could not be read.
            Ok(None) | Err(_) => break,
        }
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use std::collections::HashMap;
    use std::num::NonZeroU32;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::{Duration, Instant};

    use zbus::blocking::connection::Builder;
    use zbus::export::futures_core::Stream;
    use zbus::message::Type;
    use zbus::zvariant::Value;
    use zbus::Message;

    const DESTINATION: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const INTERFACE: &str = "org.freedesktop.portal.OpenURI";
    /// The portal replies with a request handle before it opens anything, so
    /// a reply that takes this long means the portal is not serving requests.
    const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

    /// D-Bus errors that mean no portal is serving `OpenURI`, as opposed to
    /// a running portal refusing the request.
    const NO_PORTAL_ERRORS: [&str; 5] = [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
        "org.freedesktop.DBus.Error.UnknownMethod",
        "org.freedesktop.DBus.Error.UnknownInterface",
        "org.freedesktop.DBus.Error.UnknownObject",
    ];

    #[derive(Debug)]
    pub(super) enum Failure {
        /// No session bus, or no portal serving `OpenURI` on it.
        Unavailable(String),
        /// The portal answered the call with an error.
        Rejected(String),
        /// The call was sent but no reply arrived within [`REPLY_TIMEOUT`], or
        /// the connection closed before one did.
        Unanswered,
    }

    /// Sends `OpenURI` through `hand_off` and waits for the portal's reply
    /// outside it. The outer error is the hand-off's refusal; the inner one is
    /// the portal's outcome.
    pub(super) fn open_uri<E>(
        target: &str,
        hand_off: &super::HandOff<'_, E>,
    ) -> Result<Result<(), Failure>, super::Failure<E>> {
        // The timeout bounds the bus `Hello` call made while connecting.
        let connection = match Builder::session()
            .and_then(|builder| builder.method_timeout(REPLY_TIMEOUT).build())
        {
            Ok(connection) => connection,
            Err(error) => return Ok(Err(Failure::Unavailable(error.to_string()))),
        };
        // Subscribed before the call is sent, so the reply cannot be missed.
        let messages = zbus::MessageStream::from(connection.inner());
        // An empty parent window identifier: the host process has no
        // exported window handle to pass.
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        let call = match Message::method_call(PATH, "OpenURI")
            .and_then(|builder| builder.destination(DESTINATION))
            .and_then(|builder| builder.interface(INTERFACE))
            .and_then(|builder| builder.build(&("", target, options)))
        {
            Ok(call) => call,
            Err(error) => return Ok(Err(classify(error))),
        };
        let serial = call.primary_header().serial_num();
        exchange(
            hand_off,
            || connection.send(&call),
            || await_reply(messages, serial, REPLY_TIMEOUT),
        )
    }

    /// Runs `send` through `hand_off`, then `wait` after the hand-off has
    /// returned. `wait` does not run when sending failed or was refused.
    pub(super) fn exchange<E>(
        hand_off: &super::HandOff<'_, E>,
        send: impl FnOnce() -> zbus::Result<()>,
        wait: impl FnOnce() -> Result<(), Failure>,
    ) -> Result<Result<(), Failure>, super::Failure<E>> {
        if let Err(error) = super::gated(hand_off, send)? {
            return Ok(Err(classify(error)));
        }
        Ok(wait())
    }

    /// Waits up to `timeout` for the reply to the method call numbered
    /// `serial`, skipping other messages.
    pub(super) fn await_reply<S>(
        mut messages: S,
        serial: NonZeroU32,
        timeout: Duration,
    ) -> Result<(), Failure>
    where
        S: Stream<Item = zbus::Result<Message>> + Unpin,
    {
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let deadline = Instant::now() + timeout;
        loop {
            match Pin::new(&mut messages).poll_next(&mut context) {
                Poll::Ready(Some(Ok(message))) => {
                    if message.header().reply_serial() != Some(serial) {
                        continue;
                    }
                    return match message.message_type() {
                        Type::MethodReturn => Ok(()),
                        _ => Err(classify(zbus::Error::from(message))),
                    };
                }
                Poll::Ready(Some(Err(error))) => return Err(classify(error)),
                // The connection closed after the call was sent, so the portal
                // may still act on it.
                Poll::Ready(None) => return Err(Failure::Unanswered),
                Poll::Pending => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(Failure::Unanswered);
                    }
                    std::thread::park_timeout(remaining);
                }
            }
        }
    }

    /// Wakes the thread blocked in [`await_reply`].
    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    /// Maps a failed `OpenURI` call to the action the caller takes: only
    /// [`Failure::Unavailable`] falls back to xdg-open.
    pub(super) fn classify(error: zbus::Error) -> Failure {
        match error {
            zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => {
                Failure::Unanswered
            }
            zbus::Error::MethodError(name, _, _) if NO_PORTAL_ERRORS.contains(&name.as_str()) => {
                Failure::Unavailable(name.to_string())
            }
            error @ zbus::Error::MethodError(..) => Failure::Rejected(error.to_string()),
            error => Failure::Unavailable(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(unix, not(target_os = "macos")))]
    fn ungated(step: &mut dyn FnMut()) -> Result<(), ()> {
        step();
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn failure(result: Result<(), super::Failure<()>>) -> String {
        match result {
            Err(super::Failure::Failed(reason)) => reason,
            other => panic!("expected a platform failure, got {other:?}"),
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn refused_hand_off_does_not_start_the_program() {
        let refuse = |_step: &mut dyn FnMut()| Err("locked");
        let marker = std::env::temp_dir().join(format!("foks-opener-{}", std::process::id()));
        let result = super::spawn_detached(
            std::path::Path::new("/usr/bin/touch"),
            marker.to_str().unwrap(),
            &refuse,
        );
        assert!(matches!(result, Err(super::Failure::Refused("locked"))));
        assert!(!marker.exists());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn fallback_opener_returns_while_the_handler_is_still_running() {
        let started = std::time::Instant::now();
        super::spawn_detached(std::path::Path::new("/bin/sleep"), "30", &ungated).unwrap();
        assert!(
            started.elapsed() < super::EARLY_EXIT_WINDOW + std::time::Duration::from_secs(5),
            "starting the handler must not wait for it to exit"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn fallback_opener_reports_an_early_failed_exit() {
        let error = failure(super::spawn_detached(
            std::path::Path::new("/bin/false"),
            "x",
            &ungated,
        ));
        assert!(error.contains("/bin/false exited with"), "{error}");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn fallback_opener_accepts_an_early_successful_exit() {
        super::spawn_detached(std::path::Path::new("/bin/true"), "x", &ungated).unwrap();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn fallback_opener_reports_a_program_that_cannot_start() {
        let error = failure(super::spawn_detached(
            std::path::Path::new("/nonexistent/xdg-open"),
            "x",
            &ungated,
        ));
        assert!(error.contains("/nonexistent/xdg-open"), "{error}");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn method_error(name: &str) -> zbus::Error {
        let call = zbus::Message::method_call("/org/freedesktop/portal/desktop", "OpenURI")
            .unwrap()
            .build(&())
            .unwrap();
        let reply = zbus::Message::error(&call.header(), name)
            .unwrap()
            .build(&("detail",))
            .unwrap();
        zbus::Error::from(reply)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn open_uri_call() -> zbus::Message {
        zbus::Message::method_call("/org/freedesktop/portal/desktop", "OpenURI")
            .unwrap()
            .build(&())
            .unwrap()
    }

    /// Yields queued messages, then stays pending.
    #[cfg(all(unix, not(target_os = "macos")))]
    struct Queue(std::collections::VecDeque<zbus::Message>);

    #[cfg(all(unix, not(target_os = "macos")))]
    impl zbus::export::futures_core::Stream for Queue {
        type Item = zbus::Result<zbus::Message>;

        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            match self.0.pop_front() {
                Some(message) => std::task::Poll::Ready(Some(Ok(message))),
                None => std::task::Poll::Pending,
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn portal_reply_wait_runs_after_the_hand_off_returns() {
        let inside = std::cell::Cell::new(false);
        let sent = std::cell::Cell::new(false);
        let hand_off = |step: &mut dyn FnMut()| -> Result<(), ()> {
            inside.set(true);
            step();
            inside.set(false);
            Ok(())
        };
        let outcome = super::portal::exchange(
            &hand_off,
            || {
                assert!(inside.get(), "the call must be sent inside the hand-off");
                sent.set(true);
                Ok(())
            },
            || {
                assert!(sent.get());
                assert!(
                    !inside.get(),
                    "the reply wait must run outside the hand-off"
                );
                Ok(())
            },
        );
        assert!(matches!(outcome, Ok(Ok(()))), "{outcome:?}");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn refused_portal_hand_off_sends_nothing() {
        let refuse = |_step: &mut dyn FnMut()| Err("locked");
        let outcome = super::portal::exchange(
            &refuse,
            || panic!("a refused hand-off must not send the call"),
            || panic!("a refused hand-off must not wait for a reply"),
        );
        assert!(matches!(outcome, Err(super::Failure::Refused("locked"))));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn failed_portal_send_does_not_wait() {
        let outcome = super::portal::exchange(
            &ungated,
            || {
                Err(zbus::Error::from(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                )))
            },
            || panic!("a failed send must not wait for a reply"),
        );
        assert!(
            matches!(outcome, Ok(Err(super::portal::Failure::Unavailable(_)))),
            "{outcome:?}"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn portal_reply_is_matched_by_serial() {
        let call = open_uri_call();
        let other = open_uri_call();
        let unrelated = zbus::Message::error(&other.header(), "org.example.Error")
            .unwrap()
            .build(&("detail",))
            .unwrap();
        let reply = zbus::Message::method_return(&call.header())
            .unwrap()
            .build(&())
            .unwrap();
        let outcome = super::portal::await_reply(
            Queue([unrelated, reply].into()),
            call.primary_header().serial_num(),
            std::time::Duration::from_secs(5),
        );
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn portal_error_reply_is_a_refusal() {
        let call = open_uri_call();
        let reply = zbus::Message::error(&call.header(), "org.freedesktop.portal.Error.NotAllowed")
            .unwrap()
            .build(&("detail",))
            .unwrap();
        let outcome = super::portal::await_reply(
            Queue([reply].into()),
            call.primary_header().serial_num(),
            std::time::Duration::from_secs(5),
        );
        assert!(
            matches!(outcome, Err(super::portal::Failure::Rejected(_))),
            "{outcome:?}"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn missing_portal_reply_is_unanswered() {
        let call = open_uri_call();
        let outcome = super::portal::await_reply(
            Queue(Default::default()),
            call.primary_header().serial_num(),
            std::time::Duration::from_millis(50),
        );
        assert!(
            matches!(outcome, Err(super::portal::Failure::Unanswered)),
            "{outcome:?}"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn portal_timeout_does_not_fall_back() {
        let error = zbus::Error::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "no reply",
        ));
        assert!(matches!(
            super::portal::classify(error),
            super::portal::Failure::Unanswered
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn missing_bus_or_portal_falls_back() {
        for error in [
            zbus::Error::from(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "no bus",
            )),
            zbus::Error::Unsupported,
            method_error("org.freedesktop.DBus.Error.ServiceUnknown"),
            method_error("org.freedesktop.DBus.Error.UnknownMethod"),
        ] {
            let failure = super::portal::classify(error);
            assert!(
                matches!(failure, super::portal::Failure::Unavailable(_)),
                "{failure:?}"
            );
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn portal_refusal_does_not_fall_back() {
        let failure =
            super::portal::classify(method_error("org.freedesktop.portal.Error.NotAllowed"));
        assert!(
            matches!(failure, super::portal::Failure::Rejected(_)),
            "{failure:?}"
        );
    }
}
