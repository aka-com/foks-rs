use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};

use foks_proto::{
    BeaconHint, EntityId, KvDirectoryVersion, KvDirentVersion, KvPathVersionVector, RealtimeWire,
    RtAppId, RtChannelId, RtChannelMetadata, RtInboxChannel, ENTITY_HOST, ENTITY_USER,
};

use crate::soft_schema::{APPLICATION_ID, CHAT_SCHEMA, KNOWN_STORES_SCHEMA, KV_SCHEMA, VERSION};
use crate::{sqlite_integer, stored_unsigned, Acceptance, ChatLimits, Error, Result};

pub const MAX_DISCOVERY_HINTS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatInboxScope {
    pub host: Vec<u8>,
    pub uid: Vec<u8>,
    pub app: RtAppId,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChatInboxState {
    pub cursor: u64,
    pub head: u64,
    pub degraded: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatInboxEntry {
    pub metadata: RtChannelMetadata,
    pub inbox_version: u64,
    pub read_through: u64,
    pub pending_read: Option<u64>,
    pub hidden: bool,
    pub muted: bool,
}

fn validate_chat_inbox_scope(scope: &ChatInboxScope) -> Result<()> {
    if scope.host.len() != 33
        || scope.host.first() != Some(&ENTITY_HOST)
        || scope.uid.len() != 33
        || scope.uid.first() != Some(&ENTITY_USER)
        || scope.app != RtAppId::Chat
    {
        return Err(Error::InvalidChatInbox("invalid account scope"));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnownTeamStore {
    pub account_alias: String,
    pub team_alias: String,
    pub team_id_hex: String,
    pub team_kind: String,
    pub display_name: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KnownStore {
    Account {
        account_alias: String,
        last_seen_at: u64,
    },
    Team {
        store: KnownTeamStore,
        last_seen_at: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvProjectedEntry {
    /// False only for an explicit child permission denial. Retain its authenticated
    /// dirent for namespace and version checks, without caching any node payload.
    pub readable: bool,
    pub dirent_id: [u8; 16],
    pub node_id: [u8; 17],
    pub version: u64,
    pub directory_version: u64,
    pub name: Vec<u8>,
    pub write_role_type: u64,
    pub write_role_visibility: i64,
    pub creation_time: u64,
    pub dirent_bytes: Vec<u8>,
    pub node_bytes: Option<Vec<u8>>,
    pub content: Option<Vec<u8>>,
    pub symlink: Option<Vec<u8>>,
    pub large_file_size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvDirectoryProjection {
    pub host_id: Vec<u8>,
    pub party_id: Vec<u8>,
    pub root_version: u64,
    pub root_directory_id: [u8; 16],
    pub root_bytes: Vec<u8>,
    pub directory_id: [u8; 16],
    pub directory_version: u64,
    pub directory_bytes: Vec<u8>,
    pub entries: Vec<KvProjectedEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvLargeFileStage {
    id: i64,
    pub node_id: [u8; 17],
    pub size: u64,
}

pub struct SoftStateStore {
    connection: Connection,
    owned_stages: std::collections::BTreeSet<i64>,
}

impl SoftStateStore {
    /// Validates an exclusively reserved existing snapshot without rebuilding caches.
    pub fn inspect_existing(path: &Path) -> Result<Self> {
        let connection = crate::inspection::open_existing(path, APPLICATION_ID, VERSION, |c| {
            c.execute_batch(KV_SCHEMA)?;
            c.execute_batch(KNOWN_STORES_SCHEMA)?;
            c.execute_batch(CHAT_SCHEMA)?;
            Ok(())
        })?;
        Ok(Self {
            connection,
            owned_stages: std::collections::BTreeSet::new(),
        })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(Error::SymlinkDatabase);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(path)?.permissions().mode();
            if mode & 0o077 != 0 {
                return Err(Error::InsecureSoftPermissions(mode & 0o777));
            }
        }
        let database_path = path.canonicalize()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = Connection::open_with_flags(&database_path, flags)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "trusted_schema", false)?;
        connection.pragma_update(None, "secure_delete", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        initialize(&mut connection, &database_path)?;
        Ok(Self {
            connection,
            owned_stages: std::collections::BTreeSet::new(),
        })
    }

    pub fn chat_inbox_state(&self, scope: &ChatInboxScope) -> Result<ChatInboxState> {
        validate_chat_inbox_scope(scope)?;
        self.connection
            .query_row(
                "SELECT cursor,head,degraded FROM chat_inbox_state
                 WHERE host_id=?1 AND uid=?2 AND app_id=?3",
                params![scope.host, scope.uid, scope.app as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
            .map(|(cursor, head, degraded)| {
                Ok(ChatInboxState {
                    cursor: stored_unsigned("chat inbox cursor", cursor)?,
                    head: stored_unsigned("chat inbox head", head)?,
                    degraded: degraded != 0,
                })
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn apply_chat_inbox_page(
        &mut self,
        scope: &ChatInboxScope,
        head: u64,
        channels: &[RtInboxChannel],
    ) -> Result<ChatInboxState> {
        validate_chat_inbox_scope(scope)?;
        let head = sqlite_integer("chat inbox head", head)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO chat_inbox_state(host_id,uid,app_id,cursor,head,degraded)
             VALUES (?1,?2,?3,0,0,0) ON CONFLICT DO NOTHING",
            params![scope.host, scope.uid, scope.app as i64],
        )?;
        let (cursor, old_head): (i64, i64) = transaction.query_row(
            "SELECT cursor,head FROM chat_inbox_state
             WHERE host_id=?1 AND uid=?2 AND app_id=?3",
            params![scope.host, scope.uid, scope.app as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if head < cursor || head < old_head {
            return Err(Error::InvalidChatInbox("inbox head rolled back"));
        }
        let mut page_cursor = cursor;
        let mut ids = std::collections::BTreeSet::new();
        let mut versions = std::collections::BTreeSet::new();
        for channel in channels {
            channel
                .validate()
                .map_err(|_| Error::InvalidChatInbox("malformed inbox channel"))?;
            let version = sqlite_integer("chat inbox row version", channel.inbox_version)?;
            let read_through = sqlite_integer("chat read pointer", channel.read_through)?;
            if channel.metadata.app != scope.app
                || channel.metadata.id.0 == [0; 16]
                || channel.metadata.team.entity().as_bytes().len() != 33
                || version <= cursor
                || version > head
                || !ids.insert(channel.metadata.id)
                || !versions.insert(version)
                || channel
                    .metadata
                    .last_message
                    .as_ref()
                    .map_or(channel.read_through != 0, |last| {
                        channel.read_through > last.sequence
                    })
            {
                return Err(Error::InvalidChatInbox(
                    "invalid inbox page ordering or identity",
                ));
            }
            let existing: Option<Vec<u8>> = transaction
                .query_row(
                    "SELECT channel_id FROM chat_inbox_channels
                     WHERE host_id=?1 AND uid=?2 AND app_id=?3 AND inbox_version=?4",
                    params![scope.host, scope.uid, scope.app as i64, version],
                    |row| row.get(0),
                )
                .optional()?;
            if existing
                .as_deref()
                .is_some_and(|id| id != channel.metadata.id.0.as_slice())
            {
                return Err(Error::InvalidChatInbox("inbox version reused"));
            }
            let metadata = channel
                .metadata
                .encoded()
                .map_err(|_| Error::InvalidChatInbox("invalid channel metadata"))?;
            transaction.execute(
                "INSERT INTO chat_inbox_channels(
                     host_id,uid,app_id,channel_id,team_id,inbox_version,
                     read_through,pending_read,hidden,muted,metadata
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,?8,?9,?10)
                 ON CONFLICT(host_id,uid,app_id,channel_id) DO UPDATE SET
                     team_id=excluded.team_id,
                     inbox_version=excluded.inbox_version,
                     read_through=excluded.read_through,
                     pending_read=CASE
                         WHEN chat_inbox_channels.pending_read<=excluded.read_through THEN NULL
                         ELSE chat_inbox_channels.pending_read
                     END,
                     hidden=excluded.hidden,
                     muted=excluded.muted,
                     metadata=excluded.metadata
                 WHERE excluded.inbox_version>chat_inbox_channels.inbox_version",
                params![
                    scope.host,
                    scope.uid,
                    scope.app as i64,
                    channel.metadata.id.0.as_slice(),
                    channel.metadata.team.entity().as_bytes(),
                    version,
                    read_through,
                    i64::from(channel.hidden),
                    i64::from(channel.muted),
                    metadata
                ],
            )?;
            page_cursor = page_cursor.max(version);
        }
        if !channels.is_empty() && page_cursor == cursor {
            return Err(Error::InvalidChatInbox("inbox page made no progress"));
        }
        transaction.execute(
            "UPDATE chat_inbox_state SET cursor=?4,head=?5,degraded=0
             WHERE host_id=?1 AND uid=?2 AND app_id=?3",
            params![scope.host, scope.uid, scope.app as i64, page_cursor, head],
        )?;
        transaction.commit()?;
        Ok(ChatInboxState {
            cursor: stored_unsigned("chat inbox cursor", page_cursor)?,
            head: stored_unsigned("chat inbox head", head)?,
            degraded: false,
        })
    }

    pub fn observe_chat_inbox_head(
        &mut self,
        scope: &ChatInboxScope,
        head: u64,
        degraded: bool,
    ) -> Result<ChatInboxState> {
        validate_chat_inbox_scope(scope)?;
        let state = self.chat_inbox_state(scope)?;
        if head < state.cursor || head < state.head {
            return Err(Error::InvalidChatInbox("inbox head rolled back"));
        }
        self.connection.execute(
            "INSERT INTO chat_inbox_state(host_id,uid,app_id,cursor,head,degraded)
             VALUES (?1,?2,?3,0,?4,?5)
             ON CONFLICT(host_id,uid,app_id) DO UPDATE SET
                 head=excluded.head,degraded=excluded.degraded",
            params![
                scope.host,
                scope.uid,
                scope.app as i64,
                sqlite_integer("chat inbox head", head)?,
                i64::from(degraded)
            ],
        )?;
        Ok(ChatInboxState {
            head,
            degraded,
            ..state
        })
    }

    pub fn reset_chat_inbox(&mut self, scope: &ChatInboxScope) -> Result<()> {
        validate_chat_inbox_scope(scope)?;
        self.connection.execute(
            "DELETE FROM chat_inbox_state WHERE host_id=?1 AND uid=?2 AND app_id=?3",
            params![scope.host, scope.uid, scope.app as i64],
        )?;
        Ok(())
    }

    pub fn chat_inbox_entries(
        &self,
        scope: &ChatInboxScope,
        team: &[u8],
    ) -> Result<Vec<ChatInboxEntry>> {
        validate_chat_inbox_scope(scope)?;
        if team.len() != 33 {
            return Err(Error::InvalidChatInbox("invalid chat team"));
        }
        let mut statement = self.connection.prepare(
            "SELECT metadata,inbox_version,read_through,pending_read,hidden,muted
             FROM chat_inbox_channels
             WHERE host_id=?1 AND uid=?2 AND app_id=?3 AND team_id=?4
             ORDER BY inbox_version DESC LIMIT 1001",
        )?;
        let rows = statement
            .query_map(
                params![scope.host, scope.uid, scope.app as i64, team],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > ChatLimits::INBOX_ROWS {
            return Err(Error::ChatLimit("inbox conversation limit"));
        }
        rows.into_iter()
            .map(
                |(metadata, inbox_version, read_through, pending_read, hidden, muted)| {
                    Ok(ChatInboxEntry {
                        metadata: RtChannelMetadata::decode(&metadata)
                            .map_err(|_| Error::InvalidChatInbox("stored channel metadata"))?,
                        inbox_version: stored_unsigned("chat inbox row version", inbox_version)?,
                        read_through: stored_unsigned("chat read pointer", read_through)?,
                        pending_read: pending_read
                            .map(|value| stored_unsigned("pending chat read", value))
                            .transpose()?,
                        hidden: hidden != 0,
                        muted: muted != 0,
                    })
                },
            )
            .collect()
    }

    pub fn retain_chat_inbox_team(
        &mut self,
        scope: &ChatInboxScope,
        team: &[u8],
        channels: &[RtChannelId],
    ) -> Result<()> {
        let retained = channels
            .iter()
            .map(|id| id.0)
            .collect::<std::collections::BTreeSet<_>>();
        for entry in self.chat_inbox_entries(scope, team)? {
            if !retained.contains(&entry.metadata.id.0) {
                self.connection.execute(
                    "DELETE FROM chat_inbox_channels
                     WHERE host_id=?1 AND uid=?2 AND app_id=?3 AND channel_id=?4",
                    params![
                        scope.host,
                        scope.uid,
                        scope.app as i64,
                        entry.metadata.id.0.as_slice()
                    ],
                )?;
            }
        }
        Ok(())
    }

    pub fn stage_chat_read(
        &mut self,
        scope: &ChatInboxScope,
        channel: RtChannelId,
        sequence: u64,
    ) -> Result<()> {
        validate_chat_inbox_scope(scope)?;
        if sequence == 0 {
            return Err(Error::InvalidChatInbox("invalid read pointer"));
        }
        let metadata: Vec<u8> = self
            .connection
            .query_row(
                "SELECT metadata FROM chat_inbox_channels
                 WHERE host_id=?1 AND uid=?2 AND app_id=?3 AND channel_id=?4",
                params![
                    scope.host,
                    scope.uid,
                    scope.app as i64,
                    channel.0.as_slice()
                ],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::ChatNotFound("inbox channel"))?;
        let metadata = RtChannelMetadata::decode(&metadata)
            .map_err(|_| Error::InvalidChatInbox("stored channel metadata"))?;
        if metadata
            .last_message
            .as_ref()
            .is_none_or(|last| sequence > last.sequence)
        {
            return Err(Error::InvalidChatInbox(
                "read pointer exceeds known messages",
            ));
        }
        let count = self.connection.execute(
            "UPDATE chat_inbox_channels SET pending_read=max(coalesce(pending_read,0),?5)
             WHERE host_id=?1 AND uid=?2 AND app_id=?3 AND channel_id=?4",
            params![
                scope.host,
                scope.uid,
                scope.app as i64,
                channel.0.as_slice(),
                sqlite_integer("pending chat read", sequence)?
            ],
        )?;
        if count != 1 {
            return Err(Error::ChatNotFound("inbox channel"));
        }
        Ok(())
    }

    pub fn confirm_chat_read(
        &mut self,
        scope: &ChatInboxScope,
        channel: RtChannelId,
        sequence: u64,
    ) -> Result<()> {
        validate_chat_inbox_scope(scope)?;
        let count = self.connection.execute(
            "UPDATE chat_inbox_channels
             SET read_through=max(read_through,?5),
                 pending_read=CASE WHEN pending_read<=?5 THEN NULL ELSE pending_read END
             WHERE host_id=?1 AND uid=?2 AND app_id=?3 AND channel_id=?4",
            params![
                scope.host,
                scope.uid,
                scope.app as i64,
                channel.0.as_slice(),
                sqlite_integer("confirmed chat read", sequence)?
            ],
        )?;
        if count != 1 {
            return Err(Error::ChatNotFound("inbox channel"));
        }
        Ok(())
    }

    pub fn pending_chat_reads(&self, scope: &ChatInboxScope) -> Result<Vec<(RtChannelId, u64)>> {
        validate_chat_inbox_scope(scope)?;
        let mut statement = self.connection.prepare(
            "SELECT channel_id,pending_read FROM chat_inbox_channels
             WHERE host_id=?1 AND uid=?2 AND app_id=?3 AND pending_read IS NOT NULL
             ORDER BY channel_id LIMIT 1001",
        )?;
        let rows = statement
            .query_map(params![scope.host, scope.uid, scope.app as i64], |row| {
                Ok((row.get::<_, [u8; 16]>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > ChatLimits::INBOX_ROWS {
            return Err(Error::ChatLimit("pending read limit"));
        }
        rows.into_iter()
            .map(|(id, sequence)| {
                Ok((
                    RtChannelId(id),
                    stored_unsigned("pending chat read", sequence)?,
                ))
            })
            .collect()
    }

    pub fn replace_known_accounts(&mut self, aliases: &[String], observed_at: u64) -> Result<()> {
        let observed_at = sqlite_integer("known-store observation time", observed_at)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM known_stores WHERE kind = 1", [])?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO known_stores
                     (kind, account_alias, team_alias, last_seen_at)
                 VALUES (1, ?1, '', ?2)",
            )?;
            for alias in aliases {
                insert.execute(params![alias, observed_at])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn replace_known_teams(
        &mut self,
        teams: &[KnownTeamStore],
        observed_at: u64,
    ) -> Result<()> {
        let observed_at = sqlite_integer("known-store observation time", observed_at)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM known_stores WHERE kind = 2", [])?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO known_stores
                     (kind, account_alias, team_alias, team_id_hex, team_kind,
                      display_name, active, last_seen_at)
                 VALUES (2, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for team in teams {
                insert.execute(params![
                    team.account_alias,
                    team.team_alias,
                    team.team_id_hex,
                    team.team_kind,
                    team.display_name,
                    i64::from(team.active),
                    observed_at,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn known_stores(&self) -> Result<Vec<KnownStore>> {
        let mut statement = self.connection.prepare(
            "SELECT kind, account_alias, team_alias, team_id_hex, team_kind,
                    display_name, active, last_seen_at
             FROM known_stores
             ORDER BY kind, account_alias, team_alias",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    kind,
                    account_alias,
                    team_alias,
                    team_id_hex,
                    team_kind,
                    display_name,
                    active,
                    last_seen_at,
                )| {
                    let last_seen_at =
                        stored_unsigned("known-store observation time", last_seen_at)?;
                    match (kind, team_id_hex, team_kind, active) {
                        (1, None, None, None) if team_alias.is_empty() => Ok(KnownStore::Account {
                            account_alias,
                            last_seen_at,
                        }),
                        (2, Some(team_id_hex), Some(team_kind), Some(active))
                            if !team_alias.is_empty() =>
                        {
                            Ok(KnownStore::Team {
                                store: KnownTeamStore {
                                    account_alias,
                                    team_alias,
                                    team_id_hex,
                                    team_kind,
                                    display_name,
                                    active: active != 0,
                                },
                                last_seen_at,
                            })
                        }
                        _ => Err(Error::InvalidKnownStore),
                    }
                },
            )
            .collect()
    }

    /// Stores an authenticated routing hint and evicts the least-recently-used
    /// hints. This cache is never sufficient to authorize a host: callers must
    /// probe the address and bind the returned hostchain to `host_id` first.
    pub fn store_discovery_hint(&mut self, hint: &BeaconHint, observed_at: u64) -> Result<()> {
        hint.host_id
            .clone()
            .require_type(ENTITY_HOST)
            .map_err(|_| Error::InvalidDiscoveryHint)?;
        let validated = BeaconHint::new(hint.host_id.clone(), hint.address.clone())
            .map_err(|_| Error::InvalidDiscoveryHint)?;
        let observed_at = sqlite_integer("discovery observation time", observed_at)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO federation_discovery_hints
                 (host_id, address, observed_at, last_used_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(host_id) DO UPDATE SET
                 address = excluded.address,
                 observed_at = excluded.observed_at,
                 last_used_at = max(federation_discovery_hints.last_used_at,
                                    excluded.last_used_at)",
            params![validated.host_id.as_bytes(), validated.address, observed_at],
        )?;
        transaction.execute(
            "DELETE FROM federation_discovery_hints
             WHERE host_id NOT IN (
                 SELECT host_id FROM federation_discovery_hints
                 ORDER BY last_used_at DESC, observed_at DESC, host_id DESC
                 LIMIT ?1
             )",
            [i64::try_from(MAX_DISCOVERY_HINTS).expect("discovery cache bound fits SQLite")],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Returns and touches an untrusted routing hint. A hit must still be
    /// followed by the same authenticated direct probe as a fresh Beacon hit.
    pub fn discovery_hint(
        &mut self,
        host_id: &EntityId,
        used_at: u64,
    ) -> Result<Option<BeaconHint>> {
        host_id
            .clone()
            .require_type(ENTITY_HOST)
            .map_err(|_| Error::InvalidDiscoveryHint)?;
        let used_at = sqlite_integer("discovery use time", used_at)?;
        let stored = self
            .connection
            .query_row(
                "SELECT address FROM federation_discovery_hints WHERE host_id = ?1",
                [host_id.as_bytes()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(address) = stored else {
            return Ok(None);
        };
        let hint =
            BeaconHint::new(host_id.clone(), address).map_err(|_| Error::InvalidDiscoveryHint)?;
        self.connection.execute(
            "UPDATE federation_discovery_hints
             SET last_used_at = max(last_used_at, ?2) WHERE host_id = ?1",
            params![host_id.as_bytes(), used_at],
        )?;
        Ok(Some(hint))
    }

    /// Starts a durable, initially invisible large-file download. Chunks are
    /// committed independently so callers never need to retain the complete
    /// plaintext in memory. The stage becomes visible only when a verified
    /// directory projection references it.
    pub fn begin_large_file(
        &mut self,
        host_id: &[u8],
        party_id: &[u8],
        node_id: [u8; 17],
    ) -> Result<KvLargeFileStage> {
        if host_id.len() != 33 || party_id.len() != 33 || node_id[0] != 2 {
            return Err(Error::InvalidKvProjection);
        }
        self.connection.execute(
            "INSERT INTO kv_large_files (host_id, party_id, node_id, size, complete) \
             VALUES (?1, ?2, ?3, 0, 0)",
            params![host_id, party_id, node_id.as_slice()],
        )?;
        let id = self.connection.last_insert_rowid();
        self.owned_stages.insert(id);
        Ok(KvLargeFileStage {
            id,
            node_id,
            size: 0,
        })
    }

    pub fn append_large_file(
        &mut self,
        stage: &mut KvLargeFileStage,
        plaintext: &[u8],
    ) -> Result<()> {
        let stored = self
            .connection
            .query_row(
                "SELECT size, complete FROM kv_large_files WHERE id = ?1",
                [stage.id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((size, complete)) = stored else {
            return Err(Error::InvalidKvProjection);
        };
        let size = stored_unsigned("KV staged file size", size)?;
        if complete != 0 || size != stage.size {
            return Err(Error::InvalidKvProjection);
        }
        if plaintext.is_empty() {
            return Ok(());
        }
        let new_size = size
            .checked_add(
                u64::try_from(plaintext.len()).map_err(|_| Error::IntegerOutOfRange {
                    field: "KV staged chunk size",
                    value: u64::MAX,
                })?,
            )
            .ok_or(Error::IntegerOutOfRange {
                field: "KV staged file size",
                value: u64::MAX,
            })?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO kv_large_file_chunks (file_id, offset, content) VALUES (?1, ?2, ?3)",
            params![
                stage.id,
                sqlite_integer("KV chunk offset", size)?,
                plaintext
            ],
        )?;
        transaction.execute(
            "UPDATE kv_large_files SET size = ?2 WHERE id = ?1 AND complete = 0",
            params![stage.id, sqlite_integer("KV staged file size", new_size)?],
        )?;
        transaction.commit()?;
        stage.size = new_size;
        Ok(())
    }

    pub fn finish_large_file(&mut self, stage: &KvLargeFileStage) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE kv_large_files SET complete = 1 WHERE id = ?1 AND size = ?2 AND complete = 0",
            params![stage.id, sqlite_integer("KV staged file size", stage.size)?],
        )?;
        if changed != 1 {
            return Err(Error::InvalidKvProjection);
        }
        Ok(())
    }

    pub fn discard_large_files(&mut self, stages: &[KvLargeFileStage]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for stage in stages {
            transaction.execute("DELETE FROM kv_large_files WHERE id = ?1", [stage.id])?;
        }
        transaction.commit()?;
        for stage in stages {
            self.owned_stages.remove(&stage.id);
        }
        Ok(())
    }

    /// Reclaims invisible stages left by a process crash. The caller must
    /// ensure no other `SoftStateStore` is actively downloading into this
    /// database while this maintenance operation runs.
    pub fn reclaim_orphaned_large_files(&mut self) -> Result<usize> {
        if !self.owned_stages.is_empty() {
            return Err(Error::KvProjectionConflict(
                "cannot reclaim while this store owns active large-file stages",
            ));
        }
        Ok(self.connection.execute(
            "DELETE FROM kv_large_files WHERE NOT EXISTS (\
             SELECT 1 FROM kv_entries WHERE kv_entries.large_file_id = kv_large_files.id)",
            [],
        )?)
    }

    /// Atomically replaces one verified directory projection. Advancing the
    /// root invalidates every cached directory for that party before the new
    /// projection is installed.
    pub fn project_directory(&mut self, snapshot: &KvDirectoryProjection) -> Result<Acceptance> {
        self.project_tree(std::slice::from_ref(snapshot))
    }

    /// Atomically replaces a complete verified tree projection. Every
    /// directory must describe the same party and root; a failure leaves the
    /// previously cached tree untouched.
    pub fn project_tree(&mut self, snapshots: &[KvDirectoryProjection]) -> Result<Acceptance> {
        self.project_tree_with_large_files(snapshots, &[])
    }

    pub fn project_tree_with_large_files(
        &mut self,
        snapshots: &[KvDirectoryProjection],
        large_files: &[KvLargeFileStage],
    ) -> Result<Acceptance> {
        self.project_tree_impl(snapshots, large_files, false, false)
    }

    /// Replaces verified stale directories and removes only directory/file
    /// rows no longer readable/reachable from the persisted root. Ciphertext-only
    /// directory and dirent rollback anchors survive pruning.
    pub fn project_reachable_tree_with_large_files(
        &mut self,
        snapshots: &[KvDirectoryProjection],
        large_files: &[KvLargeFileStage],
    ) -> Result<Acceptance> {
        self.project_tree_impl(snapshots, large_files, true, false)
    }

    /// Records an authenticated traversal of every readable directory under the same
    /// rollback and fork checks as a content projection, while deliberately
    /// omitting plaintext and large-file stages.
    pub fn project_reachable_metadata(
        &mut self,
        snapshots: &[KvDirectoryProjection],
    ) -> Result<Acceptance> {
        self.project_tree_impl(snapshots, &[], true, true)
    }

    fn project_tree_impl(
        &mut self,
        snapshots: &[KvDirectoryProjection],
        large_files: &[KvLargeFileStage],
        prune: bool,
        metadata_only: bool,
    ) -> Result<Acceptance> {
        let Some(root) = snapshots.first() else {
            return Err(Error::InvalidKvProjection);
        };
        let large_file_count = large_files.len();
        let large_files = large_files
            .iter()
            .map(|stage| (stage.node_id, *stage))
            .collect::<std::collections::BTreeMap<_, _>>();
        if large_files.len() != large_file_count {
            return Err(Error::InvalidKvProjection);
        }
        let mut directory_ids = std::collections::BTreeSet::new();
        let mut used_large_files = std::collections::BTreeSet::new();
        for snapshot in snapshots {
            validate(snapshot, metadata_only)?;
            if snapshot.host_id != root.host_id
                || snapshot.party_id != root.party_id
                || snapshot.root_version != root.root_version
                || snapshot.root_directory_id != root.root_directory_id
                || snapshot.root_bytes != root.root_bytes
                || !directory_ids.insert(snapshot.directory_id)
            {
                return Err(Error::InvalidKvProjection);
            }
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut previous_large_files = std::collections::BTreeSet::new();
        {
            let mut statement = transaction.prepare(
                "SELECT DISTINCT e.large_file_id FROM kv_entries e WHERE e.host_id = ?1 \
                 AND e.party_id = ?2 AND e.large_file_id IS NOT NULL",
            )?;
            previous_large_files.extend(
                statement
                    .query_map(params![root.host_id, root.party_id], |row| {
                        row.get::<_, i64>(0)
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            );
        }
        let stored_root = transaction
            .query_row(
                "SELECT root_version, root_dir_id, root_bytes FROM kv_parties \
                 WHERE host_id = ?1 AND party_id = ?2",
                params![root.host_id, root.party_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;
        let mut acceptance = Acceptance::Unchanged;
        match stored_root {
            None => {
                transaction.execute(
                    "INSERT INTO kv_parties (host_id, party_id, root_version, root_dir_id, root_bytes) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        root.host_id,
                        root.party_id,
                        sqlite_integer("KV root version", root.root_version)?,
                        root.root_directory_id.as_slice(),
                        root.root_bytes,
                    ],
                )?;
                acceptance = Acceptance::Inserted;
            }
            Some((version, root_id, root_bytes)) => {
                let version = stored_unsigned("KV root version", version)?;
                if root.root_version < version {
                    return Err(Error::KvRootRollback {
                        stored: version,
                        received: root.root_version,
                    });
                }
                if root.root_version == version {
                    if root_id != root.root_directory_id || root_bytes != root.root_bytes {
                        return Err(Error::KvProjectionConflict(
                            "root changed at the same version",
                        ));
                    }
                } else {
                    transaction.execute(
                        "DELETE FROM kv_parties WHERE host_id = ?1 AND party_id = ?2",
                        params![root.host_id, root.party_id],
                    )?;
                    transaction.execute(
                        "INSERT INTO kv_parties (host_id, party_id, root_version, root_dir_id, root_bytes) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            root.host_id,
                            root.party_id,
                            sqlite_integer("KV root version", root.root_version)?,
                            root.root_directory_id.as_slice(),
                            root.root_bytes,
                        ],
                    )?;
                    acceptance = Acceptance::Advanced;
                }
            }
        }

        for snapshot in snapshots {
            let anchor = transaction.query_row(
                "SELECT version, dir_bytes FROM kv_directory_history WHERE host_id = ?1 AND party_id = ?2 AND dir_id = ?3",
                params![snapshot.host_id, snapshot.party_id, snapshot.directory_id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
            ).optional()?;
            if let Some((version, bytes)) = anchor {
                let version = stored_unsigned("KV directory version", version)?;
                if snapshot.directory_version < version {
                    return Err(Error::KvDirectoryRollback {
                        stored: version,
                        received: snapshot.directory_version,
                    });
                }
                if snapshot.directory_version == version && snapshot.directory_bytes != bytes {
                    return Err(Error::KvProjectionConflict(
                        "directory changed at the same version",
                    ));
                }
            }
            transaction.execute(
                "INSERT INTO kv_directory_history (host_id, party_id, dir_id, version, dir_bytes) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(host_id, party_id, dir_id) DO UPDATE SET version = excluded.version, dir_bytes = excluded.dir_bytes",
                params![snapshot.host_id, snapshot.party_id, snapshot.directory_id.as_slice(), sqlite_integer("KV directory version", snapshot.directory_version)?, snapshot.directory_bytes],
            )?;
            let stored_directory = load_directory(
                &transaction,
                &snapshot.host_id,
                &snapshot.party_id,
                &snapshot.directory_id,
            )?;
            if let Some(stored) = stored_directory {
                if snapshot.directory_version < stored.directory_version {
                    return Err(Error::KvDirectoryRollback {
                        stored: stored.directory_version,
                        received: snapshot.directory_version,
                    });
                }
                if snapshot.directory_version == stored.directory_version
                    && (stored.host_id != snapshot.host_id
                        || stored.party_id != snapshot.party_id
                        || stored.root_version != snapshot.root_version
                        || stored.root_directory_id != snapshot.root_directory_id
                        || stored.root_bytes != snapshot.root_bytes
                        || stored.directory_id != snapshot.directory_id
                        || stored.directory_bytes != snapshot.directory_bytes)
                {
                    return Err(Error::KvProjectionConflict(
                        "directory changed at the same version",
                    ));
                }
                if stored != *snapshot && acceptance == Acceptance::Unchanged {
                    acceptance = Acceptance::Advanced;
                }
            } else if acceptance == Acceptance::Unchanged {
                acceptance = Acceptance::Advanced;
            }

            transaction.execute(
                "INSERT INTO kv_directories (host_id, party_id, dir_id, version, dir_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(host_id, party_id, dir_id) DO UPDATE SET \
                 version = excluded.version, dir_bytes = excluded.dir_bytes",
                params![
                    snapshot.host_id,
                    snapshot.party_id,
                    snapshot.directory_id.as_slice(),
                    sqlite_integer("KV directory version", snapshot.directory_version)?,
                    snapshot.directory_bytes,
                ],
            )?;
            transaction.execute(
                "DELETE FROM kv_entries WHERE host_id = ?1 AND party_id = ?2 AND parent_dir_id = ?3",
                params![
                    snapshot.host_id,
                    snapshot.party_id,
                    snapshot.directory_id.as_slice()
                ],
            )?;
            for entry in &snapshot.entries {
                let prior: Option<(i64, Vec<u8>, i64)> = transaction
                    .query_row(
                        "SELECT version, dirent_bytes, present FROM kv_entry_history
                         WHERE host_id = ?1 AND party_id = ?2 AND parent_dir_id = ?3
                           AND dirent_id = ?4",
                        params![
                            snapshot.host_id,
                            snapshot.party_id,
                            snapshot.directory_id.as_slice(),
                            entry.dirent_id.as_slice()
                        ],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                if let Some((version, exact, present)) = prior {
                    let version = stored_unsigned("KV historical dirent version", version)?;
                    if entry.version < version
                        || (entry.version == version && exact != entry.dirent_bytes)
                        || (present == 0 && entry.version == version)
                    {
                        return Err(Error::KvProjectionConflict(
                            "dirent rolled back or changed at the same version",
                        ));
                    }
                }
            }
            transaction.execute(
                "UPDATE kv_entry_history SET present = 0
                 WHERE host_id = ?1 AND party_id = ?2 AND parent_dir_id = ?3",
                params![
                    snapshot.host_id,
                    snapshot.party_id,
                    snapshot.directory_id.as_slice()
                ],
            )?;
            for entry in &snapshot.entries {
                transaction.execute(
                    "INSERT INTO kv_entry_history
                     (host_id, party_id, parent_dir_id, dirent_id, version, dirent_bytes, present)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)
                     ON CONFLICT(host_id, party_id, parent_dir_id, dirent_id) DO UPDATE SET
                       version = excluded.version,
                       dirent_bytes = excluded.dirent_bytes,
                       present = 1",
                    params![
                        snapshot.host_id,
                        snapshot.party_id,
                        snapshot.directory_id.as_slice(),
                        entry.dirent_id.as_slice(),
                        sqlite_integer("KV dirent version", entry.version)?,
                        entry.dirent_bytes
                    ],
                )?;
                let large_file_id = if entry.readable && entry.node_id[0] == 2 && !metadata_only {
                    let stage = large_files
                        .get(&entry.node_id)
                        .ok_or(Error::InvalidKvProjection)?;
                    let valid = transaction
                        .query_row(
                            "SELECT 1 FROM kv_large_files WHERE id = ?1 AND host_id = ?2 AND \
                             party_id = ?3 AND node_id = ?4 AND size = ?5 AND complete = 1",
                            params![
                                stage.id,
                                snapshot.host_id,
                                snapshot.party_id,
                                stage.node_id.as_slice(),
                                sqlite_integer("KV staged file size", stage.size)?,
                            ],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some();
                    if !valid || entry.large_file_size != Some(stage.size) {
                        return Err(Error::InvalidKvProjection);
                    }
                    used_large_files.insert(stage.id);
                    Some(stage.id)
                } else {
                    None
                };
                transaction.execute(
                    "INSERT INTO kv_entries (host_id, party_id, parent_dir_id, dirent_id, node_id, \
                     version, dir_version, name, write_role_type, write_role_visibility, creation_time, \
                     dirent_bytes, node_bytes, content, symlink, large_file_id, readable) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                    params![
                        snapshot.host_id,
                        snapshot.party_id,
                        snapshot.directory_id.as_slice(),
                        entry.dirent_id.as_slice(),
                        entry.node_id.as_slice(),
                        sqlite_integer("KV dirent version", entry.version)?,
                        sqlite_integer("KV dirent directory version", entry.directory_version)?,
                        entry.name,
                        sqlite_integer("KV write role", entry.write_role_type)?,
                        entry.write_role_visibility,
                        sqlite_integer("KV creation time", entry.creation_time)?,
                        entry.dirent_bytes,
                        entry.node_bytes,
                        entry.content,
                        entry.symlink,
                        large_file_id,
                        entry.readable,
                    ],
                )?;
            }
        }
        if used_large_files.len() != large_files.len() {
            return Err(Error::InvalidKvProjection);
        }
        if prune {
            prune_unreachable(
                &transaction,
                &root.host_id,
                &root.party_id,
                &root.root_directory_id,
            )?;
            ensure_complete_tree(
                &transaction,
                &root.host_id,
                &root.party_id,
                &root.root_directory_id,
            )?;
        }
        transaction.execute(
            "UPDATE kv_parties SET cache_complete = ?3 WHERE host_id = ?1 AND party_id = ?2",
            params![root.host_id, root.party_id, i64::from(prune)],
        )?;
        for file_id in previous_large_files {
            transaction.execute(
                "DELETE FROM kv_large_files WHERE id = ?1 AND NOT EXISTS (\
                 SELECT 1 FROM kv_entries WHERE kv_entries.large_file_id = kv_large_files.id)",
                [file_id],
            )?;
        }
        transaction.commit()?;
        for file_id in used_large_files {
            self.owned_stages.remove(&file_id);
        }
        Ok(acceptance)
    }

    pub fn directory(
        &self,
        host_id: &[u8],
        party_id: &[u8],
        directory_id: &[u8; 16],
    ) -> Result<Option<KvDirectoryProjection>> {
        load_directory(&self.connection, host_id, party_id, directory_id)
    }

    pub fn tree(&self, host_id: &[u8], party_id: &[u8]) -> Result<Vec<KvDirectoryProjection>> {
        let mut statement = self.connection.prepare(
            "SELECT dir_id FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 ORDER BY dir_id",
        )?;
        let ids = statement
            .query_map(params![host_id, party_id], |row| row.get::<_, Vec<u8>>(0))?
            .map(|row| row?.try_into().map_err(|_| Error::InvalidKvProjection))
            .collect::<Result<Vec<[u8; 16]>>>()?;
        ids.into_iter()
            .map(|id| {
                load_directory(&self.connection, host_id, party_id, &id)?
                    .ok_or(Error::InvalidKvProjection)
            })
            .collect()
    }

    /// Invalidates only directories affected by a locally accepted namespace
    /// mutation and marks the party projection incomplete. The next sync must
    /// re-fetch authenticated server state instead of accepting a cache check
    /// that cannot discover newly created dirents absent from its vector.
    pub fn invalidate_directories(
        &mut self,
        host_id: &[u8],
        party_id: &[u8],
        directory_ids: &[[u8; 16]],
    ) -> Result<()> {
        if host_id.len() != 33 || party_id.len() != 33 || directory_ids.is_empty() {
            return Err(Error::InvalidKvProjection);
        }
        let unique = directory_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for directory_id in unique {
            transaction.execute(
                "DELETE FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 AND dir_id = ?3",
                params![host_id, party_id, directory_id.as_slice()],
            )?;
        }
        let changed = transaction.execute(
            "UPDATE kv_parties SET cache_complete = 0 WHERE host_id = ?1 AND party_id = ?2",
            params![host_id, party_id],
        )?;
        if changed != 1 {
            return Err(Error::InvalidKvProjection);
        }
        transaction.execute(
            "DELETE FROM kv_large_files WHERE host_id = ?1 AND party_id = ?2 AND NOT EXISTS (\
             SELECT 1 FROM kv_entries WHERE kv_entries.large_file_id = kv_large_files.id)",
            params![host_id, party_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Whether this cache has ever accepted a root for the party, including
    /// roots whose directory projection has since been invalidated.
    pub fn has_kv_root(&self, host_id: &[u8], party_id: &[u8]) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM kv_parties WHERE host_id = ?1 AND party_id = ?2)",
            params![host_id, party_id],
            |row| row.get(0),
        )?)
    }

    /// Reconstructs the exact cache precondition from durable projected
    /// versions. Returning `None` means this party has no complete cache.
    pub fn version_vector(
        &self,
        host_id: &[u8],
        party_id: &[u8],
    ) -> Result<Option<KvPathVersionVector>> {
        let root_version = self
            .connection
            .query_row(
                "SELECT root_version FROM kv_parties WHERE host_id = ?1 AND party_id = ?2 \
                 AND cache_complete = 1",
                params![host_id, party_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(root_version) = root_version else {
            return Ok(None);
        };
        let mut directories = Vec::new();
        let mut directory_statement = self.connection.prepare(
            "SELECT dir_id, version FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 \
             ORDER BY dir_id",
        )?;
        let rows = directory_statement.query_map(params![host_id, party_id], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (id, version) = row?;
            let id: [u8; 16] = id.try_into().map_err(|_| Error::InvalidKvProjection)?;
            let mut entry_statement = self.connection.prepare(
                "SELECT dirent_id, version FROM kv_entries WHERE host_id = ?1 AND party_id = ?2 \
                 AND parent_dir_id = ?3 ORDER BY dirent_id",
            )?;
            let entries = entry_statement
                .query_map(params![host_id, party_id, id.as_slice()], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
                })?
                .map(|row| {
                    let (id, version) = row?;
                    Ok(KvDirentVersion {
                        id: id.try_into().map_err(|_| Error::InvalidKvProjection)?,
                        version: stored_unsigned("KV dirent version", version)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            directories.push(KvDirectoryVersion {
                id,
                version: stored_unsigned("KV directory version", version)?,
                entries,
            });
        }
        Ok(Some(KvPathVersionVector {
            root_version: stored_unsigned("KV root version", root_version)?,
            directories,
        }))
    }

    /// Streams a projected large file to `writer`, reading at most one
    /// plaintext chunk from SQLite at a time.
    pub fn write_large_file<W: Write>(
        &self,
        host_id: &[u8],
        party_id: &[u8],
        node_id: &[u8; 17],
        writer: &mut W,
    ) -> Result<Option<u64>> {
        let file = self
            .connection
            .query_row(
                "SELECT f.id, f.size FROM kv_large_files f WHERE f.host_id = ?1 AND f.party_id = ?2 \
                 AND f.node_id = ?3 AND f.complete = 1 AND EXISTS (SELECT 1 FROM kv_entries e \
                 WHERE e.large_file_id = f.id) ORDER BY f.id DESC LIMIT 1",
                params![host_id, party_id, node_id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((file_id, size)) = file else {
            return Ok(None);
        };
        let size = stored_unsigned("KV large file size", size)?;
        let mut offset = 0u64;
        let mut statement = self.connection.prepare(
            "SELECT offset, content FROM kv_large_file_chunks WHERE file_id = ?1 ORDER BY offset",
        )?;
        let mut rows = statement.query([file_id])?;
        while let Some(row) = rows.next()? {
            let stored_offset = stored_unsigned("KV chunk offset", row.get(0)?)?;
            let content: Vec<u8> = row.get(1)?;
            if stored_offset != offset {
                return Err(Error::InvalidKvProjection);
            }
            writer.write_all(&content)?;
            offset = offset
                .checked_add(content.len() as u64)
                .ok_or(Error::InvalidKvProjection)?;
        }
        if offset != size {
            return Err(Error::InvalidKvProjection);
        }
        Ok(Some(size))
    }

    /// Reads one bounded range from a complete projected large file without
    /// materializing the rest of the plaintext. The returned size is the
    /// authenticated projected size used to compute EOF.
    pub fn read_large_file_chunk(
        &self,
        host_id: &[u8],
        party_id: &[u8],
        node_id: &[u8; 17],
        offset: u64,
        length: usize,
    ) -> Result<Option<(Vec<u8>, u64)>> {
        let file = self
            .connection
            .query_row(
                "SELECT f.id, f.size FROM kv_large_files f WHERE f.host_id = ?1 AND f.party_id = ?2 \
                 AND f.node_id = ?3 AND f.complete = 1 AND EXISTS (SELECT 1 FROM kv_entries e \
                 WHERE e.large_file_id = f.id) ORDER BY f.id DESC LIMIT 1",
                params![host_id, party_id, node_id.as_slice()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((file_id, size)) = file else {
            return Ok(None);
        };
        let size = stored_unsigned("KV large file size", size)?;
        if offset > size {
            return Err(Error::InvalidKvProjection);
        }
        let wanted_end = offset
            .checked_add(length as u64)
            .ok_or(Error::InvalidKvProjection)?
            .min(size);
        let mut output = Vec::with_capacity((wanted_end - offset) as usize);
        let mut expected_offset = offset;
        let mut statement = self.connection.prepare(
            "SELECT offset, content FROM kv_large_file_chunks WHERE file_id = ?1 \
             AND offset < ?2 AND offset + length(content) > ?3 ORDER BY offset",
        )?;
        let mut rows = statement.query(params![
            file_id,
            sqlite_integer("KV chunk range end", wanted_end)?,
            sqlite_integer("KV chunk range offset", offset)?,
        ])?;
        let mut first = true;
        while let Some(row) = rows.next()? {
            let chunk_offset = stored_unsigned("KV chunk offset", row.get(0)?)?;
            let content: Vec<u8> = row.get(1)?;
            let chunk_end = chunk_offset
                .checked_add(content.len() as u64)
                .ok_or(Error::InvalidKvProjection)?;
            if (first && (chunk_offset > expected_offset || chunk_end <= expected_offset))
                || (!first && chunk_offset != expected_offset)
            {
                return Err(Error::InvalidKvProjection);
            }
            let start = (expected_offset - chunk_offset) as usize;
            let end_offset = wanted_end.min(chunk_end);
            let end = (end_offset - chunk_offset) as usize;
            output.extend_from_slice(&content[start..end]);
            expected_offset = end_offset;
            first = false;
        }
        if expected_offset != wanted_end || output.len() as u64 != wanted_end - offset {
            return Err(Error::InvalidKvProjection);
        }
        Ok(Some((output, size)))
    }
}

impl Drop for SoftStateStore {
    fn drop(&mut self) {
        for stage in &self.owned_stages {
            let _ = self
                .connection
                .execute("DELETE FROM kv_large_files WHERE id = ?1", [stage]);
        }
    }
}

fn prune_unreachable(
    connection: &Connection,
    host_id: &[u8],
    party_id: &[u8],
    root: &[u8; 16],
) -> Result<()> {
    connection.execute(
        "WITH RECURSIVE reachable(dir_id) AS (VALUES (?3) UNION SELECT substr(e.node_id, 2, 16) \
         FROM kv_entries e JOIN reachable r ON e.parent_dir_id = r.dir_id \
         WHERE e.host_id = ?1 AND e.party_id = ?2 AND e.readable = 1 AND substr(e.node_id, 1, 1) = x'01') \
         DELETE FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 \
         AND dir_id NOT IN (SELECT dir_id FROM reachable)",
        params![host_id, party_id, root.as_slice()],
    )?;
    Ok(())
}

fn ensure_complete_tree(
    connection: &Connection,
    host_id: &[u8],
    party_id: &[u8],
    root: &[u8; 16],
) -> Result<()> {
    let missing: i64 = connection.query_row(
        "WITH RECURSIVE reachable(dir_id) AS (VALUES (?3) UNION SELECT substr(e.node_id, 2, 16) \
         FROM kv_entries e JOIN reachable r ON e.parent_dir_id = r.dir_id \
         WHERE e.host_id = ?1 AND e.party_id = ?2 AND e.readable = 1 AND substr(e.node_id, 1, 1) = x'01') \
         SELECT count(*) FROM reachable r LEFT JOIN kv_directories d ON d.host_id = ?1 \
         AND d.party_id = ?2 AND d.dir_id = r.dir_id WHERE d.dir_id IS NULL",
        params![host_id, party_id, root.as_slice()],
        |row| row.get(0),
    )?;
    if missing != 0 {
        return Err(Error::InvalidKvProjection);
    }
    Ok(())
}

fn initialize(connection: &mut Connection, path: &Path) -> Result<()> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id == 0 && version == 0 {
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if count != 0 {
            return Err(Error::WrongSoftApplicationId {
                found: application_id,
                expected: APPLICATION_ID,
            });
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(KV_SCHEMA)?;
        transaction.execute_batch(KNOWN_STORES_SCHEMA)?;
        transaction.execute_batch(CHAT_SCHEMA)?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", VERSION)?;
        transaction.commit()?;
        return Ok(());
    }
    if application_id != APPLICATION_ID {
        return Err(Error::WrongSoftApplicationId {
            found: application_id,
            expected: APPLICATION_ID,
        });
    }
    if version != VERSION {
        return Err(Error::UnsupportedSoftSchema {
            path: path.display().to_string(),
            found: version,
            supported: VERSION,
        });
    }
    Ok(())
}

fn validate(snapshot: &KvDirectoryProjection, metadata_only: bool) -> Result<()> {
    if snapshot.host_id.len() != 33
        || snapshot.party_id.len() != 33
        || snapshot.root_version == 0
        || snapshot.directory_version == 0
        || snapshot.root_bytes.is_empty()
        || snapshot.directory_bytes.is_empty()
        || snapshot.entries.iter().any(|entry| {
            entry.version == 0
                || entry.directory_version == 0
                || entry.name.is_empty()
                || entry.dirent_bytes.is_empty()
                || !matches!(
                    (entry.write_role_type, entry.write_role_visibility),
                    (1, -32768..=32767) | (2 | 3, 0)
                )
                || if !entry.readable {
                    !matches!(entry.node_id[0], 1..=4)
                        || entry.node_bytes.is_some()
                        || entry.content.is_some()
                        || entry.symlink.is_some()
                        || entry.large_file_size.is_some()
                } else if metadata_only {
                    match entry.node_id[0] {
                        1 => {
                            entry.node_bytes.is_some()
                                || entry.content.is_some()
                                || entry.symlink.is_some()
                                || entry.large_file_size.is_some()
                        }
                        2..=4 => {
                            entry.node_bytes.is_none()
                                || entry.content.is_some()
                                || entry.symlink.is_some()
                                || entry.large_file_size.is_some()
                        }
                        _ => true,
                    }
                } else {
                    match entry.node_id[0] {
                        1 => {
                            entry.content.is_some()
                                || entry.symlink.is_some()
                                || entry.large_file_size.is_some()
                        }
                        2 => {
                            entry.content.is_some()
                                || entry.symlink.is_some()
                                || entry.large_file_size.is_none()
                        }
                        3 => {
                            entry.content.is_none()
                                || entry.symlink.is_some()
                                || entry.large_file_size.is_some()
                        }
                        4 => {
                            entry.content.is_some()
                                || entry.symlink.is_none()
                                || entry.large_file_size.is_some()
                        }
                        _ => true,
                    }
                }
        })
    {
        return Err(Error::InvalidKvProjection);
    }
    let mut ids = snapshot
        .entries
        .iter()
        .map(|entry| entry.dirent_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut names = snapshot
        .entries
        .iter()
        .map(|entry| entry.name.as_slice())
        .collect::<Vec<_>>();
    names.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1])
        || names.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(Error::InvalidKvProjection);
    }
    Ok(())
}

fn load_directory(
    connection: &Connection,
    host_id: &[u8],
    party_id: &[u8],
    directory_id: &[u8; 16],
) -> Result<Option<KvDirectoryProjection>> {
    let root = connection
        .query_row(
            "SELECT root_version, root_dir_id, root_bytes FROM kv_parties WHERE host_id = ?1 AND party_id = ?2",
            params![host_id, party_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
        )
        .optional()?;
    let Some((root_version, root_directory_id, root_bytes)) = root else {
        return Ok(None);
    };
    let directory = connection
        .query_row(
            "SELECT version, dir_bytes FROM kv_directories WHERE host_id = ?1 AND party_id = ?2 AND dir_id = ?3",
            params![host_id, party_id, directory_id.as_slice()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    let Some((directory_version, directory_bytes)) = directory else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "SELECT e.dirent_id, e.node_id, e.version, e.dir_version, e.name, e.write_role_type, \
         e.write_role_visibility, e.creation_time, e.dirent_bytes, e.node_bytes, e.content, \
         e.symlink, f.size, e.readable FROM kv_entries e LEFT JOIN kv_large_files f ON f.id = e.large_file_id \
         WHERE e.host_id = ?1 AND e.party_id = ?2 AND e.parent_dir_id = ?3 \
         ORDER BY name, dirent_id",
    )?;
    let entries = statement
        .query_map(params![host_id, party_id, directory_id.as_slice()], |row| {
            let dirent_id: Vec<u8> = row.get(0)?;
            let node_id: Vec<u8> = row.get(1)?;
            Ok((
                dirent_id,
                node_id,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Vec<u8>>(8)?,
                row.get::<_, Option<Vec<u8>>>(9)?,
                row.get::<_, Option<Vec<u8>>>(10)?,
                row.get::<_, Option<Vec<u8>>>(11)?,
                row.get::<_, Option<i64>>(12)?,
                row.get::<_, bool>(13)?,
            ))
        })?
        .map(|row| {
            let row = row?;
            Ok(KvProjectedEntry {
                readable: row.13,
                dirent_id: row.0.try_into().map_err(|_| Error::InvalidKvProjection)?,
                node_id: row.1.try_into().map_err(|_| Error::InvalidKvProjection)?,
                version: stored_unsigned("KV dirent version", row.2)?,
                directory_version: stored_unsigned("KV dirent directory version", row.3)?,
                name: row.4,
                write_role_type: stored_unsigned("KV write role", row.5)?,
                write_role_visibility: row.6,
                creation_time: stored_unsigned("KV creation time", row.7)?,
                dirent_bytes: row.8,
                node_bytes: row.9,
                content: row.10,
                symlink: row.11,
                large_file_size: row
                    .12
                    .map(|size| stored_unsigned("KV large file size", size))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(KvDirectoryProjection {
        host_id: host_id.to_vec(),
        party_id: party_id.to_vec(),
        root_version: stored_unsigned("KV root version", root_version)?,
        root_directory_id: root_directory_id
            .try_into()
            .map_err(|_| Error::InvalidKvProjection)?,
        root_bytes,
        directory_id: *directory_id,
        directory_version: stored_unsigned("KV directory version", directory_version)?,
        directory_bytes,
        entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_proto::{RtLastMessage, RtMessageType};

    fn host(fill: u8) -> EntityId {
        EntityId::from_bytes([vec![ENTITY_HOST], vec![fill; 32]].concat()).unwrap()
    }

    #[test]
    fn previous_soft_schemas_require_explicit_cache_rebuild() {
        for version in [3, 4, 5] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("soft.sqlite3");
            drop(SoftStateStore::open(&path).unwrap());
            let connection = Connection::open(&path).unwrap();
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            drop(connection);
            assert!(
                matches!(SoftStateStore::open(&path), Err(Error::UnsupportedSoftSchema { found, supported: VERSION, .. }) if found == version)
            );
        }
    }

    #[test]
    fn unsupported_soft_schema_names_the_safe_recovery_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        drop(SoftStateStore::open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        drop(connection);

        let error = match SoftStateStore::open(&path) {
            Ok(_) => panic!("unsupported schema opened"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(message.contains(path.canonicalize().unwrap().to_str().unwrap()));
        assert!(message.contains("unsupported soft-state cache schema version 2"));
        assert!(message.contains("this build supports version 6"));
        assert!(message.contains("cache must be recreated"));
        assert!(path.exists());
    }

    #[test]
    fn chat_inbox_pages_reads_pruning_and_reset_are_scope_bound() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let scope = ChatInboxScope {
            host: [vec![ENTITY_HOST], vec![2; 32]].concat(),
            uid: [vec![ENTITY_USER], vec![3; 32]].concat(),
            app: RtAppId::Chat,
        };
        let mut metadata = RtChannelMetadata::decode(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/realtime/channel.snowp"
        ))
        .unwrap();
        metadata.last_message = Some(RtLastMessage {
            sequence: 1,
            kind: RtMessageType::Basic,
            insert_time: 1,
            sender: None,
            further_user_attribution: None,
        });
        let first = RtInboxChannel {
            metadata: metadata.clone(),
            inbox_version: 1,
            read_through: 0,
            hidden: false,
            muted: false,
        };
        assert_eq!(
            store
                .apply_chat_inbox_page(&scope, 2, std::slice::from_ref(&first))
                .unwrap(),
            ChatInboxState {
                cursor: 1,
                head: 2,
                degraded: false
            }
        );
        let mut valid_metadata = metadata.clone();
        valid_metadata.id = RtChannelId([2; 16]);
        let mut invalid_metadata = metadata.clone();
        invalid_metadata.id = RtChannelId([3; 16]);
        invalid_metadata.app = RtAppId::Crdt;
        assert!(store
            .apply_chat_inbox_page(
                &scope,
                3,
                &[
                    RtInboxChannel {
                        metadata: valid_metadata,
                        inbox_version: 2,
                        read_through: 0,
                        hidden: false,
                        muted: false,
                    },
                    RtInboxChannel {
                        metadata: invalid_metadata,
                        inbox_version: 3,
                        read_through: 0,
                        hidden: false,
                        muted: false,
                    },
                ],
            )
            .is_err());
        assert_eq!(store.chat_inbox_state(&scope).unwrap().cursor, 1);
        assert_eq!(
            store
                .chat_inbox_entries(&scope, metadata.team.entity().as_bytes())
                .unwrap()
                .len(),
            1
        );
        store.stage_chat_read(&scope, metadata.id, 1).unwrap();
        assert_eq!(
            store.pending_chat_reads(&scope).unwrap(),
            vec![(metadata.id, 1)]
        );
        store.confirm_chat_read(&scope, metadata.id, 1).unwrap();
        assert!(store.pending_chat_reads(&scope).unwrap().is_empty());
        let mut second_metadata = metadata.clone();
        second_metadata.id = RtChannelId([2; 16]);
        let second = RtInboxChannel {
            metadata: second_metadata.clone(),
            inbox_version: 2,
            read_through: 0,
            hidden: false,
            muted: false,
        };
        store
            .apply_chat_inbox_page(&scope, 2, std::slice::from_ref(&second))
            .unwrap();
        assert_eq!(
            store
                .chat_inbox_entries(&scope, metadata.team.entity().as_bytes())
                .unwrap()
                .len(),
            2
        );
        store
            .retain_chat_inbox_team(&scope, metadata.team.entity().as_bytes(), &[metadata.id])
            .unwrap();
        assert_eq!(
            store
                .chat_inbox_entries(&scope, metadata.team.entity().as_bytes())
                .unwrap()
                .len(),
            1
        );
        drop(store);
        let mut store = SoftStateStore::open(&path).unwrap();
        assert_eq!(store.chat_inbox_state(&scope).unwrap().cursor, 2);
        assert!(store.observe_chat_inbox_head(&scope, 1, false).is_err());
        store.reset_chat_inbox(&scope).unwrap();
        assert_eq!(
            store.chat_inbox_state(&scope).unwrap(),
            ChatInboxState::default()
        );
        assert!(store
            .chat_inbox_entries(&scope, metadata.team.entity().as_bytes())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn known_store_sources_replace_independently_and_survive_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        store
            .replace_known_accounts(&["personal".to_owned()], 10)
            .unwrap();
        store
            .replace_known_teams(
                &[KnownTeamStore {
                    account_alias: "personal".to_owned(),
                    team_alias: "household".to_owned(),
                    team_id_hex: "03aa".to_owned(),
                    team_kind: "named".to_owned(),
                    display_name: Some("Household".to_owned()),
                    active: true,
                }],
                11,
            )
            .unwrap();
        store
            .replace_known_accounts(&["work".to_owned()], 12)
            .unwrap();
        drop(store);

        let mut store = SoftStateStore::open(&path).unwrap();
        assert_eq!(
            store.known_stores().unwrap(),
            vec![
                KnownStore::Account {
                    account_alias: "work".to_owned(),
                    last_seen_at: 12,
                },
                KnownStore::Team {
                    store: KnownTeamStore {
                        account_alias: "personal".to_owned(),
                        team_alias: "household".to_owned(),
                        team_id_hex: "03aa".to_owned(),
                        team_kind: "named".to_owned(),
                        display_name: Some("Household".to_owned()),
                        active: true,
                    },
                    last_seen_at: 11,
                },
            ],
        );
        store.replace_known_teams(&[], 13).unwrap();
        assert_eq!(
            store.known_stores().unwrap(),
            vec![KnownStore::Account {
                account_alias: "work".to_owned(),
                last_seen_at: 12,
            }],
        );
    }

    #[test]
    fn discovery_hints_are_bounded_and_touched_as_lru_soft_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        for index in 0..u8::try_from(MAX_DISCOVERY_HINTS).unwrap() {
            let hint = BeaconHint::new(host(index), format!("host-{index}.test:4430")).unwrap();
            store
                .store_discovery_hint(&hint, u64::from(index) + 1)
                .unwrap();
        }

        assert!(store.discovery_hint(&host(0), 1_000).unwrap().is_some());
        store
            .store_discovery_hint(
                &BeaconHint::new(host(200), "replacement.test:4430".to_owned()).unwrap(),
                2_000,
            )
            .unwrap();

        assert!(store.discovery_hint(&host(0), 2_001).unwrap().is_some());
        assert!(store.discovery_hint(&host(1), 2_001).unwrap().is_none());
        assert!(store.discovery_hint(&host(200), 2_001).unwrap().is_some());
        let count: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM federation_discovery_hints",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, i64::try_from(MAX_DISCOVERY_HINTS).unwrap());
    }

    fn entry(id: u8, name: &[u8]) -> KvProjectedEntry {
        let mut node_id = [id; 17];
        node_id[0] = 3;
        KvProjectedEntry {
            readable: true,
            dirent_id: [id; 16],
            node_id,
            version: 1,
            directory_version: 1,
            name: name.to_vec(),
            write_role_type: 2,
            write_role_visibility: 0,
            creation_time: u64::from(id),
            dirent_bytes: vec![id],
            node_bytes: Some(vec![id + 1]),
            content: Some(vec![id + 2]),
            symlink: None,
            large_file_size: None,
        }
    }

    fn snapshot() -> KvDirectoryProjection {
        KvDirectoryProjection {
            host_id: vec![2; 33],
            party_id: vec![3; 33],
            root_version: 1,
            root_directory_id: [4; 16],
            root_bytes: vec![5],
            directory_id: [4; 16],
            directory_version: 1,
            directory_bytes: vec![6],
            entries: vec![entry(1, b"a"), entry(2, b"b")],
        }
    }

    #[test]
    fn observed_root_survives_directory_cache_invalidation() {
        let temporary = tempfile::tempdir().unwrap();
        let mut store = SoftStateStore::open(&temporary.path().join("soft.sqlite3")).unwrap();
        let projection = snapshot();
        assert!(!store
            .has_kv_root(&projection.host_id, &projection.party_id)
            .unwrap());
        store.project_directory(&projection).unwrap();
        store
            .invalidate_directories(
                &projection.host_id,
                &projection.party_id,
                &[projection.directory_id],
            )
            .unwrap();
        assert!(store
            .tree(&projection.host_id, &projection.party_id)
            .unwrap()
            .is_empty());
        assert!(store
            .version_vector(&projection.host_id, &projection.party_id)
            .unwrap()
            .is_none());
        assert!(store
            .has_kv_root(&projection.host_id, &projection.party_id)
            .unwrap());
    }

    #[test]
    fn verified_projection_round_trips_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let snapshot = snapshot();
        assert_eq!(
            store.project_directory(&snapshot).unwrap(),
            Acceptance::Inserted
        );
        assert_eq!(
            store.project_directory(&snapshot).unwrap(),
            Acceptance::Unchanged
        );
        assert_eq!(
            store
                .directory(
                    &snapshot.host_id,
                    &snapshot.party_id,
                    &snapshot.directory_id
                )
                .unwrap(),
            Some(snapshot)
        );
    }

    #[test]
    fn rollback_and_same_version_conflicts_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let mut projected = snapshot();
        store.project_directory(&projected).unwrap();
        projected.root_bytes.push(9);
        assert!(matches!(
            store.project_directory(&projected),
            Err(Error::KvProjectionConflict(_))
        ));
        projected = snapshot();
        projected.directory_version = 2;
        for entry in &mut projected.entries {
            entry.directory_version = 2;
        }
        store.project_directory(&projected).unwrap();
        projected.directory_version = 1;
        for entry in &mut projected.entries {
            entry.directory_version = 1;
        }
        assert!(matches!(
            store.project_directory(&projected),
            Err(Error::KvDirectoryRollback { .. })
        ));
    }

    #[test]
    fn same_directory_version_accepts_external_dirent_advances_but_not_resurrection() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let initial = snapshot();
        store.project_directory(&initial).unwrap();

        let mut external = initial.clone();
        external.entries.push(entry(3, b"c"));
        assert_eq!(
            store.project_directory(&external).unwrap(),
            Acceptance::Advanced
        );

        let mut removed = external.clone();
        removed.entries.retain(|entry| entry.dirent_id != [3; 16]);
        assert_eq!(
            store.project_directory(&removed).unwrap(),
            Acceptance::Advanced
        );
        assert!(matches!(
            store.project_directory(&external),
            Err(Error::KvProjectionConflict(_))
        ));

        let mut recreated = external;
        let recreated_entry = recreated.entries.last_mut().unwrap();
        recreated_entry.version = 2;
        recreated_entry.dirent_bytes.push(2);
        assert_eq!(
            store.project_directory(&recreated).unwrap(),
            Acceptance::Advanced
        );
    }

    #[test]
    fn root_advance_invalidates_old_directories_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let old = snapshot();
        store.project_directory(&old).unwrap();
        let mut new = snapshot();
        new.root_version = 2;
        new.root_directory_id = [8; 16];
        new.root_bytes = vec![9];
        new.directory_id = [8; 16];
        new.directory_bytes = vec![10];
        assert_eq!(store.project_directory(&new).unwrap(), Acceptance::Advanced);
        assert!(store
            .directory(&old.host_id, &old.party_id, &old.directory_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .directory(&new.host_id, &new.party_id, &new.directory_id)
                .unwrap(),
            Some(new)
        );
    }

    #[test]
    fn adding_a_directory_under_the_same_root_is_an_advance() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let root = snapshot();
        store.project_directory(&root).unwrap();
        let mut child = snapshot();
        child.directory_id = [12; 16];
        child.directory_bytes = vec![13];
        assert_eq!(
            store.project_directory(&child).unwrap(),
            Acceptance::Advanced
        );
        assert!(store
            .directory(&child.host_id, &child.party_id, &child.directory_id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn tree_projection_rolls_back_every_directory_on_a_late_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let root = snapshot();
        store.project_directory(&root).unwrap();

        let mut child = snapshot();
        child.directory_id = [12; 16];
        child.directory_bytes = vec![13];
        let mut conflicting_root = root.clone();
        conflicting_root.directory_bytes.push(99);
        assert!(matches!(
            store.project_tree(&[child.clone(), conflicting_root]),
            Err(Error::KvProjectionConflict(_))
        ));

        assert!(store
            .directory(&child.host_id, &child.party_id, &child.directory_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .directory(&root.host_id, &root.party_id, &root.directory_id)
                .unwrap(),
            Some(root)
        );
    }

    #[test]
    fn version_vector_and_large_file_chunks_are_durable_and_targeted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let mut projected = snapshot();
        let mut node_id = [9; 17];
        node_id[0] = 2;
        projected.entries = vec![KvProjectedEntry {
            readable: true,
            dirent_id: [7; 16],
            node_id,
            version: 4,
            directory_version: 1,
            name: b"large.bin".to_vec(),
            write_role_type: 2,
            write_role_visibility: 0,
            creation_time: 7,
            dirent_bytes: vec![7],
            node_bytes: Some(vec![8]),
            content: None,
            symlink: None,
            large_file_size: Some(5),
        }];
        let mut stage = store
            .begin_large_file(&projected.host_id, &projected.party_id, node_id)
            .unwrap();
        store.append_large_file(&mut stage, b"ab").unwrap();
        store.append_large_file(&mut stage, b"cde").unwrap();
        store.finish_large_file(&stage).unwrap();
        store
            .project_reachable_tree_with_large_files(&[projected.clone()], &[stage])
            .unwrap();

        let versions = store
            .version_vector(&projected.host_id, &projected.party_id)
            .unwrap()
            .unwrap();
        assert_eq!(versions.root_version, 1);
        assert_eq!(versions.directories[0].id, projected.directory_id);
        assert_eq!(versions.directories[0].entries[0].version, 4);
        drop(store);

        let mut store = SoftStateStore::open(&path).unwrap();
        let mut output = Vec::new();
        assert_eq!(
            store
                .write_large_file(
                    &projected.host_id,
                    &projected.party_id,
                    &node_id,
                    &mut output,
                )
                .unwrap(),
            Some(5)
        );
        assert_eq!(output, b"abcde");
        assert_eq!(
            store
                .read_large_file_chunk(&projected.host_id, &projected.party_id, &node_id, 1, 3,)
                .unwrap(),
            Some((b"bcd".to_vec(), 5))
        );
        assert_eq!(
            store
                .read_large_file_chunk(&projected.host_id, &projected.party_id, &node_id, 0, 2,)
                .unwrap(),
            Some((b"ab".to_vec(), 5))
        );
        assert_eq!(
            store
                .read_large_file_chunk(&projected.host_id, &projected.party_id, &node_id, 4, 8,)
                .unwrap(),
            Some((b"e".to_vec(), 5))
        );
        assert_eq!(
            store
                .read_large_file_chunk(&projected.host_id, &projected.party_id, &node_id, 5, 8,)
                .unwrap(),
            Some((Vec::new(), 5))
        );

        projected.directory_version = 2;
        projected.directory_bytes = vec![10];
        projected.entries.clear();
        store
            .project_reachable_tree_with_large_files(&[projected.clone()], &[])
            .unwrap();
        assert_eq!(
            store
                .write_large_file(
                    &projected.host_id,
                    &projected.party_id,
                    &node_id,
                    &mut Vec::new(),
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn metadata_projection_pins_versions_without_plaintext_or_file_stages() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metadata.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let mut projected = snapshot();
        projected.directory_version = 2;
        projected.directory_bytes = vec![2];
        let mut node_id = [9; 17];
        node_id[0] = 2;
        projected.entries = vec![KvProjectedEntry {
            readable: true,
            dirent_id: [7; 16],
            node_id,
            version: 4,
            directory_version: 2,
            name: b"large.bin".to_vec(),
            write_role_type: 2,
            write_role_visibility: 0,
            creation_time: 7,
            dirent_bytes: vec![7],
            node_bytes: Some(vec![8]),
            content: None,
            symlink: None,
            large_file_size: None,
        }];
        store
            .project_reachable_metadata(&[projected.clone()])
            .unwrap();
        let stored = store.tree(&projected.host_id, &projected.party_id).unwrap();
        assert_eq!(stored[0].entries[0].node_bytes, Some(vec![8]));
        assert!(stored[0].entries[0].content.is_none());
        assert!(stored[0].entries[0].large_file_size.is_none());

        let mut rollback = projected;
        rollback.directory_version = 1;
        rollback.directory_bytes = vec![1];
        assert!(matches!(
            store.project_reachable_metadata(&[rollback]),
            Err(Error::KvDirectoryRollback {
                stored: 2,
                received: 1
            })
        ));
    }

    #[test]
    fn permission_pruning_preserves_directory_and_dirent_rollback_anchors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let mut root = snapshot();
        let mut child = root.clone();
        child.directory_id = [8; 16];
        child.directory_version = 2;
        child.directory_bytes = vec![8];
        child.entries[0].version = 2;
        let mut edge = entry(8, b"restricted");
        edge.node_id[0] = 1;
        edge.node_bytes = None;
        edge.content = None;
        root.entries = vec![edge];
        store
            .project_reachable_tree_with_large_files(&[root.clone(), child.clone()], &[])
            .unwrap();

        let mut denied = root.clone();
        denied.entries[0].readable = false;
        store.project_reachable_metadata(&[denied.clone()]).unwrap();
        assert!(store
            .directory(&root.host_id, &root.party_id, &child.directory_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store.tree(&root.host_id, &root.party_id).unwrap(),
            vec![denied.clone()]
        );

        let mut replay = child.clone();
        replay.directory_version = 1;
        assert!(matches!(
            store.project_reachable_tree_with_large_files(&[root.clone(), replay], &[]),
            Err(Error::KvDirectoryRollback { .. })
        ));
        let mut fork = child.clone();
        fork.directory_bytes = vec![99];
        assert!(matches!(
            store.project_reachable_tree_with_large_files(&[root.clone(), fork], &[]),
            Err(Error::KvProjectionConflict(_))
        ));
        let mut replay = child.clone();
        replay.entries[0].version = 1;
        assert!(matches!(
            store.project_reachable_tree_with_large_files(&[root.clone(), replay], &[]),
            Err(Error::KvProjectionConflict(_))
        ));
        let mut fork = child.clone();
        fork.entries[0].dirent_bytes = vec![99];
        assert!(matches!(
            store.project_reachable_tree_with_large_files(&[root.clone(), fork], &[]),
            Err(Error::KvProjectionConflict(_))
        ));
        // A denial does not mark a still-present entry deleted. Exact reaccess
        // is permitted, and all failed projections above were atomic.
        store
            .project_reachable_tree_with_large_files(&[root.clone(), child.clone()], &[])
            .unwrap();
        assert_eq!(store.tree(&root.host_id, &root.party_id).unwrap().len(), 2);
        let mut deleted = child.clone();
        deleted.entries.clear();
        store
            .project_reachable_tree_with_large_files(&[root.clone(), deleted], &[])
            .unwrap();
        store.project_reachable_metadata(&[denied.clone()]).unwrap();
        assert!(matches!(
            store.project_reachable_tree_with_large_files(&[root.clone(), child], &[]),
            Err(Error::KvProjectionConflict(_))
        ));

        // Missing a readable directory remains corruption, not a permission boundary.
        let mut missing = root.clone();
        missing.entries[0].node_id[1..].fill(10);
        missing.entries[0].version += 1;
        missing.entries[0].dirent_bytes = vec![10];
        assert!(matches!(
            store.project_reachable_tree_with_large_files(&[missing], &[]),
            Err(Error::InvalidKvProjection)
        ));
        denied.entries[0].content = Some(b"must not persist".to_vec());
        assert!(matches!(
            store.project_reachable_metadata(&[denied]),
            Err(Error::InvalidKvProjection)
        ));
    }

    #[test]
    fn local_namespace_mutation_invalidates_only_affected_directories() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let mut root = snapshot();
        let mut child = root.clone();
        child.directory_id = [8; 16];
        child.directory_bytes = vec![8];
        let mut child_node_id = [8; 17];
        child_node_id[0] = 1;
        root.entries = vec![KvProjectedEntry {
            readable: true,
            dirent_id: [9; 16],
            node_id: child_node_id,
            version: 1,
            directory_version: 1,
            name: b"child".to_vec(),
            write_role_type: 2,
            write_role_visibility: 0,
            creation_time: 9,
            dirent_bytes: vec![9],
            node_bytes: None,
            content: None,
            symlink: None,
            large_file_size: None,
        }];
        store
            .project_reachable_tree_with_large_files(&[root.clone(), child.clone()], &[])
            .unwrap();

        store
            .invalidate_directories(&root.host_id, &root.party_id, &[root.directory_id])
            .unwrap();

        assert!(store
            .version_vector(&root.host_id, &root.party_id)
            .unwrap()
            .is_none());
        assert!(store
            .directory(&root.host_id, &root.party_id, &root.directory_id)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .directory(&child.host_id, &child.party_id, &child.directory_id)
                .unwrap(),
            Some(child)
        );
    }

    #[test]
    fn orphan_reclamation_refuses_to_delete_a_live_owned_stage() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        let mut store = SoftStateStore::open(&path).unwrap();
        let projected = snapshot();
        let mut node_id = [9; 17];
        node_id[0] = 2;
        let mut stage = store
            .begin_large_file(&projected.host_id, &projected.party_id, node_id)
            .unwrap();

        assert!(matches!(
            store.reclaim_orphaned_large_files(),
            Err(Error::KvProjectionConflict(_))
        ));
        store.append_large_file(&mut stage, b"still live").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn plaintext_soft_database_must_be_private() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("soft.sqlite3");
        std::fs::write(&path, []).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            SoftStateStore::open(&path),
            Err(Error::InsecureSoftPermissions(0o644))
        ));
    }
}
