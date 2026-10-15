//! File paths authorized for upload by a drop or the native picker.

use super::{AppState, LockExt};
use crate::agent::AgentError;
use std::path::PathBuf;

impl AppState {
    pub(crate) fn record_drop_paths(&self, paths: &[PathBuf]) -> Vec<String> {
        let mut pending = self.pending_upload_paths.lock_unpoisoned();
        pending.clear();
        let wire_paths: Vec<String> = paths
            .iter()
            .filter_map(|path| path.to_str().map(ToOwned::to_owned))
            .collect();
        // Accept only a single UTF-8 file path for drops. Record multi-drop
        // paths for UI reporting without authorizing them.
        if paths.len() == 1 && wire_paths.len() == 1 {
            pending.insert(wire_paths[0].clone(), paths[0].clone());
        }
        wire_paths
    }

    pub(crate) fn clear_drop_paths(&self) {
        self.pending_upload_paths.lock_unpoisoned().clear();
    }

    pub(crate) fn record_picked_path(&self, path: PathBuf) -> Result<String, AgentError> {
        let wire_path = path.to_str().map(ToOwned::to_owned).ok_or_else(|| {
            AgentError::new(
                "upload-source",
                "The selected file path is not valid UTF-8.",
                false,
            )
        })?;
        let mut pending = self.pending_upload_paths.lock_unpoisoned();
        pending.clear();
        pending.insert(wire_path.clone(), path);
        Ok(wire_path)
    }

    pub(in crate::commands) fn upload_path(&self, wire_path: &str) -> Result<PathBuf, AgentError> {
        self.pending_upload_paths
            .lock_unpoisoned()
            .get(wire_path)
            .cloned()
            .ok_or_else(|| {
                AgentError::new(
                    "drop-not-authorized",
                    "Choose or drop the file again to import it.",
                    false,
                )
            })
    }

    pub(in crate::commands) fn release_upload_path(&self, wire_path: &str) {
        self.pending_upload_paths
            .lock_unpoisoned()
            .remove(wire_path);
    }
}
