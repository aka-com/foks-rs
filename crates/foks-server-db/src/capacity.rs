//! Stored usage against the permanent and storage limits at which writes are
//! refused. Caps on short-lived rows that expire, such as active capability
//! tokens, remote view permissions and passphrase or recovery challenges, are
//! not reported.
//!
//! The report reads one snapshot. It scans every KV and realtime message row,
//! so it is meant for an operator command, not for each metrics scrape; the
//! limits alone come from [`CapacityResource::limit`] without a database read.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rusqlite::Connection;

use crate::error::unsigned;
use crate::team_invitations::{
    MAXIMUM_INVITATION_CERTIFICATES, MAXIMUM_LOCAL_JOIN_REQUESTS, MAXIMUM_LOCAL_VIEW_PERMISSIONS,
    MAXIMUM_REMOTE_JOIN_REQUESTS,
};
use crate::{Config, Error, ReadSnapshot, Result};

/// A stored quantity whose limit refuses further writes once it is reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityResource {
    /// The latest Merkle epoch. The limit is the last epoch whose back-pointer
    /// array has a canonical encoding; no root can be published after it.
    MerkleEpoch,
    Teams,
    /// Unexpired team-name reservations that no team has claimed.
    TeamNameReservations,
    InvitationCertificates,
    LocalJoinRequests,
    RemoteJoinRequests,
    /// User and team local view permissions together.
    LocalViewPermissions,
    WaitlistEntries,
    /// SQLite main-database pages, including free pages, in bytes.
    DatabaseBytes,
    /// Stored bytes of the largest KV namespace. The limit applies to each
    /// namespace separately.
    KvNamespaceBytes,
    /// Stored objects of the KV namespace with the most objects. The limit
    /// applies to each namespace separately.
    KvNamespaceObjects,
    /// Directory heads of the KV namespace with the most directories. The
    /// limit applies to each namespace separately.
    KvNamespaceDirectories,
    /// Directory-entry heads of the KV namespace with the most entries. The
    /// limit applies to each namespace separately.
    KvNamespaceDirents,
}

impl CapacityResource {
    pub const ALL: [Self; 13] = [
        Self::MerkleEpoch,
        Self::Teams,
        Self::TeamNameReservations,
        Self::InvitationCertificates,
        Self::LocalJoinRequests,
        Self::RemoteJoinRequests,
        Self::LocalViewPermissions,
        Self::WaitlistEntries,
        Self::DatabaseBytes,
        Self::KvNamespaceBytes,
        Self::KvNamespaceObjects,
        Self::KvNamespaceDirectories,
        Self::KvNamespaceDirents,
    ];

    /// A fixed name for reports and metric labels.
    pub fn label(self) -> &'static str {
        match self {
            Self::MerkleEpoch => "merkle_epoch",
            Self::Teams => "teams",
            Self::TeamNameReservations => "team_name_reservations",
            Self::InvitationCertificates => "invitation_certificates",
            Self::LocalJoinRequests => "local_join_requests",
            Self::RemoteJoinRequests => "remote_join_requests",
            Self::LocalViewPermissions => "local_view_permissions",
            Self::WaitlistEntries => "waitlist_entries",
            Self::DatabaseBytes => "database_bytes",
            Self::KvNamespaceBytes => "kv_namespace_bytes",
            Self::KvNamespaceObjects => "kv_namespace_objects",
            Self::KvNamespaceDirectories => "kv_namespace_directories",
            Self::KvNamespaceDirents => "kv_namespace_dirents",
        }
    }

    /// The value at which the server refuses the write that would exceed it.
    pub fn limit(self, config: &Config) -> u64 {
        match self {
            Self::MerkleEpoch => foks_merkle_store::MAX_CANONICAL_MERKLE_EPOCH - 1,
            Self::Teams => config.maximum_teams as u64,
            Self::TeamNameReservations => config.maximum_team_name_reservations as u64,
            Self::InvitationCertificates => MAXIMUM_INVITATION_CERTIFICATES as u64,
            Self::LocalJoinRequests => MAXIMUM_LOCAL_JOIN_REQUESTS as u64,
            Self::RemoteJoinRequests => MAXIMUM_REMOTE_JOIN_REQUESTS as u64,
            Self::LocalViewPermissions => MAXIMUM_LOCAL_VIEW_PERMISSIONS as u64,
            Self::WaitlistEntries => config.maximum_waitlist_entries,
            Self::DatabaseBytes => config.maximum_database_bytes,
            Self::KvNamespaceBytes => config.maximum_kv_namespace_bytes,
            Self::KvNamespaceObjects => config.maximum_kv_namespace_objects,
            Self::KvNamespaceDirectories => config.maximum_kv_directories,
            Self::KvNamespaceDirents => config.maximum_kv_dirents,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityUsage {
    pub resource: CapacityResource,
    pub used: u64,
    pub limit: u64,
}

impl CapacityUsage {
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }
}

/// Stored bytes and objects of one KV namespace, counted as the namespace
/// quota counts them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvNamespaceUsage {
    pub namespace_id: Vec<u8>,
    pub bytes: u64,
    pub objects: u64,
}

