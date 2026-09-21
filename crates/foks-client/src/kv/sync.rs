//! Authenticated KV traversal and monotonic soft-state projection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use foks_client_db::{KvDirectoryProjection, KvLargeFileStage, KvProjectedEntry, SoftStateStore};
use foks_crypto::{derive_subkey_id, open_kv_chunk, open_kv_dirent_name};
use foks_proto::{
    KvDirectoryPair, KvDirectoryStatus, KvEncryptedChunk, KvListResponse, KvNode, KvNodeId,
    KvNodeType, KvParty, KvPathVersionVector, KvRoot, KvSmallFilePlaintext, SecretSeed,
    MAXIMUM_KV_DIRECTORIES, MAXIMUM_KV_DIRENTS, MAXIMUM_KV_LIST_PAGE_ENTRIES,
    MAXIMUM_KV_LIST_RESPONSE_BYTES,
};
use foks_rpc::{KvAuth, KvListCursor};
use foks_verify::VerifiedUserState;
use zeroize::{Zeroize as _, Zeroizing};

use super::rpc::KvRequest;
use super::support::{
    is_kv_stale_cache, kv_key, kv_version_vector, kv_version_vector_from_tree, reachable_kv_tree,
    user_kv_keys, validate_kv_symlink,
};
use super::{KvFetchedChunk, KvFetchedNode};
use super::{KvPrivateKeyRef, KvWriteSession, OwnedKvAuth, MAX_KV_FILE_BYTES};
use crate::{
    AuthenticatedTeamOutcome, DeviceCredential, Error, FoksClient, PinnedHost,
    ProtectedMutationStore, Result, UserPrivateKey, YubiCredential,
};

impl FoksClient {
    /// Read server accounting without initializing a KV root or uploading data.
    pub fn kv_usage(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: Option<&AuthenticatedTeamOutcome>,
    ) -> Result<foks_proto::KvUsage> {
        if team.is_some_and(|t| t.verified.host() != host.host_id()) {
            return Err(Error::TeamBinding("KV usage team belongs to another host"));
        }
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        let auth = team.map_or(KvAuth::User, |t| KvAuth::Team(&t.view_token));
        Ok(foks_proto::KvUsage::decode(
            &connection.call(auth, &KvRequest::Usage)?,
        )?)
    }

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

    /// [`Self::read_user_kv_chunk`] guarded by a version vector.
    ///
    /// A caller that resolved this node's path earlier and kept the vector
    /// that walk cited can read further chunks without walking the path
    /// again. The vector is submitted as its own request, because a chunk
    /// request carries no precondition field, and the server answers it
    /// against the current directory and dirent heads.
    ///
    /// `Ok(None)` is that answer coming back stale or naming a cited object
    /// the server no longer has. It means the caller's resolved path is no
    /// longer evidence about this node — a peer may have unlinked the dirent
    /// and created another one at the same name — so the caller must walk the
    /// path again rather than read under the node it remembered. Every other
    /// failure is the caller's to report.
    #[allow(clippy::too_many_arguments)]
    pub fn read_user_kv_chunk_if_current(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        node: KvNodeId,
        offset: u64,
        length: usize,
        precondition: &KvPathVersionVector,
    ) -> Result<Option<KvFetchedChunk>> {
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
        if !kv_precondition_holds(&mut connection, KvAuth::User, precondition)? {
            return Ok(None);
        }
        read_kv_chunk_with_fetch(
            node,
            offset,
            length,
            &keys,
            KvAuth::User,
            |auth, request| connection.call(auth, request),
        )
        .map(Some)
    }

