//! Moves retain the selected source identity and version through namespace CAS retries.
use super::*;

impl CheckedProfileSession<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn move_kv_checked(
        &self,
        alias: &str,
        path: &str,
        destination: &str,
        version: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.move_kv_selected(alias, path, destination, None, version, vault, master_key)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn move_kv_bound(
        &self,
        alias: &str,
        path: &str,
        destination: &str,
        dirent_id: [u8; 16],
        version: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.move_kv_selected(
            alias,
            path,
            destination,
            Some(dirent_id),
            version,
            vault,
            master_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn move_kv_selected(
        &self,
        alias: &str,
        path: &str,
        destination: &str,
        dirent_id: Option<[u8; 16]>,
        version: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.profile.require(Capability::Kv)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.user_kv_write_session(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        move_checked(&mut session, path, destination, dirent_id, version)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn move_team_kv_checked(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        destination: &str,
        version: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.move_team_kv_selected(
            account_alias,
            team_alias,
            team_id_hex,
            path,
            destination,
            None,
            version,
            vault,
            master_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn move_team_kv_bound(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        destination: &str,
        dirent_id: [u8; 16],
        version: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.move_team_kv_selected(
            account_alias,
            team_alias,
            team_id_hex,
            path,
            destination,
            Some(dirent_id),
            version,
            vault,
            master_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn move_team_kv_selected(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        destination: &str,
        dirent_id: Option<[u8; 16]>,
        version: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.with_team_kv_write_session(
            account_alias,
            team_alias,
            team_id_hex,
            vault,
            master_key,
            |session| move_checked(session, path, destination, dirent_id, version),
        )
    }
}

fn move_checked(
    session: &mut foks_client::KvWriteSession<'_>,
    path: &str,
    destination: &str,
    dirent_id: Option<[u8; 16]>,
    version: u64,
) -> Result<KvWriteReport> {
    let (source_parent_path, source_name) = split_parent(path)?;
    let (destination_parent_path, destination_name) = split_parent(destination)?;
    let tree = session.sync()?;
    let source_parent = resolve_directory(&tree, &source_parent_path)?;
    let destination_parent = resolve_directory(&tree, &destination_parent_path)?;
    let existing = checked_bound_entry(&tree, path, dirent_id, version)?;
    let node = KvNodeId(existing.node_id);
    let read_role = if node.node_type()? == KvNodeType::Directory {
        let directory = tree
            .iter()
            .find(|directory| directory.directory_id == node.object_id())
            .ok_or(Error::InvalidKvPath("missing source directory"))?;
        foks_proto::KvDirectoryPair::decode(&directory.directory_bytes)?
            .active
            .key
            .role
    } else {
        projected_read_role(existing)?
    };
    let options = KvWriteOptions {
        read_role,
        write_role: projected_write_role(existing)?,
        overwrite: false,
        expected_version: None,
    };
    let result = session
        .move_entry_bound(
            source_parent,
            &source_name,
            existing.dirent_id,
            version,
            destination_parent,
            &destination_name,
            options,
        )
        .map_err(checked_write_error)?;
    Ok(KvWriteReport::from_path(
        destination,
        result.dirent_version,
        &result.path,
    ))
}
