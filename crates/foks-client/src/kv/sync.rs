//! Authenticated KV traversal and monotonic soft-state projection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use foks_client_db::{KvDirectoryProjection, KvLargeFileStage, KvProjectedEntry, SoftStateStore};
use foks_crypto::{derive_subkey_id, open_kv_chunk, open_kv_dirent_name};
use foks_proto::{
    KvDirectoryPair, KvDirectoryStatus, KvEncryptedChunk, KvListResponse, KvNode, KvNodeId,
    KvNodeType, KvParty, KvRoot, KvSmallFilePlaintext, SecretSeed, MAXIMUM_KV_DIRECTORIES,
    MAXIMUM_KV_DIRENTS, MAXIMUM_KV_LIST_PAGE_ENTRIES, MAXIMUM_KV_LIST_RESPONSE_BYTES,
};
use foks_rpc::{KvAuth, KvListCursor};
use foks_verify::VerifiedUserState;
use zeroize::{Zeroize as _, Zeroizing};

use super::rpc::KvRequest;
use super::support::{
    kv_key, kv_version_vector, reachable_kv_tree, user_kv_keys, validate_kv_symlink,
};
use super::{KvFetchedChunk, KvFetchedNode};
use super::{KvPrivateKeyRef, KvWriteSession, OwnedKvAuth, MAX_KV_FILE_BYTES};
use crate::{
    AuthenticatedTeamOutcome, DeviceCredential, Error, FoksClient, PinnedHost,
    ProtectedMutationStore, Result, UserPrivateKey, YubiCredential,
};

impl FoksClient {
    /// Fetches and decrypts exactly one node in a personal KV namespace.
    /// Unlike [`Self::sync_user_kv`], this does not stage plaintext.
    pub fn read_user_kv_node(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        node: KvNodeId,
    ) -> Result<KvFetchedNode> {
        if user.uid() != &credential.uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "KV user state does not match the credential and pinned host",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        read_kv_node_with_fetch(node, &keys, KvAuth::User, |auth, request| {
            connection.call(auth, request)
        })
    }

    /// Fetches a bounded plaintext range from one personal large-file node.
    /// Only encrypted chunks for the selected node are requested.
    #[allow(clippy::too_many_arguments)]
    pub fn read_user_kv_chunk(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        node: KvNodeId,
        offset: u64,
        length: usize,
    ) -> Result<KvFetchedChunk> {
        if user.uid() != &credential.uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "KV user state does not match the credential and pinned host",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        read_kv_chunk_with_fetch(
            node,
            offset,
            length,
            &keys,
            KvAuth::User,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Fetches and decrypts exactly one node in a team KV namespace.
    pub fn read_team_kv_node(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        node: KvNodeId,
    ) -> Result<KvFetchedNode> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let keys = team
            .ptks
            .iter()
            .map(|key| KvPrivateKeyRef {
                role: key.role,
                generation: key.generation,
                seed: &key.seed,
            })
            .collect::<Vec<_>>();
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        read_kv_node_with_fetch(
            node,
            &keys,
            KvAuth::Team(&team.view_token),
            |auth, request| connection.call(auth, request),
        )
    }

    /// Fetches a bounded plaintext range from one team large-file node.
    #[allow(clippy::too_many_arguments)]
    pub fn read_team_kv_chunk(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        node: KvNodeId,
        offset: u64,
        length: usize,
    ) -> Result<KvFetchedChunk> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let keys = team
            .ptks
            .iter()
            .map(|key| KvPrivateKeyRef {
                role: key.role,
                generation: key.generation,
                seed: &key.seed,
            })
            .collect::<Vec<_>>();
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        read_kv_chunk_with_fetch(
            node,
            offset,
            length,
            &keys,
            KvAuth::Team(&team.view_token),
            |auth, request| connection.call(auth, request),
        )
    }

