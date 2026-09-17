//! Core-side clipboard writes with digest-only tracking and timed clearing.
//!
//! macOS receives the concealed pasteboard type. Linux offers no equivalent
//! hint, so its timeout is intentionally shorter. No copied vault value is
//! retained after the platform write and no value crosses back to JavaScript.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use tauri::AppHandle;
#[cfg(not(target_os = "macos"))]
use tauri_plugin_clipboard_manager::ClipboardExt as _;
use zeroize::Zeroizing;

#[cfg(target_os = "linux")]
pub const AUTO_CLEAR_SECONDS: u64 = 15;
#[cfg(not(target_os = "linux"))]
pub const AUTO_CLEAR_SECONDS: u64 = 30;

type ClipboardDigest = [u8; 32];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingClipboard {
    digest: ClipboardDigest,
    generation: u64,
}

static PENDING: Mutex<Option<PendingClipboard>> = Mutex::new(None);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static EXIT_STARTED: AtomicBool = AtomicBool::new(false);
static EXIT_FINISHED: AtomicBool = AtomicBool::new(false);

fn digest(value: &str) -> ClipboardDigest {
    Sha256::digest(value.as_bytes()).into()
}

pub fn copy_with_hygiene(app: &AppHandle, value: Zeroizing<String>) -> Result<(), String> {
    let expected = PendingClipboard {
        digest: digest(&value),
        generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
    };
    let mut pending = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    #[cfg(target_os = "macos")]
    macos::write_concealed(&value)?;
    #[cfg(not(target_os = "macos"))]
    app.clipboard()
        .write_text(value.as_str())
        .map_err(|error| format!("could not write the system clipboard: {error}"))?;
    *pending = Some(expected);
    drop(pending);

    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(AUTO_CLEAR_SECONDS));
        if let Err(error) = clear_if_unchanged(&app, expected) {
            tracing::warn!(%error, "could not auto-clear copied FOKS value");
        }
    });
    Ok(())
}

fn clear_if_unchanged(app: &AppHandle, expected: PendingClipboard) -> Result<(), String> {
    let mut pending = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pending.as_ref() != Some(&expected) {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    let _ = app;
    #[cfg(target_os = "macos")]
    let current = macos::read_plain_text();
    #[cfg(not(target_os = "macos"))]
    let current = app
        .clipboard()
        .read_text()
        .map(Some)
        .map_err(|error| format!("could not read the system clipboard: {error}"))?;
    if current.as_deref().map(digest) == Some(expected.digest) {
        #[cfg(target_os = "macos")]
        macos::clear();
        #[cfg(not(target_os = "macos"))]
        app.clipboard()
            .write_text("")
            .map_err(|error| format!("could not clear the system clipboard: {error}"))?;
    }
    *pending = None;
    Ok(())
}

fn clear_pending(app: &AppHandle) {
    let expected = *PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(expected) = expected {
        if let Err(error) = clear_if_unchanged(app, expected) {
            tracing::warn!(%error, "could not clear copied FOKS value on exit");
        }
    }
}

/// Keeps the runtime alive until the clipboard plugin can verify and clear
/// FOKS's value. `RunEvent::Exit` is too late: plugin exit hooks have already
/// run by then, and Linux clipboard ownership may already be gone.
pub fn defer_exit_cleanup(app: &AppHandle, code: Option<i32>, api: &tauri::ExitRequestApi) {
    let has_pending = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some();
    if !has_pending || EXIT_FINISHED.load(Ordering::Acquire) {
        return;
    }
    api.prevent_exit();
    if EXIT_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        clear_pending(&app);
        EXIT_FINISHED.store(true, Ordering::Release);
        app.exit(code.unwrap_or(0));
    });
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;

    const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
    const UTF8: &str = "public.utf8-plain-text";

    pub fn write_concealed(value: &str) -> Result<(), String> {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            let value = NSString::from_str(value);
            if !pasteboard.setString_forType(&value, &NSString::from_str(UTF8)) {
                return Err("failed to write the system pasteboard".to_owned());
            }
            let _ = pasteboard.setString_forType(&value, &NSString::from_str(CONCEALED_TYPE));
            Ok(())
        })
    }

    pub fn read_plain_text() -> Option<String> {
        autoreleasepool(|_| {
            NSPasteboard::generalPasteboard()
                .stringForType(&NSString::from_str(UTF8))
                .map(|value| value.to_string())
        })
    }

    pub fn clear() {
        autoreleasepool(|_| {
            NSPasteboard::generalPasteboard().clearContents();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_tracking_retains_only_a_digest() {
        let secret = "not retained";
        let fingerprint = digest(secret);
        assert_eq!(fingerprint.len(), 32);
        assert_ne!(fingerprint.as_slice(), secret.as_bytes());
    }

    #[test]
    fn repeated_copies_have_distinct_timer_tokens() {
        let first = PendingClipboard {
            digest: digest("same"),
            generation: 1,
        };
        let second = PendingClipboard {
            digest: digest("same"),
            generation: 2,
        };
        assert_ne!(first, second);
    }
}
