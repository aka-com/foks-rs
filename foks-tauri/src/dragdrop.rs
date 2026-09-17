//! The file-drop path, which is native rather than web-native.
//!
//! `dragDropEnabled: true` in `tauri.conf.json` makes the *runtime* intercept
//! the OS drag and hand Rust the dropped **paths**; in exchange HTML5 drag and
//! drop is suppressed inside the window, so the drop zone's hover state has to
//! come from here too. That fork is deliberate: with it `false` the webview
//! would receive a pathless `File`, and the only way to upload would be to read
//! the bytes into the renderer — which the secret-handling policy forbids.
//!
//! Only paths cross this boundary. Phase 3 opens each file in Rust and streams
//! it through `AgentClient::put_kv_stream`; the renderer never sees the bytes.

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
/// Emission is scoped to that window rather than broadcast: `emit_to` names the
/// label, so a future second window cannot start receiving drops by existing.
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
                // The renderer can ask to import only one of these exact
                // native paths, once. This prevents an injected script from
                // turning the path-taking command into an arbitrary local
                // file reader. Non-UTF-8 paths cannot cross the JSON boundary
                // losslessly and are therefore omitted rather than mangled.
                let state = handle.state::<crate::commands::AppState>();
                let paths = state.record_drop_paths(paths);
                // The hover state is cleared first so a failed upload cannot
                // leave the drop zone lit.
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
            // wry's event is non-exhaustive; an unknown drag phase is not an
            // error and must not be reported as one.
            _ => Ok(()),
        };
        if let Err(error) = emitted {
            tracing::warn!(%error, "a drag-drop event could not reach the FOKS window");
        }
    });
}