/// Stored realtime messages of one team. Bytes are the envelope and exact
/// message encodings together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealtimeTeamUsage {
    pub team_id: Vec<u8>,
    pub bytes: u64,
    pub messages: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapacityReport {
    /// One entry for each of [`CapacityResource::ALL`], in that order.
    pub usage: Vec<CapacityUsage>,
    /// Free pages inside the main database, which writes reuse before the
    /// file grows.
    pub database_free_bytes: u64,
    /// The largest namespaces by stored bytes, at most
    /// [`CapacityReport::LISTED_PARTIES`].
    pub kv_namespaces: Vec<KvNamespaceUsage>,
    /// The teams with the most stored realtime message bytes, at most
    /// [`CapacityReport::LISTED_PARTIES`].
    pub realtime_teams: Vec<RealtimeTeamUsage>,
}

impl CapacityReport {
    pub const LISTED_PARTIES: usize = 10;

    pub fn usage(&self, resource: CapacityResource) -> Option<CapacityUsage> {
        self.usage
            .iter()
            .copied()
            .find(|usage| usage.resource == resource)
    }
}

impl ReadSnapshot<'_> {
    /// Reports stored usage against each limit in [`CapacityResource::ALL`].
    /// `now` is in microseconds and decides which team-name reservations have
    /// expired.
    pub fn capacity_report(&self, now: u64) -> Result<CapacityReport> {
        let connection = self.connection();
        let config = self.config();
        let count = |sql: &str| -> Result<u64> {
            unsigned(connection.query_row(sql, [], |row| row.get(0))?)
        };
        let page_size = pragma(connection, "page_size")?;
        let database_bytes = pragma(connection, "page_count")?
            .checked_mul(page_size)
            .ok_or(Error::IntegerRange)?;
        let database_free_bytes = pragma(connection, "freelist_count")?
            .checked_mul(page_size)
            .ok_or(Error::IntegerRange)?;
        let reservations = unsigned(connection.query_row(
            "SELECT count(*) FROM team_names WHERE team_id IS NULL AND expires_at > ?1",
            [crate::error::sql_integer(now)?],
            |row| row.get(0),
        )?)?;
        let (kv_namespaces, largest_bytes, most_objects) = kv_namespaces(connection)?;
        let mut usage = Vec::with_capacity(CapacityResource::ALL.len());
        for resource in CapacityResource::ALL {
            let used = match resource {
                CapacityResource::MerkleEpoch => {
                    count("SELECT coalesce(max(epoch), 0) FROM merkle_roots")?
                }
                CapacityResource::Teams => count("SELECT count(*) FROM teams")?,
                CapacityResource::TeamNameReservations => reservations,
                CapacityResource::InvitationCertificates => {
                    count("SELECT count(*) FROM team_invitation_certificates")?
                }
                CapacityResource::LocalJoinRequests => {
                    count("SELECT count(*) FROM team_local_join_requests")?
                }
                CapacityResource::RemoteJoinRequests => {
                    count("SELECT count(*) FROM team_remote_join_requests")?
                }
                CapacityResource::LocalViewPermissions => count(
                    "SELECT (SELECT count(*) FROM user_local_view_permissions)
                          + (SELECT count(*) FROM team_local_view_permissions)",
                )?,
                CapacityResource::WaitlistEntries => {
                    count("SELECT entries FROM waitlist_count WHERE singleton = 0")?
                }
                CapacityResource::DatabaseBytes => database_bytes,
                CapacityResource::KvNamespaceBytes => largest_bytes,
                CapacityResource::KvNamespaceObjects => most_objects,
                CapacityResource::KvNamespaceDirectories => count(
                    "SELECT coalesce(max(heads), 0) FROM (
                         SELECT count(*) AS heads FROM kv_directory_heads GROUP BY uid
                     )",
                )?,
                CapacityResource::KvNamespaceDirents => count(
                    "SELECT coalesce(max(heads), 0) FROM (
                         SELECT count(*) AS heads FROM kv_dirent_heads GROUP BY uid
                     )",
                )?,
            };
            usage.push(CapacityUsage {
                resource,
                used,
                limit: resource.limit(config),
            });
        }
        Ok(CapacityReport {
            usage,
            database_free_bytes,
            kv_namespaces,
            realtime_teams: realtime_teams(connection)?,
        })
    }
}