    /// Team variant of [`Self::read_user_kv_chunk_if_current`].
    #[allow(clippy::too_many_arguments)]
    pub fn read_team_kv_chunk_if_current(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        node: KvNodeId,
        offset: u64,
        length: usize,
        precondition: &KvPathVersionVector,
    ) -> Result<Option<KvFetchedChunk>> {
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
        let auth = KvAuth::Team(&team.view_token);
        if !kv_precondition_holds(&mut connection, auth, precondition)? {
            return Ok(None);
        }
        read_kv_chunk_with_fetch(node, offset, length, &keys, auth, |auth, request| {
            connection.call(auth, request)
        })
        .map(Some)
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
            KvTraversalPlan::Complete,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Authenticates only the directories along `components` in the personal
    /// KV namespace instead of traversing every directory in the store. The
    /// result is the root's projection followed by one projection per
    /// resolved component, and it stops where a component is absent or
    /// unreadable, so the ordinary path helpers report the same outcome they
    /// would against a complete traversal.
    pub fn resolve_user_kv_path(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        soft_database_path: &Path,
        components: &[Vec<u8>],
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
        self.resolve_kv_path_with_fetch(
            host,
            KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            KvAuth::User,
            &keys,
            soft_database_path,
            components,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Team variant of [`Self::resolve_user_kv_path`].
    pub fn resolve_team_kv_path(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        soft_database_path: &Path,
        components: &[Vec<u8>],
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
        self.resolve_kv_path_with_fetch(
            host,
            KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            KvAuth::Team(&team.view_token),
            &keys,
            soft_database_path,
            components,
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

    /// Verifies and projects the readable team KV tree under the short-lived
    /// bearer token and PTKs returned by `load_and_pin_team`. Explicitly denied
    /// children retain their authenticated dirents but no cached node payload.
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
            KvTraversalPlan::Complete,
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
            adapter_parent: None,
            adapter_completion: false,
            scope: None,
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
            adapter_parent: None,
            adapter_completion: false,
            scope: None,
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
            adapter_parent: None,
            adapter_completion: false,
            scope: None,
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
            KvTraversalPlan::Complete,
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
            KvTraversalPlan::Complete,
            fetch,
        )
    }

    /// Fetches, authenticates and projects only the directories named by
    /// `components`, starting at the root. Every directory it uses passes the
    /// same root verification, seed opening, identity, pagination, dirent and
    /// node checks as [`Self::sync_kv_with_fetch`], and the closing cache
    /// check cites exactly the directories the walk used. The result is the
    /// path's projections, root first, and stops early where a component is
    /// absent or unreadable, exactly as a complete traversal would leave it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve_kv_path_with_fetch<F>(
        &self,
        host: &PinnedHost,
        party: KvParty,
        auth: KvAuth<'_>,
        private_keys: &[KvPrivateKeyRef<'_>],
        soft_database_path: &Path,
        components: &[Vec<u8>],
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
            KvTraversalPlan::Path(components),
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
        plan: KvTraversalPlan<'_>,
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
                // Reuse projected ciphertext for large-file and symlink metadata.
                // Cached bytes follow the same decoding and authentication path as
                // fetched bytes. Small files already arrive in list responses, and
                // directory traversal fetches directory nodes directly.
                //
                // Content synchronization still fetches every node and chunk; caching
                // here applies only to metadata synchronization.
                let cached_nodes = if mode == KvSyncMode::Metadata {
                    store.cached_node_bytes(
                        host.host_id.as_bytes(),
                        party.party.as_bytes(),
                        &[KvNodeType::File as u8, KvNodeType::Symlink as u8],
                    )?
                } else {
                    BTreeMap::new()
                };
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
                let mut inaccessible_directories = BTreeSet::new();
                let mut projections = Vec::new();
                let mut total_entries = 0usize;
                // Large files stream directly into SQLite and therefore do
                // not consume this in-memory plaintext budget.
                let mut total_inline_content_bytes = 0usize;
                // A path walk consumes one component per directory it visits,
                // in order, so the number already visited names the next one.
                let mut depth = 0usize;
                while let Some(directory_id) = queue.pop_front() {
                    if !visited.insert(directory_id) {
                        continue;
                    }
                    if visited.len() > MAXIMUM_KV_DIRECTORIES {
                        return Err(Error::KvResponse("directory traversal limit exceeded"));
                    }
                    let directory_bytes = match fetch(auth, &KvRequest::Directory(directory_id)) {
                        Err(error)
                            if directory_id != root.root && is_kv_permission_denied(&error) =>
                        {
                            inaccessible_directories.insert(directory_id);
                            continue;
                        }
                        result => result?,
                    };
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
                    let mut children: Vec<[u8; 16]> = Vec::new();
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
                                readable: true,
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
                                    children.push(child);
                                }
                                KvNodeType::SmallFile => {
                                    let (boxed, exact) = match extended.remove(&position) {
                                        Some(boxed) => (boxed, None),
                                        None => {
                                            let bytes =
                                                match fetch(auth, &KvRequest::Node(entry.value)) {
                                                    Err(error)
                                                        if is_kv_permission_denied(&error) =>
                                                    {
                                                        projected.readable = false;
                                                        entries.push(projected);
                                                        continue;
                                                    }
                                                    result => result?,
                                                };
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
                                    let bytes = match cached_kv_node(
                                        &cached_nodes,
                                        entry.value,
                                        private_keys,
                                    ) {
                                        Some(bytes) => bytes,
                                        None => match fetch(auth, &KvRequest::Node(entry.value)) {
                                            Err(error) if is_kv_permission_denied(&error) => {
                                                projected.readable = false;
                                                entries.push(projected);
                                                continue;
                                            }
                                            result => result?,
                                        },
                                    };
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
                                    let bytes = match cached_kv_node(
                                        &cached_nodes,
                                        entry.value,
                                        private_keys,
                                    ) {
                                        Some(bytes) => bytes,
                                        None => match fetch(auth, &KvRequest::Node(entry.value)) {
                                            Err(error) if is_kv_permission_denied(&error) => {
                                                projected.readable = false;
                                                entries.push(projected);
                                                continue;
                                            }
                                            result => result?,
                                        },
                                    };
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
                                        let metadata_size =
                                            keys.open_large_file_size(entry.value, &metadata)?;
                                        let measured_size = store.measured_large_file_size(
                                            host.host_id.as_bytes(),
                                            party.party.as_bytes(),
                                            &entry.value.0,
                                        )?;
                                        if metadata_size.is_some()
                                            && measured_size.is_some()
                                            && metadata_size != measured_size
                                        {
                                            return Err(Error::KvResponse(
                                                "large-file size metadata does not match cached content",
                                            ));
                                        }
                                        projected.large_file_size = metadata_size.or(measured_size);
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
                                    if keys
                                        .open_large_file_size(entry.value, &metadata)?
                                        .is_some_and(|metadata_size| metadata_size != size)
                                    {
                                        return Err(Error::KvResponse(
                                            "large-file size metadata does not match content",
                                        ));
                                    }
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
                    match plan {
                        KvTraversalPlan::Complete => queue.extend(children),
                        KvTraversalPlan::Path(components) => {
                            // Only the component named at this depth is worth
                            // another round trip; a missing or non-directory
                            // component leaves the walk exactly as short as a
                            // complete traversal would leave this path.
                            if let Some(component) = components.get(depth) {
                                if let Some(entry) = entries
                                    .iter()
                                    .find(|entry| entry.name == *component)
                                    .filter(|entry| entry.node_id[0] == KvNodeType::Directory as u8)
                                {
                                    queue.push_back(KvNodeId(entry.node_id).object_id());
                                }
                            }
                            depth += 1;
                        }
                    }
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
                    projections.push(projection);
                }
                // Keep every authenticated dirent in version/name checks, including
                // boundaries whose node bodies this actor cannot read.
                for projection in &mut projections {
                    for entry in &mut projection.entries {
                        if entry.node_id[0] == KvNodeType::Directory as u8
                            && inaccessible_directories
                                .contains(&KvNodeId(entry.node_id).object_id())
                        {
                            entry.readable = false;
                        }
                    }
                    combined.insert(projection.directory_id, projection.clone());
                }
                // A path walk cites only the directories it used, which is
                // exactly what its precondition may assert: the server checks
                // every cited directory, and an uncited one is not asserted.
                let combined = match plan {
                    KvTraversalPlan::Complete => reachable_kv_tree(root.root, combined)?,
                    KvTraversalPlan::Path(_) => combined,
                };
                let final_versions = kv_version_vector(root.version, &combined);
                fetch(auth, &KvRequest::CacheCheck(final_versions))?;
                if let KvTraversalPlan::Path(_) = plan {
                    // The rest of the namespace was not observed, so the
                    // projection keeps its rollback anchors without claiming
                    // that the persisted tree is still complete.
                    store.project_path_metadata(&projections)?;
                    return Ok(projections);
                }
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

/// The version vector a resolved path cites: the root version, and every
/// directory the walk used with every dirent it listed there.
///
/// This is what makes a remembered path checkable. A path and a dirent
/// version alone are not: a dirent version restarts at 1 after an unlink and
/// a re-create on a fresh dirent identifier, so the same pair can name
/// different nodes at different times. The vector names the dirent
/// identifiers themselves, and the server reports a cited dirent whose head
/// has moved — which an unlink does, because the tombstone is a write to that
/// dirent — as stale.
pub fn kv_path_version_vector(tree: &[KvDirectoryProjection]) -> KvPathVersionVector {
    kv_version_vector_from_tree(tree)
}

/// Submits a version vector as its own request and reports whether the server
/// still considers it current. A stale answer, or a cited object the server
/// no longer holds, is `false`; anything else is the caller's error.
fn kv_precondition_holds(
    connection: &mut super::rpc::KvConnection,
    auth: KvAuth<'_>,
    precondition: &KvPathVersionVector,
) -> Result<bool> {
    match connection.call(auth, &KvRequest::CacheCheck(precondition.clone())) {
        Ok(_) => Ok(true),
        Err(error) if is_kv_stale_cache(&error) => Ok(false),
        // The server answers a vector citing an object it no longer has with
        // not-found. That is the same statement as stale for this purpose:
        // the remembered path is not evidence about the node any more.
        Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1049, .. })) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Returns cached node ciphertext only when it decodes and the caller has the
/// required private key. Missing keys and invalid cached bytes fall back to a
/// server fetch so authorization and cache corruption are handled by the normal
/// request path.
fn cached_kv_node(
    cached: &BTreeMap<[u8; 17], Vec<u8>>,
    node: KvNodeId,
    private_keys: &[KvPrivateKeyRef<'_>],
) -> Option<Vec<u8>> {
    let bytes = cached.get(&node.0)?;
    // Fall back to the server when cached bytes cannot be decoded.
    let key = match KvNode::decode(bytes).ok()? {
        KvNode::SmallFile(boxed) | KvNode::Symlink(boxed) => boxed.key,
        KvNode::File(metadata) => metadata.key,
        KvNode::Directory(_) => return None,
    };
    kv_key(private_keys, key.role, key.generation).ok()?;
    Some(bytes.clone())
}

// Only explicit authorization refusal is an inaccessible child. Integrity,
// decoding, missing-key, transport, and not-found errors still fail closed.
fn is_kv_permission_denied(error: &Error) -> bool {
    matches!(
        error,
        Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1013, .. })
    )
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
            Ok(KvFetchedNode::Directory {
                read_role: directory.active.key.role,
            })
        }
        (KvNodeType::File, KvNode::File(metadata)) => {
            let keys = kv_key(private_keys, metadata.key.role, metadata.key.generation)?;
            // Opening the file seed under the node ID is what proves this
            // caller holds the node's key and that the metadata is bound to
            // the ID that was asked for. The chunk chain is authenticated as
            // it is streamed; walking it here would fetch and decrypt the
            // entire file to report a length the caller is about to read
            // anyway, so the node read stops at its metadata.
            let _ = keys.open_file_seed(node, &metadata)?;
            let size = keys.open_large_file_size(node, &metadata)?;
            Ok(KvFetchedNode::LargeFile { size })
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

    // Go returns the stored chunk containing the requested offset, which can
    // start before it. Authenticate that offset and then slice the requested
    // range; exact-boundary-only hosts can fall back to the bounded scan.
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
                if chunk.offset > requested_offset {
                    return Err(Error::KvResponse("chunk starts after the requested range"));
                }
                let clear = Zeroizing::new(open_kv_chunk(&file_seed, node, chunk.offset, &chunk)?);
                if clear.is_empty() && !chunk.final_chunk {
                    return Err(Error::KvResponse("empty non-final file chunk"));
                }
                let chunk_end = validate_large_file_append(chunk.offset, clear.len())?;
                if chunk_end < requested_offset
                    || (chunk_end == requested_offset && !chunk.final_chunk)
                {
                    return Err(Error::KvResponse(
                        "chunk does not contain the requested range",
                    ));
                }
                let start = usize::try_from(requested_offset - chunk.offset)
                    .map_err(|_| Error::KvResponse("KV range overflow"))?;
                let end = start.saturating_add(length).min(clear.len());
                return Ok(KvFetchedChunk {
                    content: clear[start..end].to_vec(),
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

/// Which directories one traversal visits. Both plans apply the same checks
/// to every directory they use; they differ only in what they descend into
/// and therefore in what their closing cache check may cite.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KvTraversalPlan<'a> {
    /// Every directory reachable from the root.
    Complete,
    /// The root, then one directory per named component, in order.
    Path(&'a [Vec<u8>]),
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

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn only_explicit_permission_denials_are_inaccessible_children() {
        for (status, denied) in [
            (
                foks_rpc::RpcStatus::PermissionDenied("restricted".to_owned()),
                true,
            ),
            (foks_rpc::RpcStatus::KvNoEnt, false),
            (
                foks_rpc::RpcStatus::KvPermission {
                    operation: 1,
                    resource: 1,
                },
                false,
            ),
        ] {
            let frame = foks_rpc::encode_status_response_at(&status, 1).unwrap();
            let error = Error::Rpc(
                foks_rpc::read_response(
                    &mut std::io::Cursor::new(frame),
                    foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
                    1,
                )
                .unwrap_err(),
            );
            assert_eq!(is_kv_permission_denied(&error), denied);
        }
        assert!(!is_kv_permission_denied(&Error::KvResponse(
            "missing or duplicate PUK/PTK generation"
        )));
        assert!(!is_kv_permission_denied(&Error::KvResponse(
            "malformed node"
        )));
    }
}

#[cfg(test)]
mod cached_node_tests {
    //! Verifies that repeated metadata synchronization reuses projected large-file
    //! and symlink ciphertext while preserving decoding and authentication checks.

    use foks_client_db::SoftStateStore;
    use foks_crypto::{derive_kv_keys, seal_kv_dirent_name};
    use foks_proto::{
        EntityId, KvDirectory, KvDirent, KvLargeFileMetadata, KvSmallFileBox, Role,
        RoleAndGeneration, ENTITY_USER,
    };

    use super::super::support::bound_dirent;
    use super::*;
    use crate::{verify_public_host, HardStateStore};

    const PROBE: &[u8] = include_bytes!(
        "../../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );

    /// Test fixture containing one directory of large files and symlinks sealed
    /// with the same key.
    struct DroppedFiles {
        party: KvParty,
        seed: SecretSeed,
        key: RoleAndGeneration,
        root: KvRoot,
        root_directory: [u8; 16],
        directory: KvDirectoryPair,
        directory_seed: SecretSeed,
        entries: Vec<KvDirent>,
        nodes: BTreeMap<[u8; 17], Vec<u8>>,
        next_id: u8,
    }

    impl DroppedFiles {
        fn new(party: KvParty) -> Self {
            let seed = SecretSeed::new([0x31; 32]);
            let key = RoleAndGeneration {
                role: Role::OWNER,
                generation: 1,
            };
            let root_directory = [1; 16];
            let keys = derive_kv_keys(&seed).unwrap();
            let binding_mac = keys.bind_root(&party, root_directory, 1, key).unwrap();
            let directory_seed = SecretSeed::new([0x41; 32]);
            let directory = KvDirectory {
                id: root_directory,
                version: 1,
                key,
                seed_ciphertext: keys
                    .seal_directory_seed(root_directory, &directory_seed)
                    .unwrap(),
                write_role: Role::OWNER,
                status: KvDirectoryStatus::Active,
            };
            Self {
                party,
                seed,
                key,
                root: KvRoot::new(root_directory, 1, key, binding_mac).unwrap(),
                root_directory,
                directory: KvDirectoryPair::from_active(directory).unwrap(),
                directory_seed,
                entries: Vec::new(),
                nodes: BTreeMap::new(),
                next_id: 2,
            }
        }

        fn private_keys(&self) -> Vec<KvPrivateKeyRef<'_>> {
            vec![KvPrivateKeyRef {
                role: self.key.role,
                generation: self.key.generation,
                seed: &self.seed,
            }]
        }

        fn fresh_id(&mut self) -> [u8; 16] {
            let id = [self.next_id; 16];
            self.next_id = self
                .next_id
                .checked_add(1)
                .expect("the synthetic store stays small");
            id
        }

        fn link(&mut self, name: &str, node: KvNodeId) {
            let dirent_id = self.fresh_id();
            let (name_mac, name_box) = seal_kv_dirent_name(
                &self.directory_seed,
                self.root_directory,
                1,
                name.as_bytes().to_vec(),
                [dirent_id[0]; 16],
            )
            .unwrap();
            self.entries.push(
                bound_dirent(
                    &self.directory_seed,
                    self.root_directory,
                    dirent_id,
                    node,
                    1,
                    1,
                    Role::OWNER,
                    name_mac,
                    name_box,
                    KvDirectoryStatus::Active,
                    7,
                )
                .unwrap(),
            );
        }

        fn node_id(&mut self, node_type: KvNodeType) -> KvNodeId {
            let object = self.fresh_id();
            let mut node = [0; 17];
            node[0] = node_type as u8;
            node[1..].copy_from_slice(&object);
            KvNodeId(node)
        }

        fn large_file_with_size(&mut self, name: &str, size: Option<u64>) -> KvNodeId {
            let node = self.node_id(KvNodeType::File);
            let file_seed = SecretSeed::new([node.0[1]; 32]);
            let keys = derive_kv_keys(&self.seed).unwrap();
            let mut metadata = keys
                .seal_file_seed(node, self.key, 1, &file_seed, [node.0[1]; 16])
                .unwrap();
            if let Some(size) = size {
                metadata.custom_metadata = Some(
                    keys.seal_large_file_size(node, metadata.version, size, [node.0[1]; 16])
                        .unwrap(),
                );
            }
            self.nodes
                .insert(node.0, KvNode::File(metadata).encoded().unwrap());
            self.link(name, node);
            node
        }

        fn symlink(&mut self, name: &str, target: &str) -> KvNodeId {
            let node = self.node_id(KvNodeType::Symlink);
            let boxed = derive_kv_keys(&self.seed)
                .unwrap()
                .seal_small_file(
                    node,
                    self.key,
                    KvSmallFilePlaintext::Symlink(target.as_bytes().to_vec()),
                )
                .unwrap();
            self.nodes
                .insert(node.0, KvNode::Symlink(boxed).encoded().unwrap());
            self.link(name, node);
            node
        }

        fn serve(&self, fetched: &mut Vec<[u8; 17]>, request: &KvRequest) -> Result<Vec<u8>> {
            match request {
                KvRequest::Root => Ok(self.root.encoded().to_vec()),
                KvRequest::Directory(id) if *id == self.root_directory => {
                    Ok(self.directory.encoded().to_vec())
                }
                KvRequest::List { .. } => {
                    Ok(KvListResponse::new(self.entries.clone(), true, Vec::new())
                        .unwrap()
                        .encoded()
                        .to_vec())
                }
                KvRequest::Node(node) => {
                    fetched.push(node.0);
                    self.nodes
                        .get(&node.0)
                        .cloned()
                        .ok_or(Error::KvResponse("unknown node"))
                }
                KvRequest::CacheCheck(_) => Ok(Vec::new()),
                _ => Err(Error::KvResponse("unexpected request")),
            }
        }
    }

    fn pinned(directory: &tempfile::TempDir) -> (FoksClient, PinnedHost) {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let hard_path = directory.path().join("hard.sqlite3");
        HardStateStore::open(&hard_path)
            .unwrap()
            .accept_verified_host(&public.snapshot)
            .unwrap();
        let client = FoksClient::webpki();
        let host = client.pinned_host("foks.app", &hard_path).unwrap();
        (client, host)
    }

    fn party_for(host: &PinnedHost) -> KvParty {
        let mut party = vec![0x30; 33];
        party[0] = ENTITY_USER;
        KvParty {
            party: EntityId::from_bytes(party).unwrap(),
            host: host.host_id.clone(),
        }
    }

    /// Eight dropped files and one symlink, which is what a store of large
    /// files looks like to the catalog.
    fn dropped_store(host: &PinnedHost) -> DroppedFiles {
        let mut store = DroppedFiles::new(party_for(host));
        for index in 0..8 {
            store.large_file_with_size(&format!("drop-{index:02}"), Some(index));
        }
        store.symlink("latest", "/drop-00");
        store
    }

    fn metadata_pass(
        client: &FoksClient,
        host: &PinnedHost,
        store: &DroppedFiles,
        soft: &Path,
    ) -> (Result<Vec<KvDirectoryProjection>>, Vec<[u8; 17]>) {
        let mut fetched = Vec::new();
        let outcome = client.list_kv_metadata_with_fetch(
            host,
            store.party.clone(),
            KvAuth::User,
            &store.private_keys(),
            soft,
            |_, request| store.serve(&mut fetched, request),
        );
        (outcome, fetched)
    }

    #[test]
    fn a_second_metadata_pass_over_dropped_files_fetches_no_nodes() {
        let directory = tempfile::tempdir().unwrap();
        let (client, host) = pinned(&directory);
        let soft = directory.path().join("soft.sqlite3");
        let store = dropped_store(&host);

        let (first, fetched) = metadata_pass(&client, &host, &store, &soft);
        let projections = first.unwrap();
        assert_eq!(projections.len(), 1);
        assert_eq!(projections[0].entries.len(), 9);
        assert_eq!(
            projections[0]
                .entries
                .iter()
                .filter_map(|entry| entry.large_file_size)
                .collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        // The initial pass fetches one node for each large file and symlink.
        assert_eq!(fetched.len(), 9);

        let (second, fetched) = metadata_pass(&client, &host, &store, &soft);
        let projections = second.unwrap();
        assert_eq!(projections[0].entries.len(), 9);
        assert!(fetched.is_empty(), "{fetched:?}");
        // The second pass preserves the complete projected ciphertext for every node.
        assert!(projections[0]
            .entries
            .iter()
            .all(|entry| entry.node_bytes.is_some()));
    }

    #[test]
    fn legacy_metadata_uses_a_node_bound_local_measurement_without_chunks() {
        let directory = tempfile::tempdir().unwrap();
        let (client, host) = pinned(&directory);
        let soft = directory.path().join("soft.sqlite3");
        let mut remote = DroppedFiles::new(party_for(&host));
        let node = remote.large_file_with_size("legacy.bin", None);
        let mut cache = SoftStateStore::open(&soft).unwrap();
        cache
            .record_large_file_size(
                host.host_id.as_bytes(),
                remote.party.party.as_bytes(),
                &node.0,
                42,
            )
            .unwrap();
        drop(cache);

        let (result, fetched) = metadata_pass(&client, &host, &remote, &soft);
        let projections = result.unwrap();
        assert_eq!(fetched, vec![node.0]);
        assert_eq!(projections[0].entries[0].large_file_size, Some(42));
    }

    /// Flips one byte of the sealed payload of every cached node, leaving the
    /// encoding and the key header intact so the traversal reaches the same
    /// authentication the fetched path performs.
    fn tamper_cached_nodes(soft: &Path, host: &PinnedHost, party: &KvParty) {
        let mut store = SoftStateStore::open(soft).unwrap();
        let mut tree = store
            .tree(host.host_id.as_bytes(), party.party.as_bytes())
            .unwrap();
        let mut tampered = 0;
        for projection in &mut tree {
            for entry in &mut projection.entries {
                let Some(bytes) = entry.node_bytes.as_ref() else {
                    continue;
                };
                let replacement = match KvNode::decode(bytes).unwrap() {
                    KvNode::Symlink(mut boxed) => {
                        boxed.ciphertext[0] ^= 0x01;
                        KvNode::Symlink(KvSmallFileBox {
                            key: boxed.key,
                            ciphertext: boxed.ciphertext,
                        })
                    }
                    KvNode::File(metadata) => {
                        let mut seed = metadata.key_seed;
                        seed.ciphertext[0] ^= 0x01;
                        KvNode::File(KvLargeFileMetadata {
                            key: metadata.key,
                            key_seed: seed,
                            version: metadata.version,
                            custom_metadata: metadata.custom_metadata,
                        })
                    }
                    _ => continue,
                };
                entry.node_bytes = Some(replacement.encoded().unwrap());
                tampered += 1;
            }
        }
        assert_eq!(tampered, 9);
        store.project_reachable_metadata(&tree).unwrap();
    }

    #[test]
    fn a_tampered_cached_node_still_fails_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let (client, host) = pinned(&directory);
        let soft = directory.path().join("soft.sqlite3");
        let store = dropped_store(&host);

        metadata_pass(&client, &host, &store, &soft).0.unwrap();
        tamper_cached_nodes(&soft, &host, &store.party);

        let (outcome, fetched) = metadata_pass(&client, &host, &store, &soft);
        // No server nodes are fetched; authentication fails on the tampered cached
        // ciphertext.
        assert!(fetched.is_empty(), "{fetched:?}");
        assert!(
            matches!(outcome, Err(Error::Crypto(_))),
            "{:?}",
            outcome.map(|projections| projections.len())
        );
    }
}
