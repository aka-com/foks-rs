//! Report-only enumeration. Filesystem pages use bounded selection memory;
//! scanning a nonempty directory is an offline operation, not activation work.
use super::*;
pub use foks_server_db::ChunkInventoryEntry;
use object_store::list::{PaginatedListOptions, PaginatedListStore};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkInventoryPage {
    pub entries: Vec<ChunkInventoryEntry>,
    pub next: Option<String>,
}
fn limit(value: usize) -> Result<()> {
    if !(1..=256).contains(&value) {
        Err(Error::Chunk("invalid inventory page limit"))
    } else {
        Ok(())
    }
}
impl FilesystemChunks {
    pub(super) fn empty(&self) -> Result<bool> {
        self.recheck()?;
        for entry in std::fs::read_dir(&self.directory)? {
            let entry = entry?;
            if entry.file_name() != NAMESPACE_MARKER && entry.file_name() != NAMESPACE_LOCK {
                return Ok(false);
            }
        }
        self.recheck()?;
        Ok(true)
    }
    pub(super) fn inventory(
        &self,
        cursor: Option<&str>,
        maximum: usize,
    ) -> Result<ChunkInventoryPage> {
        limit(maximum)?;
        self.recheck()?;
        let mut selected = std::collections::BTreeMap::new();
        let mut more = false;
        for entry in std::fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::Chunk("inventory key is not UTF-8"))?;
            if name == NAMESPACE_MARKER
                || name == NAMESPACE_LOCK
                || cursor.is_some_and(|c| name.as_str() <= c)
            {
                continue;
            }
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Error::Chunk("inventory encountered nonregular object"));
            }
            selected.insert(name, metadata.len());
            if selected.len() > maximum {
                selected.pop_last();
                more = true;
            }
        }
        self.recheck()?;
        let next = if more {
            selected.last_key_value().map(|(key, _)| key.clone())
        } else {
            None
        };
        Ok(ChunkInventoryPage {
            entries: selected
                .into_iter()
                .map(|(key, encoded_bytes)| ChunkInventoryEntry { key, encoded_bytes })
                .collect(),
            next,
        })
    }
}
impl S3Chunks {
    pub(super) fn inventory(
        &self,
        cursor: Option<&str>,
        maximum: usize,
    ) -> Result<ChunkInventoryPage> {
        limit(maximum)?;
        if cursor.is_some_and(|s| s.len() > 4096) {
            return Err(Error::Chunk("inventory token is too large"));
        }
        let store = self.store.clone();
        let prefix = self.prefix.to_string();
        let token = cursor.map(str::to_owned);
        self.run(async move {
            let prefix = if prefix.is_empty() {
                String::new()
            } else {
                format!("{prefix}/")
            };
            let page = store
                .list_paginated(
                    Some(&prefix),
                    PaginatedListOptions {
                        max_keys: Some(maximum),
                        page_token: token,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| Error::Chunk("object inventory listing failed"))?;
            if page.result.objects.len() > maximum
                || !page.result.common_prefixes.is_empty()
                || page.page_token.as_ref().is_some_and(|s| s.len() > 4096)
            {
                return Err(Error::Chunk("object inventory page exceeds bounds"));
            }
            let mut entries = Vec::with_capacity(page.result.objects.len());
            for object in page.result.objects {
                let key = object
                    .location
                    .as_ref()
                    .strip_prefix(&prefix)
                    .ok_or(Error::Chunk("inventory escaped namespace"))?;
                if key == NAMESPACE_MARKER {
                    continue;
                }
                if key.is_empty() || key.len() > 1024 {
                    return Err(Error::Chunk("invalid inventory key"));
                }
                entries.push(ChunkInventoryEntry {
                    key: key.to_owned(),
                    encoded_bytes: object.size,
                });
            }
            Ok(ChunkInventoryPage {
                entries,
                next: page.page_token,
            })
        })
    }
    pub(super) fn empty(&self) -> Result<bool> {
        let page = self.inventory(None, 2)?;
        Ok(page.entries.is_empty() && page.next.is_none())
    }
}
