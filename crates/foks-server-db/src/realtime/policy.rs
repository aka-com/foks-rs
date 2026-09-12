//! Authorization evaluated in the caller's transaction or read snapshot.
use super::*;

/// Invoke after authorization, on the same transaction/snapshot as the read.
pub(super) fn require_basic(c: &Connection, channel: RtChannelId) -> Result<()> {
    let format: i64 = c.query_row(
        "SELECT format FROM rt_channels WHERE channel_id=?1",
        params![channel.0.as_slice()],
        |row| row.get(0),
    )?;
    if format != 1 {
        return Err(Error::RtUnsupportedFormat);
    }
    Ok(())
}

pub(super) fn role(kind: i64, visibility: i64) -> Result<Role> {
    match kind {
        1 => Ok(Role::member(
            i16::try_from(visibility).map_err(|_| Error::IntegerRange)?,
        )),
        2 if visibility == 0 => Ok(Role::ADMIN),
        3 if visibility == 0 => Ok(Role::OWNER),
        _ => Err(Error::Invalid("stored realtime role")),
    }
}
pub(super) fn check_actor(c: &Connection, a: &RealtimeActor, now: u64) -> Result<()> {
    if now >= a.certificate_expires_at
        || a.uid.first() != Some(&ENTITY_USER)
        || !matches!(
            a.credential.first(),
            Some(&ENTITY_DEVICE) | Some(&ENTITY_SUBKEY)
        )
    {
        return Err(Error::AuthorizationChanged);
    }
    let active = c
        .query_row(
            "SELECT 1 FROM devices WHERE uid=?1 AND active=1
        AND role_type=3 AND visibility=0 AND (device_id=?2 OR subkey_id=?2)",
            params![a.uid, a.credential],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !active {
        return Err(Error::AuthorizationChanged);
    }
    Ok(())
}
pub(super) fn membership_role(
    c: &Connection,
    a: &RealtimeActor,
    team: &[u8],
) -> Result<Option<Role>> {
    c.query_row(
        "SELECT m.role_type, m.visibility FROM team_members m
        JOIN teams t ON t.team_id=m.team_id WHERE m.team_id=?1 AND t.team_kind=3
        AND t.host_id=?2 AND m.party_id=?3 AND m.source_role_type=3 AND m.source_visibility=0
        AND (m.scoped_host_id IS NULL OR m.scoped_host_id=?2)",
        params![team, a.host, a.uid],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .map(|(kind, visibility)| role(kind, visibility))
    .transpose()
}
pub(super) fn authorize(c: &Connection, a: &RealtimeActor, team: &[u8], now: u64) -> Result<Role> {
    check_actor(c, a, now)?;
    membership_role(c, a, team)?.ok_or(Error::AuthorizationChanged)
}
pub(super) fn floor(md: &RtChannelMetadata) -> Result<Role> {
    match md.tier {
        RtChannelTier::Bottom => Ok(MIN_ROLE),
        RtChannelTier::Admin => Ok(Role::ADMIN),
        _ => Err(Error::Invalid("channel tier")),
    }
}
/// One policy for discovery, history, writes, and durable inbox fanout.
/// The caller supplies membership loaded in the operation's transaction/snapshot.
pub(super) struct ChannelPolicy {
    discover: Role,
    read: Role,
    write: Role,
}
impl ChannelPolicy {
    pub(super) fn new(md: &RtChannelMetadata) -> Result<Self> {
        Ok(Self {
            discover: floor(md)?,
            read: md.roles.read,
            write: md.roles.write,
        })
    }
    pub(super) fn can_discover(&self, role: Role) -> bool {
        role >= self.discover
    }
    pub(super) fn can_read(&self, role: Role) -> bool {
        self.can_discover(role) && role >= self.read
    }
    pub(super) fn require_read(&self, role: Role) -> Result<()> {
        if self.can_read(role) {
            Ok(())
        } else {
            Err(Error::AuthorizationChanged)
        }
    }
    pub(super) fn require_write(&self, role: Role) -> Result<()> {
        if self.can_read(role) && role >= self.write {
            Ok(())
        } else {
            Err(Error::AuthorizationChanged)
        }
    }
    pub(super) fn project(
        &self,
        role: Role,
        mut md: RtChannelMetadata,
    ) -> Option<RtChannelMetadata> {
        if !self.can_discover(role) {
            return None;
        }
        if !self.can_read(role) {
            md.description = None;
            md.last_message = None;
            md.mtime = md.ctime;
            md.unreadable = true;
        }
        Some(md)
    }
}
pub(super) fn current_key(c: &Connection, team: &[u8], key: RoleAndGeneration) -> Result<()> {
    let generation: Option<i64> = c.query_row(
        "SELECT MAX(generation) FROM team_shared_keys
        WHERE team_id=?1 AND role_type=?2 AND visibility=?3",
        params![
            team,
            key.role.protocol_value() as i64,
            key.role.visibility().unwrap_or(0)
        ],
        |r| r.get(0),
    )?;
    if key.generation == 0 || generation != Some(sql_integer(key.generation)?) {
        return Err(Error::RtRace);
    }
    Ok(())
}
pub(super) fn require_read(
    c: &Connection,
    a: &RealtimeActor,
    md: &RtChannelMetadata,
    now: u64,
) -> Result<()> {
    let access = authorize(c, a, md.team.entity().as_bytes(), now)?;
    ChannelPolicy::new(md)?.require_read(access)
}
