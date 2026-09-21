//! Authenticated stat evidence without large-file content reads.
use super::*;

#[derive(Serialize)]
pub struct DataStatReport {
    pub path: String,
    pub version: Option<u64>,
    /// Base64 on the wire; the reader is `foks_agent_proto::data::DataStat`,
    /// which uses the same adapter.
    #[serde(with = "foks_agent_proto::base64_bytes::optional")]
    pub dirent: Option<Vec<u8>>,
    #[serde(with = "foks_agent_proto::base64_bytes::optional")]
    pub directory: Option<Vec<u8>>,
    #[serde(with = "foks_agent_proto::base64_bytes::optional")]
    pub node: Option<Vec<u8>>,
    pub size: Option<u64>,
    pub target: Option<String>,
}

impl CheckedProfileSession<'_> {
    /// Reports the authenticated evidence for one addressed path.
    ///
    /// Only the directories on `path` are walked, and the final component is
    /// included so that a directory's own generation is fetched rather than
    /// looked up in a whole-namespace projection. A non-directory final
    /// component is not descended into, so a file costs the same walk its
    /// parent does.
    ///
    /// Absence is still decided the way a complete traversal decides it: a
    /// missing component is missing from the full listing of the directory
    /// that would hold it. Nothing here reads absence from the rest of the
    /// store.
    pub fn data_stat(
        &self,
        alias: &str,
        team_id: Option<&str>,
        path: &str,
        version: Option<u64>,
        vault: &mut AccountVault<'_>,
    ) -> Result<DataStatReport> {
        let (account, user, team) = self.data_context(alias, team_id, vault)?;
        let tree = self.data_path_tree(&account, &user, team.as_ref(), path, KvPathScope::Entry)?;
        if path == "/" {
            if version.is_some() {
                return Err(Error::InvalidKvPath("root has no dirent version"));
            }
            let root = root_directory(&tree)?;
            let root = tree
                .iter()
                .find(|dir| dir.directory_id == root)
                .ok_or(Error::InvalidKvPath("missing root"))?;
            return Ok(DataStatReport {
                path: path.into(),
                version,
                dirent: None,
                directory: Some(root.directory_bytes.clone()),
                node: None,
                size: None,
                target: None,
            });
        }
        let entry = checked_entry(
            &tree,
            path,
            version.ok_or(Error::InvalidKvPath("stat requires an entry version"))?,
        )?;
        let node_id = KvNodeId(entry.node_id);
        let directory = if node_id.node_type()? == KvNodeType::Directory {
            Some(
                tree.iter()
                    .find(|dir| dir.directory_id == node_id.object_id())
                    .ok_or(Error::InvalidKvPath("missing directory"))?
                    .directory_bytes
                    .clone(),
            )
        } else {
            None
        };
        let mut result = DataStatReport {
            path: path.into(),
            version,
            dirent: Some(entry.dirent_bytes.clone()),
            directory,
            node: entry.node_bytes.clone(),
            size: None,
            target: None,
        };
        if matches!(
            node_id.node_type()?,
            KvNodeType::SmallFile | KvNodeType::Symlink
        ) {
            let host = self.pinned_host()?;
            let value = match &team {
                Some(team) => {
                    self.client
                        .read_team_kv_node(&host, &account.credential, team, node_id)?
                }
                None => self.client.read_user_kv_node(
                    &host,
                    &account.credential,
                    &user.verified,
                    &user.puks,
                    node_id,
                )?,
            };
            match value {
                foks_client::KvFetchedNode::SmallFile(mut bytes) => {
                    result.size = Some(bytes.len() as u64);
                    bytes.zeroize();
                }
                foks_client::KvFetchedNode::Symlink(mut bytes) => {
                    result.target = Some(
                        std::str::from_utf8(&bytes)
                            .map_err(|_| Error::InvalidKvPath("invalid symlink text"))?
                            .to_owned(),
                    );
                    bytes.zeroize();
                }
                _ => return Err(Error::InvalidKvPath("stat node type changed")),
            }
        }
        // Preserve Go's large-file stat placeholder even when the catalog can
        // expose the Rust size extension through its separate optional field.
        if node_id.node_type()? == KvNodeType::File {
            result.size = Some(0);
        }
        Ok(result)
    }
}
