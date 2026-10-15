//! Hands an external address to the desktop's default handler.
//!
//! Callers validate the address; this module only selects the platform
//! mechanism. On Linux the address goes to the XDG desktop portal's `OpenURI`
//! method on the session bus, which replies as soon as it has accepted the
//! request. When no portal is reachable, `/usr/bin/xdg-open` is started and
//! reaped on its own thread instead of being waited on, so a handler that stays
//! in the foreground, such as a terminal browser, does not hold the calling
//! worker. On macOS `/usr/bin/open` passes the address to Launch Services and
//! exits once it has been handed off. Both programs are named by absolute path,
//! so a `PATH` entry cannot substitute another program.

/// Opens `target` with the desktop's default handler for its scheme.
#[cfg(target_os = "macos")]
pub fn open(target: &str) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| format!("Could not start /usr/bin/open: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("/usr/bin/open exited with {status}"))
    }
}

/// Opens `target` with the desktop's default handler for its scheme.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn open(target: &str) -> Result<(), String> {
    match portal::open_uri(target) {
        Ok(()) => Ok(()),
        // The portal may still act on a request it did not acknowledge in
        // time, so starting xdg-open here could open the address twice.
        Err(portal::Failure::Unanswered) => Err("The desktop portal did not answer.".to_owned()),
        Err(portal::Failure::Unavailable(reason)) => {
            tracing::debug!(%reason, "Desktop portal unavailable; starting xdg-open");
            spawn_detached(std::path::Path::new(XDG_OPEN), target)
        }
    }
}

#[cfg(not(unix))]
pub fn open(_target: &str) -> Result<(), String> {
    Err("Opening links is unavailable on this platform.".to_owned())
}

#[cfg(all(unix, not(target_os = "macos")))]
const XDG_OPEN: &str = "/usr/bin/xdg-open";

/// Starts `program` with one argument and returns once it is running. Its exit
/// status is not observed; a thread waits for it so it does not remain a
/// zombie process.
#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_detached(program: &std::path::Path, argument: &str) -> Result<(), String> {
    let mut child = std::process::Command::new(program)
        .arg(argument)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", program.display()))?;
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

    pub(super) enum Failure {
        /// No session bus, no portal, or the portal rejected the request.
        Unavailable(String),
        /// The call was sent but no reply arrived within [`REPLY_TIMEOUT`].
        Unanswered,
    }

    pub(super) fn open_uri(target: &str) -> Result<(), Failure> {
        let connection = Builder::session()
            .and_then(|builder| builder.method_timeout(REPLY_TIMEOUT).build())
            .map_err(|error| Failure::Unavailable(error.to_string()))?;
        // An empty parent window identifier: the host process has no
        // exported window handle to pass.
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "OpenURI",
                &("", target, options),
            )
            .map(|_request_handle| ())
            .map_err(|error| match error {
                zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => {
                    Failure::Unanswered
                }
                error => Failure::Unavailable(error.to_string()),
            })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn fallback_opener_returns_while_the_handler_is_still_running() {
        let started = std::time::Instant::now();
        super::spawn_detached(std::path::Path::new("/bin/sleep"), "5").unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "starting the handler must not wait for it to exit"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn fallback_opener_reports_a_program_that_cannot_start() {
        let error =
            super::spawn_detached(std::path::Path::new("/nonexistent/xdg-open"), "x").unwrap_err();
        assert!(error.contains("/nonexistent/xdg-open"), "{error}");
    }
}