    /// Verifies and projects the complete reachable personal KV tree. The PUK
    /// remains caller-owned; only encrypted wire objects and decrypted file
    /// projections are written to the separate soft-state database.
    pub fn sync_user_kv(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if user.uid() != &credential.uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "KV user state does not match the credential and pinned host",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            KvAuth::User,
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Authenticates and traverses the personal KV namespace for a live
    /// catalog without downloading file plaintext or updating the local
    /// content projection.
    pub fn list_user_kv_metadata(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if user.uid() != &credential.uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "KV user state does not match the credential and pinned host",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch_mode(
            host,
            KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            KvAuth::User,
            &keys,
            soft_database_path,
            KvSyncMode::Metadata,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Hardware-backed credential variant of [`Self::sync_user_kv`].
    pub fn sync_user_kv_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if user.uid() != &credential.uid
            || user.host() != host.host_id()
            || !user.devices().iter().any(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
        {
            return Err(Error::CredentialBinding(
                "KV Yubi credential does not match the supplied user state",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            KvAuth::User,
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Verifies and projects the complete reachable team KV tree under the
    /// short-lived bearer token and PTKs returned by `load_and_pin_team`.
    pub fn sync_team_kv(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let keys = team
            .ptks
            .iter()
            .map(|key| KvPrivateKeyRef {
                role: key.role,
                generation: key.generation,
                seed: &key.seed,
            })
            .collect::<Vec<_>>();
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            KvAuth::Team(&team.view_token),
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Authenticates and traverses a team KV namespace for a live catalog
    /// without downloading file plaintext or updating the local projection.
    pub fn list_team_kv_metadata(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let keys = team
            .ptks
            .iter()
            .map(|key| KvPrivateKeyRef {
                role: key.role,
                generation: key.generation,
                seed: &key.seed,
            })
            .collect::<Vec<_>>();
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch_mode(
            host,
            KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            KvAuth::Team(&team.view_token),
            &keys,
            soft_database_path,
            KvSyncMode::Metadata,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Hardware-backed credential variant of [`Self::sync_team_kv`].
    pub fn sync_team_kv_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let keys = team
            .ptks
            .iter()
            .map(|key| KvPrivateKeyRef {
                role: key.role,
                generation: key.generation,
                seed: &key.seed,
            })
            .collect::<Vec<_>>();
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            KvAuth::Team(&team.view_token),
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    pub fn user_kv_write_session<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &'a [UserPrivateKey],
        soft_database_path: &Path,
        protected_store: &'a mut impl ProtectedMutationStore,
    ) -> Result<KvWriteSession<'a>> {
        if user.uid() != &credential.uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "KV user state does not match the credential and pinned host",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        Ok(KvWriteSession {
            client: self,
            host: host.clone(),
            party: KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            auth: OwnedKvAuth::User,
            private_keys: keys,
            protected_store,
            soft_database_path: soft_database_path.to_owned(),
            connection,
        })
    }

    pub fn user_kv_write_session_yubi<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        puks: &'a [UserPrivateKey],
        soft_database_path: &Path,
        protected_store: &'a mut impl ProtectedMutationStore,
    ) -> Result<KvWriteSession<'a>> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if user.uid() != &credential.uid
            || user.host() != host.host_id()
            || !user.devices().iter().any(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
        {
            return Err(Error::CredentialBinding(
                "KV Yubi credential does not match the supplied user state",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let connection = self.kv_connection_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        Ok(KvWriteSession {
            client: self,
            host: host.clone(),
            party: KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            auth: OwnedKvAuth::User,
            private_keys: keys,
            protected_store,
            soft_database_path: soft_database_path.to_owned(),
            connection,
        })
    }

    pub fn team_kv_write_session<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &'a AuthenticatedTeamOutcome,
        soft_database_path: &Path,
        protected_store: &'a mut impl ProtectedMutationStore,
    ) -> Result<KvWriteSession<'a>> {
        self.team_kv_write_session_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
            team,
            soft_database_path,
            protected_store,
        )
    }

    pub fn team_kv_write_session_yubi<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &'a AuthenticatedTeamOutcome,
        soft_database_path: &Path,
        protected_store: &'a mut impl ProtectedMutationStore,
    ) -> Result<KvWriteSession<'a>> {
        self.team_kv_write_session_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
            team,
            soft_database_path,
            protected_store,
        )
    }

    fn team_kv_write_session_with_material<'a>(
        &'a self,
        host: &PinnedHost,
        device_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        team: &'a AuthenticatedTeamOutcome,
        soft_database_path: &Path,
        protected_store: &'a mut impl ProtectedMutationStore,
    ) -> Result<KvWriteSession<'a>> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let connection = self.kv_connection_with_material(host, device_seed, certificate_chain)?;
        Ok(KvWriteSession {
            client: self,
            host: host.clone(),
            party: KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            auth: OwnedKvAuth::Team(team.view_token),
            private_keys: team
                .ptks
                .iter()
                .map(|key| KvPrivateKeyRef {
                    role: key.role,
                    generation: key.generation,
                    seed: &key.seed,
                })
                .collect(),
            protected_store,
            soft_database_path: soft_database_path.to_owned(),
            connection,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_kv_with_fetch<F>(
        &self,
        host: &PinnedHost,
        party: KvParty,
        auth: KvAuth<'_>,
        private_keys: &[KvPrivateKeyRef<'_>],
        soft_database_path: &Path,
        fetch: F,
    ) -> Result<Vec<KvDirectoryProjection>>
    where
        F: FnMut(KvAuth<'_>, &KvRequest) -> Result<Vec<u8>>,
    {
        self.sync_kv_with_fetch_mode(
            host,
            party,
            auth,
            private_keys,
            soft_database_path,
            KvSyncMode::Content,
            fetch,
        )
    }

    pub(crate) fn list_kv_metadata_with_fetch<F>(
        &self,
        host: &PinnedHost,
        party: KvParty,
        auth: KvAuth<'_>,
        private_keys: &[KvPrivateKeyRef<'_>],
        soft_database_path: &Path,
        fetch: F,
    ) -> Result<Vec<KvDirectoryProjection>>
    where
        F: FnMut(KvAuth<'_>, &KvRequest) -> Result<Vec<u8>>,
    {
        self.sync_kv_with_fetch_mode(
            host,
            party,
            auth,
            private_keys,
            soft_database_path,
            KvSyncMode::Metadata,
            fetch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn sync_kv_with_fetch_mode<F>(
        &self,
        host: &PinnedHost,
        party: KvParty,
        auth: KvAuth<'_>,
        private_keys: &[KvPrivateKeyRef<'_>],
        soft_database_path: &Path,
        mode: KvSyncMode,
        mut fetch: F,
    ) -> Result<Vec<KvDirectoryProjection>>
    where
        F: FnMut(KvAuth<'_>, &KvRequest) -> Result<Vec<u8>>,
    {
        const PAGE_SIZE: u64 = MAXIMUM_KV_LIST_PAGE_ENTRIES as u64;
        const MAX_PAGES_PER_DIRECTORY: usize = 4096;
        const MAX_FILE_CHUNKS: usize = 4096;
        const MAX_TOTAL_CONTENT_BYTES: usize = 512 * 1024 * 1024;

        for _ in 0..3 {
            let attempt = (|| -> Result<Vec<KvDirectoryProjection>> {
                let mut store = SoftStateStore::open(soft_database_path)?;
                // Go's PathVersionVector is a partial assertion: a successful
                // cache check proves that cited entries are unchanged, but it
                // cannot prove that a peer did not add an uncited entry. A
                // complete namespace projection therefore requires a fresh
                // traversal; the final cache check below still closes races.
                let root_bytes = match fetch(auth, &KvRequest::Root) {
                    Ok(bytes) => bytes,
                    Err(error @ Error::Rpc(foks_rpc::Error::RemoteStatus { code: 8016, .. })) => {
                        // Go accounts may not have written their first item yet.
                        // Only absence of the root itself means an empty store;
                        // missing descendants and disappearance of a known root
                        // remain errors. Reads never create a namespace.
                        if store.has_kv_root(party.host.as_bytes(), party.party.as_bytes())? {
                            return Err(error);
                        }
                        return Ok(Vec::new());
                    }
                    Err(error) => return Err(error),
                };
                let root = KvRoot::decode(&root_bytes)?;
                let mut queue = VecDeque::from([root.root]);
                if root.version == 0 {
                    return Err(Error::KvResponse("root version is zero"));
                }
                let root_keys = kv_key(private_keys, root.key.role, root.key.generation)?;
                root_keys.verify_root(&root, &party)?;

                let mut combined = BTreeMap::new();
                let mut large_files: Vec<KvLargeFileStage> = Vec::new();
                let mut visited = BTreeSet::new();
                let mut projections = Vec::new();
                let mut total_entries = 0usize;
                // Large files stream directly into SQLite and therefore do
                // not consume this in-memory plaintext budget.
                let mut total_inline_content_bytes = 0usize;
                while let Some(directory_id) = queue.pop_front() {
                    if !visited.insert(directory_id) {
                        continue;
                    }
                    if visited.len() > MAXIMUM_KV_DIRECTORIES {
                        return Err(Error::KvResponse("directory traversal limit exceeded"));
                    }
                    let directory_bytes = fetch(auth, &KvRequest::Directory(directory_id))?;
                    let directory = KvDirectoryPair::decode(&directory_bytes)?;
                    if directory.active.id != directory_id
                        || directory.active.status != KvDirectoryStatus::Active
                        || directory.encrypting.as_ref().is_some_and(|encrypting| {
                            encrypting.id != directory_id
                                || encrypting.status != KvDirectoryStatus::Encrypting
                        })
                    {
                        return Err(Error::KvResponse("directory response ID mismatch"));
                    }
                    let mut directory_seeds = BTreeMap::new();
                    for generation in
                        std::iter::once(&directory.active).chain(directory.encrypting.iter())
                    {
                        let keys =
                            kv_key(private_keys, generation.key.role, generation.key.generation)?;
                        let seed = keys.open_directory_seed(generation)?;
                        if directory_seeds.insert(generation.version, seed).is_some() {
                            return Err(Error::KvResponse(
                                "duplicate directory encryption version",
                            ));
                        }
                    }

                    let mut cursor = KvListCursor::None;
                    let mut entries = Vec::new();
                    let mut entry_ids = BTreeSet::new();
                    let mut names = BTreeSet::new();
                    let mut final_page = false;
                    for _ in 0..MAX_PAGES_PER_DIRECTORY {
                        let list_bytes = fetch(
                            auth,
                            &KvRequest::List {
                                directory: directory_id,
                                cursor,
                                number: PAGE_SIZE,
                                // Metadata traversal must authenticate each small node's
                                // encrypted role header even though it discards plaintext.
                                // Asking kvList for its existing extended side table avoids
                                // one extra RPC per small file and keeps small-file-heavy
                                // traversals within the server's per-session request budget.
                                load_small_files: true,
                            },
                        )?;
                        if list_bytes.len() > MAXIMUM_KV_LIST_RESPONSE_BYTES {
                            return Err(Error::KvResponse("KV list response is too large"));
                        }
                        let listing = KvListResponse::decode(&list_bytes)?;
                        if listing.entries.len() > MAXIMUM_KV_LIST_PAGE_ENTRIES
                            || listing.extended.len() > listing.entries.len()
                        {
                            return Err(Error::KvResponse("KV list response exceeds page limits"));
                        }
                        if !listing.final_page && listing.entries.is_empty() {
                            return Err(Error::KvResponse("non-final KV list page is empty"));
                        }
                        let mut extended = BTreeMap::new();
                        for item in listing.extended {
                            let position = usize::try_from(item.position).map_err(|_| {
                                Error::KvResponse("extended dirent position overflow")
                            })?;
                            if position >= listing.entries.len()
                                || extended.insert(position, item.small_file).is_some()
                            {
                                return Err(Error::KvResponse("invalid extended dirent position"));
                            }
                        }
                        let last_mac = listing.entries.last().map(|entry| entry.name_mac);
                        for (position, mut entry) in listing.entries.into_iter().enumerate() {
                            total_entries = total_entries
                                .checked_add(1)
                                .ok_or(Error::KvResponse("entry count overflow"))?;
                            if total_entries > MAXIMUM_KV_DIRENTS {
                                return Err(Error::KvResponse("entry traversal limit exceeded"));
                            }
                            entry.bind_list_parent(directory_id)?;
                            if !entry_ids.insert(entry.id) {
                                return Err(Error::KvResponse("duplicate directory entry ID"));
                            }
                            let seed = directory_seeds.get(&entry.directory_version).ok_or(
                                Error::KvResponse("dirent uses an unavailable directory key"),
                            )?;
                            let name = open_kv_dirent_name(seed, &entry)?.name;
                            if std::str::from_utf8(&name).is_err()
                                || name.contains(&0)
                                || name.contains(&b'/')
                                || name == b"."
                                || name == b".."
                                || !names.insert(name.clone())
                            {
                                return Err(Error::KvResponse(
                                    "invalid or duplicate plaintext name",
                                ));
                            }
                            if entry.value.node_type()? == KvNodeType::None {
                                if extended.remove(&position).is_some() {
                                    return Err(Error::KvResponse(
                                        "tombstone has extended file data",
                                    ));
                                }
                                continue;
                            }
                            let mut projected = KvProjectedEntry {
                                dirent_id: entry.id,
                                node_id: entry.value.0,
                                version: entry.version,
                                directory_version: entry.directory_version,
                                name,
                                write_role_type: entry.write_role.protocol_value(),
                                write_role_visibility: i64::from(
                                    entry.write_role.visibility().unwrap_or(0),
                                ),
                                creation_time: entry.creation_time,
                                dirent_bytes: entry.encoded().to_vec(),
                                node_bytes: None,
                                content: None,
                                symlink: None,
                                large_file_size: None,
                            };
                            match entry.value.node_type()? {
                                KvNodeType::Directory => {
                                    let child = entry.value.object_id();
                                    queue.push_back(child);
                                }
                                KvNodeType::SmallFile => {
                                    let (boxed, exact) = match extended.remove(&position) {
                                        Some(boxed) => (boxed, None),
                                        None => {
                                            let bytes = fetch(auth, &KvRequest::Node(entry.value))?;
                                            let KvNode::SmallFile(boxed) = KvNode::decode(&bytes)?
                                            else {
                                                return Err(Error::KvResponse(
                                                    "small-file node type mismatch",
                                                ));
                                            };
                                            (boxed, Some(bytes))
                                        }
                                    };
                                    // The extended response omits the exact node frame, but
                                    // content consumers still need authenticated key-role/type
                                    // metadata. Re-encoding the decoded ciphertext is the same
                                    // fallback metadata mode uses and retains no extra plaintext.
                                    projected.node_bytes = Some(match exact {
                                        Some(bytes) => bytes,
                                        None => KvNode::SmallFile(boxed.clone()).encoded()?,
                                    });
                                    let keys =
                                        kv_key(private_keys, boxed.key.role, boxed.key.generation)?;
                                    let KvSmallFilePlaintext::File(mut content) =
                                        keys.open_small_file(entry.value, &boxed)?
                                    else {
                                        return Err(Error::KvResponse(
                                            "small-file plaintext type mismatch",
                                        ));
                                    };
                                    if mode == KvSyncMode::Content {
                                        total_inline_content_bytes = total_inline_content_bytes
                                            .checked_add(content.len())
                                            .ok_or(Error::KvResponse("content size overflow"))?;
                                        projected.content = Some(content);
                                    } else {
                                        content.zeroize();
                                    }
                                }
                                KvNodeType::Symlink => {
                                    let bytes = fetch(auth, &KvRequest::Node(entry.value))?;
                                    let KvNode::Symlink(boxed) = KvNode::decode(&bytes)? else {
                                        return Err(Error::KvResponse(
                                            "symlink node type mismatch",
                                        ));
                                    };
                                    projected.node_bytes = Some(bytes);
                                    let keys =
                                        kv_key(private_keys, boxed.key.role, boxed.key.generation)?;
                                    let KvSmallFilePlaintext::Symlink(mut target) =
                                        keys.open_small_file(entry.value, &boxed)?
                                    else {
                                        return Err(Error::KvResponse(
                                            "symlink plaintext type mismatch",
                                        ));
                                    };
                                    validate_kv_symlink(&target)?;
                                    if mode == KvSyncMode::Content {
                                        total_inline_content_bytes = total_inline_content_bytes
                                            .checked_add(target.len())
                                            .ok_or(Error::KvResponse("content size overflow"))?;
                                        projected.symlink = Some(target);
                                    } else {
                                        target.zeroize();
                                    }
                                }
                                KvNodeType::File => {
                                    let bytes = fetch(auth, &KvRequest::Node(entry.value))?;
                                    let KvNode::File(metadata) = KvNode::decode(&bytes)? else {
                                        return Err(Error::KvResponse(
                                            "large-file node type mismatch",
                                        ));
                                    };
                                    projected.node_bytes = Some(bytes);
                                    if mode == KvSyncMode::Metadata {
                                        let keys = kv_key(
                                            private_keys,
                                            metadata.key.role,
                                            metadata.key.generation,
                                        )?;
                                        let _file_seed =
                                            keys.open_file_seed(entry.value, &metadata)?;
                                        entries.push(projected);
                                        continue;
                                    }
                                    let keys = kv_key(
                                        private_keys,
                                        metadata.key.role,
                                        metadata.key.generation,
                                    )?;
                                    let size = if let Some(stage) = large_files
                                        .iter()
                                        .find(|stage| stage.node_id == entry.value.0)
                                    {
                                        stage.size
                                    } else {
                                        let file_seed =
                                            keys.open_file_seed(entry.value, &metadata)?;
                                        let mut stage = store.begin_large_file(
                                            host.host_id.as_bytes(),
                                            party.party.as_bytes(),
                                            entry.value.0,
                                        )?;
                                        let mut offset = 0u64;
                                        let mut complete = false;
                                        for _ in 0..MAX_FILE_CHUNKS {
                                            let chunk_bytes = fetch(
                                                auth,
                                                &KvRequest::Chunk {
                                                    file: entry.value,
                                                    offset,
                                                },
                                            )?;
                                            let chunk = KvEncryptedChunk::decode(&chunk_bytes)?;
                                            let clear = open_kv_chunk(
                                                &file_seed,
                                                entry.value,
                                                offset,
                                                &chunk,
                                            )?;
                                            if clear.is_empty() && !chunk.final_chunk {
                                                return Err(Error::KvResponse(
                                                    "empty non-final file chunk",
                                                ));
                                            }
                                            validate_large_file_append(stage.size, clear.len())?;
                                            store.append_large_file(&mut stage, &clear)?;
                                            offset = stage.size;
                                            if chunk.final_chunk {
                                                complete = true;
                                                break;
                                            }
                                        }
                                        if !complete {
                                            return Err(Error::KvResponse(
                                                "file chunk limit exceeded",
                                            ));
                                        }
                                        store.finish_large_file(&stage)?;
                                        let size = stage.size;
                                        large_files.push(stage);
                                        size
                                    };
                                    projected.large_file_size = Some(size);
                                }
                                KvNodeType::None => unreachable!("rejected above"),
                            }
                            entries.push(projected);
                            if total_inline_content_bytes > MAX_TOTAL_CONTENT_BYTES {
                                return Err(Error::KvResponse("total content limit exceeded"));
                            }
                        }
                        if !extended.is_empty() {
                            return Err(Error::KvResponse(
                                "extended entry does not reference a small file",
                            ));
                        }
                        if listing.final_page {
                            final_page = true;
                            break;
                        }
                        cursor = KvListCursor::Mac(
                            last_mac.ok_or(Error::KvResponse("list page lacks pagination MAC"))?,
                        );
                    }
                    if !final_page {
                        return Err(Error::KvResponse("directory pagination limit exceeded"));
                    }
                    entries.sort_by(|left, right| {
                        left.name
                            .cmp(&right.name)
                            .then(left.dirent_id.cmp(&right.dirent_id))
                    });
                    let projection = KvDirectoryProjection {
                        host_id: host.host_id.as_bytes().to_vec(),
                        party_id: party.party.as_bytes().to_vec(),
                        root_version: root.version,
                        root_directory_id: root.root,
                        root_bytes: root_bytes.clone(),
                        directory_id,
                        directory_version: directory.active.version,
                        directory_bytes,
                        entries,
                    };
                    combined.insert(directory_id, projection.clone());
                    projections.push(projection);
                }
                let combined = reachable_kv_tree(root.root, combined)?;
                let final_versions = kv_version_vector(root.version, &combined);
                fetch(auth, &KvRequest::CacheCheck(final_versions))?;
                if mode == KvSyncMode::Metadata {
                    store.project_reachable_metadata(&projections)?;
                    return Ok(projections);
                }
                store.project_reachable_tree_with_large_files(&projections, &large_files)?;
                Ok(store.tree(host.host_id.as_bytes(), party.party.as_bytes())?)
            })();
            match attempt {
                Err(Error::Rpc(foks_rpc::Error::KvStaleCache(_))) => continue,
                result => return result,
            }
        }
        Err(Error::KvResponse(
            "KV cache changed during three synchronization attempts",
        ))
    }
}

pub(crate) fn read_kv_node_with_fetch<F>(
    node: KvNodeId,
    private_keys: &[KvPrivateKeyRef<'_>],
    auth: KvAuth<'_>,
    mut fetch: F,
) -> Result<KvFetchedNode>
where
    F: FnMut(KvAuth<'_>, &KvRequest) -> Result<Vec<u8>>,
{
    let bytes = fetch(auth, &KvRequest::Node(node))?;
    match (node.node_type()?, KvNode::decode(&bytes)?) {
        (KvNodeType::SmallFile, KvNode::SmallFile(boxed)) => {
            let keys = kv_key(private_keys, boxed.key.role, boxed.key.generation)?;
            let KvSmallFilePlaintext::File(content) = keys.open_small_file(node, &boxed)? else {
                return Err(Error::KvResponse("small-file plaintext type mismatch"));
            };
            Ok(KvFetchedNode::SmallFile(content))
        }
        (KvNodeType::Symlink, KvNode::Symlink(boxed)) => {
            let keys = kv_key(private_keys, boxed.key.role, boxed.key.generation)?;
            let KvSmallFilePlaintext::Symlink(target) = keys.open_small_file(node, &boxed)? else {
                return Err(Error::KvResponse("symlink plaintext type mismatch"));
            };
            validate_kv_symlink(&target)?;
            Ok(KvFetchedNode::Symlink(target))
        }
        (KvNodeType::Directory, KvNode::Directory(directory)) => {
            if directory.active.id != node.object_id()
                || directory.active.status != KvDirectoryStatus::Active
                || directory.encrypting.as_ref().is_some_and(|encrypting| {
                    encrypting.id != node.object_id()
                        || encrypting.status != KvDirectoryStatus::Encrypting
                })
            {
                return Err(Error::KvResponse("directory response ID mismatch"));
            }
            for generation in std::iter::once(&directory.active).chain(directory.encrypting.iter())
            {
                let keys = kv_key(private_keys, generation.key.role, generation.key.generation)?;
                let _ = keys.open_directory_seed(generation)?;
            }
            Ok(KvFetchedNode::Directory)
        }
        (KvNodeType::File, KvNode::File(metadata)) => {
            let keys = kv_key(private_keys, metadata.key.role, metadata.key.generation)?;
            let file_seed = keys.open_file_seed(node, &metadata)?;
            let mut offset = 0u64;
            for _ in 0..4096 {
                let chunk_bytes = fetch(auth, &KvRequest::Chunk { file: node, offset })?;
                let chunk = KvEncryptedChunk::decode(&chunk_bytes)?;
                let clear = Zeroizing::new(open_kv_chunk(&file_seed, node, offset, &chunk)?);
                if clear.is_empty() && !chunk.final_chunk {
                    return Err(Error::KvResponse("empty non-final file chunk"));
                }
                offset = validate_large_file_append(offset, clear.len())?;
                if chunk.final_chunk {
                    return Ok(KvFetchedNode::LargeFile { size: offset });
                }
            }
            Err(Error::KvResponse("file chunk limit exceeded"))
        }
        _ => Err(Error::KvResponse("KV node response type mismatch")),
    }
}

pub(crate) fn read_kv_chunk_with_fetch<F>(
    node: KvNodeId,
    requested_offset: u64,
    length: usize,
    private_keys: &[KvPrivateKeyRef<'_>],
    auth: KvAuth<'_>,
    mut fetch: F,
) -> Result<KvFetchedChunk>
where
    F: FnMut(KvAuth<'_>, &KvRequest) -> Result<Vec<u8>>,
{
    if node.node_type()? != KvNodeType::File {
        return Err(Error::KvResponse("KV chunk node is not a large file"));
    }
    let node_bytes = fetch(auth, &KvRequest::Node(node))?;
    let KvNode::File(metadata) = KvNode::decode(&node_bytes)? else {
        return Err(Error::KvResponse("large-file node type mismatch"));
    };
    let keys = kv_key(private_keys, metadata.key.role, metadata.key.generation)?;
    let file_seed = keys.open_file_seed(node, &metadata)?;
    let requested_end = requested_offset
        .checked_add(u64::try_from(length).map_err(|_| Error::KvResponse("KV range overflow"))?)
        .ok_or(Error::KvResponse("KV range overflow"))?;
    if length == 0 {
        return Err(Error::KvResponse("KV range length is zero"));
    }

    // Desktop callers advance by their prior response length. Most FOKS
    // writers use stable chunk sizes, so that offset is normally an exact
    // stored-chunk boundary. Try it first; an older writer may have chosen a
    // larger chunk, in which case the server's no-entry response is safe to
    // fall back from to the bounded sequential scan below.
    if requested_offset >= length as u64 {
        match fetch(
            auth,
            &KvRequest::Chunk {
                file: node,
                offset: requested_offset,
            },
        ) {
            Ok(chunk_bytes) => {
                let chunk = KvEncryptedChunk::decode(&chunk_bytes)?;
                let clear =
                    Zeroizing::new(open_kv_chunk(&file_seed, node, requested_offset, &chunk)?);
                if clear.is_empty() && !chunk.final_chunk {
                    return Err(Error::KvResponse("empty non-final file chunk"));
                }
                let chunk_end = validate_large_file_append(requested_offset, clear.len())?;
                let count = length.min(clear.len());
                return Ok(KvFetchedChunk {
                    content: clear[..count].to_vec(),
                    eof: chunk.final_chunk && requested_end >= chunk_end,
                });
            }
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: 8016, .. })) => {}
            Err(error) => return Err(error),
        }
    }
    let mut output = Vec::with_capacity(length);
    let mut chunk_offset = 0u64;
    for _ in 0..4096 {
        let chunk_bytes = fetch(
            auth,
            &KvRequest::Chunk {
                file: node,
                offset: chunk_offset,
            },
        )?;
        let chunk = KvEncryptedChunk::decode(&chunk_bytes)?;
        let clear = Zeroizing::new(open_kv_chunk(&file_seed, node, chunk_offset, &chunk)?);
        if clear.is_empty() && !chunk.final_chunk {
            return Err(Error::KvResponse("empty non-final file chunk"));
        }
        let chunk_end = validate_large_file_append(chunk_offset, clear.len())?;
        if chunk_end > requested_offset && chunk_offset < requested_end {
            let start = requested_offset.saturating_sub(chunk_offset) as usize;
            let end = requested_end.min(chunk_end).saturating_sub(chunk_offset) as usize;
            output.extend_from_slice(&clear[start..end]);
        }
        if chunk.final_chunk {
            if requested_offset > chunk_end {
                return Err(Error::KvResponse("KV range starts beyond end of file"));
            }
            return Ok(KvFetchedChunk {
                content: output,
                eof: requested_end >= chunk_end,
            });
        }
        chunk_offset = chunk_end;
        if chunk_offset >= requested_end {
            return Ok(KvFetchedChunk {
                content: output,
                eof: false,
            });
        }
    }
    Err(Error::KvResponse("file chunk limit exceeded"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KvSyncMode {
    Content,
    Metadata,
}

fn validate_large_file_append(current_size: u64, chunk_size: usize) -> Result<u64> {
    let chunk_size =
        u64::try_from(chunk_size).map_err(|_| Error::KvResponse("file chunk size overflow"))?;
    let next_size = current_size
        .checked_add(chunk_size)
        .ok_or(Error::KvResponse("file size overflow"))?;
    if next_size > MAX_KV_FILE_BYTES {
        return Err(Error::KvResponse("file size limit exceeded"));
    }
    Ok(next_size)
}

#[cfg(test)]
mod capacity_tests {
    use super::*;

    #[test]
    fn synchronization_accepts_the_complete_upstream_file_range() {
        let former_limit = 128 * 1024 * 1024;
        assert_eq!(
            validate_large_file_append(former_limit, 1).unwrap(),
            former_limit + 1
        );
        assert_eq!(
            validate_large_file_append(MAX_KV_FILE_BYTES - 1, 1).unwrap(),
            MAX_KV_FILE_BYTES
        );
    }

    #[test]
    fn synchronization_rejects_only_above_the_upstream_file_limit() {
        assert!(matches!(
            validate_large_file_append(MAX_KV_FILE_BYTES, 1),
            Err(Error::KvResponse("file size limit exceeded"))
        ));
        assert!(matches!(
            validate_large_file_append(u64::MAX, 1),
            Err(Error::KvResponse("file size overflow"))
        ));
    }
}
