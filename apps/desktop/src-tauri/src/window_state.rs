use serde::Serialize;
use tauri::{Emitter as _, WindowEvent};

pub const EVT_WINDOW_STATE: &str = "foks://window-state";

#[derive(Clone, Copy, Serialize)]
pub struct WindowState {
    maximized: bool,
    fullscreen: bool,
}

fn current(window: &tauri::WebviewWindow) -> Result<WindowState, String> {
    Ok(WindowState {
        maximized: window.is_maximized().map_err(|error| error.to_string())?,
        fullscreen: window.is_fullscreen().map_err(|error| error.to_string())?,
    })
}

#[tauri::command]
pub fn get_window_state(window: tauri::WebviewWindow) -> Result<WindowState, String> {
    current(&window)
}

fn emit(window: &tauri::WebviewWindow) {
    let Ok(state) = current(window) else {
        return;
    };
    if let Err(error) = window.emit(EVT_WINDOW_STATE, state) {
        tracing::warn!(%error, "Failed to emit window state event");
    }
}

pub fn observe(window: &tauri::WebviewWindow) {
    emit(window);
    let observed = window.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Focused(true)
        ) {
            emit(&observed);
            #[cfg(target_os = "macos")]
            if let Err(error) = apply_traffic_visibility(&observed) {
                tracing::warn!(%error, "Failed to update titlebar controls");
            }
        }
    });
}

// AppKit owns these controls; CSS only affects the webview's traffic strip.
#[cfg(target_os = "macos")]
static TRAFFIC_VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[cfg(target_os = "macos")]
fn apply_traffic_visibility(window: &tauri::WebviewWindow) -> Result<(), String> {
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            use objc2_app_kit::{NSWindow, NSWindowButton};
            let Ok(pointer) = target.ns_window() else {
                return;
            };
            // The Tauri window is retained by `target`, and AppKit is accessed only
            // on the main thread. ns_window returns this window's NSWindow pointer.
            let native = unsafe { &*pointer.cast::<NSWindow>() };
            let hidden = !TRAFFIC_VISIBLE.load(std::sync::atomic::Ordering::Relaxed);
            for kind in [
                NSWindowButton::CloseButton,
                NSWindowButton::MiniaturizeButton,
                NSWindowButton::ZoomButton,
            ] {
                if let Some(button) = native.standardWindowButton(kind) {
                    button.setHidden(hidden);
                }
            }
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_traffic_lights_visible(
    window: tauri::WebviewWindow,
    visible: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        TRAFFIC_VISIBLE.store(visible, std::sync::atomic::Ordering::Relaxed);
        apply_traffic_visibility(&window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, visible);
        Ok(())
    }
}
