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

//! The step that hands the address to another process (the portal call, or
//! starting the program) runs inside a caller-supplied [`HandOff`], so a caller
//! can serialize it with application locking. Connecting to the session bus
//! and waiting for a started program run outside it.

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
    use std::time::Duration;

    use zbus::blocking::connection::Builder;
    use zbus::zvariant::Value;

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
        /// The call was sent but no reply arrived within [`REPLY_TIMEOUT`].
        Unanswered,
    }

    /// Calls `OpenURI` through `hand_off`. The outer error is the hand-off's
    /// refusal; the inner one is the portal's outcome.
    pub(super) fn open_uri<E>(
        target: &str,
        hand_off: &super::HandOff<'_, E>,
    ) -> Result<Result<(), Failure>, super::Failure<E>> {
        let connection = match Builder::session()
            .and_then(|builder| builder.method_timeout(REPLY_TIMEOUT).build())
        {
            Ok(connection) => connection,
            Err(error) => return Ok(Err(Failure::Unavailable(error.to_string()))),
        };
        // An empty parent window identifier: the host process has no
        // exported window handle to pass.
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        super::gated(hand_off, || {
            connection
                .call_method(
                    Some(DESTINATION),
                    PATH,
                    Some(INTERFACE),
                    "OpenURI",
                    &("", target, options),
                )
                .map(|_request_handle| ())
                .map_err(classify)
        })
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
