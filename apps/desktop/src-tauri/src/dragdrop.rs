//! Native file-drop handling.
//!
//! When `dragDropEnabled` is active, the runtime intercepts OS drag events and passes
//! file paths directly to Rust. This allows file uploads via `AgentClient::put_kv_stream`
//! without exposing file contents to the webview renderer.

use serde::Serialize;
use tauri::{DragDropEvent, Emitter as _, Manager as _, WindowEvent};

/// Carries the dropped paths to the webview.
pub const EVT_DROP_PATHS: &str = "foks://drop-paths";
/// Carries whether a drag is currently over the window.
pub const EVT_DROP_HOVER: &str = "foks://drop-hover";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DropHover {
    hovering: bool,
}

/// Registers the drag-drop forwarder on the app's own window.
///
/// Emission is scoped to the target window using `emit_to`.
pub fn observe(window: &tauri::WebviewWindow) {
    let handle = window.app_handle().clone();
    let label = window.label().to_owned();
    window.on_window_event(move |event| {
        let WindowEvent::DragDrop(event) = event else {
            return;
        };
        let emitted = match event {
            DragDropEvent::Enter { .. } | DragDropEvent::Over { .. } => {
                handle.emit_to(label.as_str(), EVT_DROP_HOVER, DropHover { hovering: true })
            }
            DragDropEvent::Leave => handle.emit_to(
                label.as_str(),
                EVT_DROP_HOVER,
                DropHover { hovering: false },
            ),
            DragDropEvent::Drop { paths, .. } => {
                // Restrict imports to explicitly dropped native paths consumed once.
                // Non-UTF-8 paths are omitted to preserve lossless path encoding.
                let state = handle.state::<crate::commands::AppState>();
                let paths = state.record_drop_paths(paths);
                // Clear hover state before emitting drop paths to ensure the UI
                // drop zone resets even if processing fails.
                let cleared = handle.emit_to(
                    label.as_str(),
                    EVT_DROP_HOVER,
                    DropHover { hovering: false },
                );
                let dropped = handle.emit_to(label.as_str(), EVT_DROP_PATHS, paths);
                if dropped.is_err() {
                    state.clear_drop_paths();
                }
                cleared.and(dropped)
            }
            // Ignore unrecognized drag events from wry without logging an error.
            _ => Ok(()),
        };
        if let Err(error) = emitted {
            tracing::warn!(%error, "Failed to emit drag-drop event to window");
        }
    });
}
