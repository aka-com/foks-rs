//! Authenticated stat evidence without large-file content reads.
use super::*;

#[derive(Serialize)]
pub struct DataStatReport {
    pub path: String,
    pub version: Option<u64>,
    pub dirent: Option<Vec<u8>>,
    pub directory: Option<Vec<u8>>,
    pub node: Option<Vec<u8>>,
    pub size: Option<u64>,
    pub target: Option<String>,
}

impl CheckedProfileSession<'_> {
    pub fn data_stat(
        &self,
        alias: &str,
        team_id: Option<&str>,
        path: &str,
        version: Option<u64>,
        vault: &mut AccountVault<'_>,
    ) -> Result<DataStatReport> {
        let (account, user, team) = self.data_context(alias, team_id, vault)?;
        let tree = self.data_tree(&account, &user, team.as_ref())?;
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
        // Go reports zero for a large file: its metadata does not contain the
        // plaintext length. Preserve that behavior without scanning file chunks.
        if node_id.node_type()? == KvNodeType::File {
            result.size = Some(0);
        }
        Ok(result)
    }
}