fn pragma(connection: &Connection, name: &str) -> Result<u64> {
    unsigned(connection.pragma_query_value(None, name, |row| row.get(0))?)
}

/// Returns the largest namespaces by bytes, the largest byte count and the
/// largest object count. The terms match the namespace quota check in
/// `kv::ensure_kv_capacity`, grouped over every namespace at once. Only the
/// listed namespaces are held in memory.
fn kv_namespaces(connection: &Connection) -> Result<(Vec<KvNamespaceUsage>, u64, u64)> {
    let mut statement = connection.prepare(
        "SELECT uid, sum(bytes), sum(objects) FROM (
             SELECT uid, sum(length(exact_directory)) AS bytes, count(*) AS objects
               FROM kv_directories GROUP BY uid
             UNION ALL SELECT uid, sum(length(exact_root)), count(*) FROM kv_roots GROUP BY uid
             UNION ALL SELECT uid, sum(length(exact_node)), count(*) FROM kv_nodes GROUP BY uid
             UNION ALL SELECT uid, sum(length(exact_dirent)), count(*) FROM kv_dirents GROUP BY uid
             UNION ALL SELECT uid, sum(length(exact_metadata)), count(*)
               FROM kv_file_uploads GROUP BY uid
             UNION ALL SELECT uid, sum(length(ciphertext) + length(exact_upload_chunk)), count(*)
               FROM kv_file_chunks GROUP BY uid
             UNION ALL SELECT uid, count(*) * 64, count(*) FROM kv_locks GROUP BY uid
         ) GROUP BY uid",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    // Ordered so that the greatest key ranks lowest (fewer bytes, then the
    // larger ID); popping the maximum keeps the listed namespaces.
    let mut listed = BinaryHeap::with_capacity(CapacityReport::LISTED_PARTIES + 1);
    let (mut largest_bytes, mut most_objects) = (0, 0);
    for row in rows {
        let (namespace_id, bytes, objects) = row?;
        let (bytes, objects) = (unsigned(bytes)?, unsigned(objects)?);
        largest_bytes = largest_bytes.max(bytes);
        most_objects = most_objects.max(objects);
        listed.push((Reverse(bytes), namespace_id, objects));
        if listed.len() > CapacityReport::LISTED_PARTIES {
            listed.pop();
        }
    }
    let namespaces = listed
        .into_sorted_vec()
        .into_iter()
        .map(|(Reverse(bytes), namespace_id, objects)| KvNamespaceUsage {
            namespace_id,
            bytes,
            objects,
        })
        .collect();
    Ok((namespaces, largest_bytes, most_objects))
}

fn realtime_teams(connection: &Connection) -> Result<Vec<RealtimeTeamUsage>> {
    let mut statement = connection.prepare(
        "SELECT c.team_id,
                sum(length(m.envelope) + length(m.exact_message)) AS bytes,
                count(*)
           FROM rt_messages m JOIN rt_channels c ON c.channel_id = m.channel_id
          GROUP BY c.team_id
          ORDER BY bytes DESC, c.team_id
          LIMIT ?1",
    )?;
    let limit = i64::try_from(CapacityReport::LISTED_PARTIES).map_err(|_| Error::IntegerRange)?;
    let rows = statement.query_map([limit], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (team_id, bytes, messages) = row?;
        Ok(RealtimeTeamUsage {
            team_id,
            bytes: unsigned(bytes)?,
            messages: unsigned(messages)?,
        })
    })
    .collect()
}
