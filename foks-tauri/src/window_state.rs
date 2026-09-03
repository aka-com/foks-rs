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
        tracing::warn!(%error, "could not report the FOKS window state");
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
        }
    });
}
