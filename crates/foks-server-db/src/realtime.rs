//! Durable ciphertext service. Authorization is evaluated on the same SQLite
//! transaction/snapshot as the operation, never from a cached inbox membership.
use crate::{error::sql_integer, Database, Error, ReadSnapshot, Result};
use foks_proto::*;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

mod limits;
pub use limits::RealtimeLimits;
use RealtimeLimits as Limits;
const MIN_ROLE: Role = Role::member(-0x4000);

/// Identity authenticated by the server. The expiry is the actual presented
/// certificate's expiry, so a queued write cannot outlive that credential.
#[derive(Clone)]
pub struct RealtimeActor {
    pub host: Vec<u8>,
    pub uid: Vec<u8>,
    pub credential: Vec<u8>,
    pub certificate_expires_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealtimeWakeTarget {
    pub host: Vec<u8>,
    pub uid: Vec<u8>,
    pub app: RtAppId,
}
pub struct RealtimeCommit<T> {
    pub value: T,
    pub wake: Vec<RealtimeWakeTarget>,
}

fn proto<T>(value: foks_proto::Result<T>) -> Result<T> {
    value.map_err(|_| Error::Invalid("realtime wire value"))
}
fn role(kind: i64, visibility: i64) -> Result<Role> {
    match kind {
        1 => Ok(Role::member(
            i16::try_from(visibility).map_err(|_| Error::IntegerRange)?,
        )),
        2 if visibility == 0 => Ok(Role::ADMIN),
        3 if visibility == 0 => Ok(Role::OWNER),
        _ => Err(Error::Invalid("stored realtime role")),
    }
}
fn check_actor(c: &Connection, a: &RealtimeActor, now: u64) -> Result<()> {
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
fn membership_role(c: &Connection, a: &RealtimeActor, team: &[u8]) -> Result<Option<Role>> {
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
fn authorize(c: &Connection, a: &RealtimeActor, team: &[u8], now: u64) -> Result<Role> {
    check_actor(c, a, now)?;
    membership_role(c, a, team)?.ok_or(Error::AuthorizationChanged)
}
fn floor(md: &RtChannelMetadata) -> Result<Role> {
    match md.tier {
        RtChannelTier::Bottom => Ok(MIN_ROLE),
        RtChannelTier::Admin => Ok(Role::ADMIN),
        _ => Err(Error::Invalid("channel tier")),
    }
}
/// One policy for discovery, history, writes, and durable inbox fanout.
/// The caller supplies membership loaded in the operation's transaction/snapshot.
struct ChannelPolicy {
    discover: Role,
    read: Role,
    write: Role,
}
impl ChannelPolicy {
    fn new(md: &RtChannelMetadata) -> Result<Self> {
        Ok(Self {
            discover: floor(md)?,
            read: md.roles.read,
            write: md.roles.write,
        })
    }
    fn can_discover(&self, role: Role) -> bool {
        role >= self.discover
    }
    fn can_read(&self, role: Role) -> bool {
        self.can_discover(role) && role >= self.read
    }
    fn require_read(&self, role: Role) -> Result<()> {
        if self.can_read(role) {
            Ok(())
        } else {
            Err(Error::AuthorizationChanged)
        }
    }
    fn require_write(&self, role: Role) -> Result<()> {
        if self.can_read(role) && role >= self.write {
            Ok(())
        } else {
            Err(Error::AuthorizationChanged)
        }
    }
    fn project(&self, role: Role, mut md: RtChannelMetadata) -> Option<RtChannelMetadata> {
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
fn current_key(c: &Connection, team: &[u8], key: RoleAndGeneration) -> Result<()> {
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
fn channel(c: &Connection, id: &[u8; 16]) -> Result<RtChannelMetadata> {
    let (bytes, activity): (Vec<u8>, Option<Vec<u8>>) = c
        .query_row(
            "SELECT metadata, last_message FROM rt_channels WHERE channel_id=?1",
            params![id.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound("realtime channel"))?;
    project_channel(&bytes, activity.as_deref())
}
// Configuration is immutable in this slice; activity is projected only for reads.
fn project_channel(bytes: &[u8], activity: Option<&[u8]>) -> Result<RtChannelMetadata> {
    let mut md = proto(RtChannelMetadata::decode(bytes))?;
    if let Some(bytes) = activity {
        let last = proto(RtLastMessage::decode(bytes))?;
        md.mtime = last.insert_time;
        md.last_message = Some(last);
    }
    Ok(md)
}
fn require_read(c: &Connection, a: &RealtimeActor, md: &RtChannelMetadata, now: u64) -> Result<()> {
    let access = authorize(c, a, md.team.entity().as_bytes(), now)?;
    ChannelPolicy::new(md)?.require_read(access)
}
fn recipients(c: &Connection, a: &RealtimeActor, md: &RtChannelMetadata) -> Result<Vec<Vec<u8>>> {
    let mut query = c.prepare("SELECT party_id, role_type, visibility FROM team_members WHERE team_id=?1
        AND source_role_type=3 AND source_visibility=0 AND (scoped_host_id IS NULL OR scoped_host_id=?2)
        ORDER BY party_id LIMIT ?3")?;
    let rows = query.query_map(
        params![
            md.team.entity().as_bytes(),
            a.host,
            (Limits::FANOUT_MEMBERS + 1) as i64
        ],
        |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    let policy = ChannelPolicy::new(md)?;
    let mut found = Vec::new();
    let mut scanned = 0;
    for row in rows {
        scanned += 1;
        if scanned > Limits::FANOUT_MEMBERS {
            return Err(Error::Capacity("realtime team fanout"));
        }
        let (uid, kind, visibility) = row?;
        let role = role(kind, visibility)?;
        if uid.first() == Some(&ENTITY_USER) && policy.can_read(role) {
            found.push(uid);
        }
    }
    Ok(found)
}
fn stamp(
    c: &Connection,
    a: &RealtimeActor,
    md: &RtChannelMetadata,
    sequence: u64,
) -> Result<Vec<RealtimeWakeTarget>> {
    let mut wake = Vec::new();
    for uid in recipients(c, a, md)? {
        c.execute("INSERT INTO rt_user_inboxes(uid,app_id,version) VALUES (?1,1,0) ON CONFLICT DO NOTHING", params![uid])?;
        let old: i64 = c.query_row(
            "SELECT version FROM rt_user_inboxes WHERE uid=?1 AND app_id=1",
            params![uid],
            |r| r.get(0),
        )?;
        let next = old.checked_add(1).ok_or(Error::IntegerRange)?;
        c.execute(
            "UPDATE rt_user_inboxes SET version=?2 WHERE uid=?1 AND app_id=1",
            params![uid, next],
        )?;
        let read = if uid == a.uid {
            sql_integer(sequence)?
        } else {
            0
        };
        c.execute("INSERT INTO rt_user_channels(uid,app_id,channel_id,inbox_version,read_through) VALUES (?1,1,?2,?3,?4)
            ON CONFLICT(uid,channel_id) DO UPDATE SET inbox_version=excluded.inbox_version, read_through=MAX(read_through,excluded.read_through)",
            params![uid,md.id.0.as_slice(),next,read])?;
        wake.push(RealtimeWakeTarget {
            host: a.host.clone(),
            uid,
            app: RtAppId::Chat,
        });
    }
    Ok(wake)
}

fn next_inbox_version(c: &Connection, uid: &[u8], app: RtAppId) -> Result<u64> {
    c.execute(
        "INSERT INTO rt_user_inboxes(uid,app_id,version) VALUES (?1,?2,0) ON CONFLICT DO NOTHING",
        params![uid, app as i64],
    )?;
    let old: i64 = c.query_row(
        "SELECT version FROM rt_user_inboxes WHERE uid=?1 AND app_id=?2",
        params![uid, app as i64],
        |r| r.get(0),
    )?;
    let next = old.checked_add(1).ok_or(Error::IntegerRange)?;
    c.execute(
        "UPDATE rt_user_inboxes SET version=?3 WHERE uid=?1 AND app_id=?2",
        params![uid, app as i64, next],
    )?;
    crate::error::unsigned(next)
}

fn remove_user_channel(
    c: &Connection,
    uid: &[u8],
    app: RtAppId,
    channel: RtChannelId,
) -> Result<bool> {
    let removed = c.execute(
        "DELETE FROM rt_user_channels WHERE uid=?1 AND app_id=?2 AND channel_id=?3",
        params![uid, app as i64, channel.0.as_slice()],
    )?;
    if removed == 0 {
        return Ok(false);
    }
    next_inbox_version(c, uid, app)?;
    Ok(true)
}

fn insert_user_channel(
    c: &Connection,
    uid: &[u8],
    app: RtAppId,
    channel: RtChannelId,
    read_through: u64,
) -> Result<bool> {
    let exists = c
        .query_row(
            "SELECT 1 FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
            params![uid, channel.0.as_slice()],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        return Ok(false);
    }
    let version = next_inbox_version(c, uid, app)?;
    c.execute(
        "INSERT INTO rt_user_channels(uid,app_id,channel_id,inbox_version,read_through) VALUES (?1,?2,?3,?4,?5)",
        params![
            uid,
            app as i64,
            channel.0.as_slice(),
            sql_integer(version)?,
            sql_integer(read_through)?
        ],
    )?;
    Ok(true)
}

impl Database {
    pub fn rt_create_channel(
        &mut self,
        actor: &RealtimeActor,
        arg: &RtCreateChannelArgument,
        now: u64,
    ) -> Result<RealtimeCommit<()>> {
        proto(arg.validate())?;
        let mut md = arg.metadata.clone();
        if md.app != RtAppId::Chat
            || md.id.0 == [0; 16]
            || md.sequence != 1
            || md.last_message.is_some()
            || md.unreadable
            || md.updated_at != arg.set_version
            || arg.set_version == 0
            || md.roles.read < floor(&md)?
            || md.roles.write < md.roles.read
            || md.name.key.role != floor(&md)?
            || md.name.boxed.ciphertext.len() < 16
            || md
                .description
                .as_ref()
                .is_some_and(|b| b.key.role != md.roles.read || b.boxed.ciphertext.len() < 16)
        {
            return Err(Error::Invalid("initial realtime channel metadata"));
        }
        md.ctime = now / 1000;
        md.mtime = now / 1000;
        let exact = proto(md.encoded())?;
        if exact.len() > Limits::CHANNEL_CONFIGURATION_BYTES {
            return Err(Error::Capacity("realtime channel metadata"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let access = authorize(&tx, actor, md.team.entity().as_bytes(), now)?;
        ChannelPolicy::new(&md)?.require_write(access)?;
        current_key(&tx, md.team.entity().as_bytes(), md.name.key)?;
        if let Some(desc) = &md.description {
            current_key(&tx, md.team.entity().as_bytes(), desc.key)?;
        }
        let exists = tx
            .query_row(
                "SELECT 1 FROM rt_channels WHERE channel_id=?1 OR short_id=?2",
                params![md.id.0.as_slice(), md.id.short().get()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            return Err(Error::Duplicate("realtime channel"));
        }
        let old: i64 = tx
            .query_row(
                "SELECT version FROM rt_channel_sets WHERE team_id=?1 AND app_id=1",
                params![md.team.entity().as_bytes()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let next = old.checked_add(1).ok_or(Error::IntegerRange)?;
        if sql_integer(arg.set_version)? != next {
            return Err(Error::RtRace);
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM rt_channels WHERE team_id=?1 AND app_id=1",
            params![md.team.entity().as_bytes()],
            |r| r.get(0),
        )?;
        if count >= Limits::CHANNELS_PER_TEAM as i64 {
            return Err(Error::Capacity("realtime channels"));
        }
        tx.execute("INSERT INTO rt_channel_sets(team_id,app_id,version,mtime) VALUES (?1,1,?2,?3)
            ON CONFLICT(team_id,app_id) DO UPDATE SET version=excluded.version,mtime=excluded.mtime",params![md.team.entity().as_bytes(),next,sql_integer(md.mtime)?])?;
        tx.execute("INSERT INTO rt_channels(channel_id,short_id,team_id,app_id,metadata) VALUES (?1,?2,?3,1,?4)",params![md.id.0.as_slice(),md.id.short().get(),md.team.entity().as_bytes(),exact])?;
        let wake = stamp(&tx, actor, &md, 0)?;
        tx.commit()?;
        Ok(RealtimeCommit { value: (), wake })
    }

    pub fn rt_send(
        &mut self,
        actor: &RealtimeActor,
        arg: &RtSendArgument,
        now: u64,
    ) -> Result<RealtimeCommit<RtSendResult>> {
        proto(arg.validate())?;
        let send = &arg.send;
        let RtMessageWrapper::Encrypted(boxed) = &send.wrapper else {
            return Err(Error::Invalid("encrypted realtime message required"));
        };
        if send.metadata.kind != RtMessageType::Basic
            || send.metadata.id.0 == [0; 16]
            || boxed.ciphertext.0.len() < 16
            || send.metadata.further_user_attribution.is_some()
            || (send.metadata.previous_sequence == 0) != (send.metadata.previous_id.0 == [0; 16])
        {
            return Err(Error::Invalid("Basic realtime message metadata"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: Vec<u8> = tx
            .query_row(
                "SELECT channel_id FROM rt_channels WHERE short_id=?1",
                params![send.channel.get()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound("realtime channel"))?;
        let md = channel(
            &tx,
            &id.as_slice()
                .try_into()
                .map_err(|_| Error::Invalid("channel id"))?,
        )?;
        let access = authorize(&tx, actor, md.team.entity().as_bytes(), now)?;
        ChannelPolicy::new(&md)?.require_write(access)?;
        if boxed.key.role != md.roles.read {
            return Err(Error::Invalid("message encryption role"));
        }
        // expected_previous_sequence is a precondition, not encrypted identity.
        let mut identity = send.clone();
        identity.expected_previous_sequence = 0;
        let envelope = proto(identity.encoded())?;
        let existing=tx.query_row("SELECT channel_id,sender,envelope,sequence,insert_time FROM rt_messages WHERE message_id=?1",params![send.metadata.id.0.as_slice()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).optional()?;
        if let Some((channel, sender, old, sequence, insert_time)) = existing {
            if channel != id || sender != actor.uid || old != envelope {
                return Err(Error::ReceiptConflict);
            }
            return Ok(RealtimeCommit {
                value: RtSendResult {
                    sequence: crate::error::unsigned(sequence)?,
                    insert_time: crate::error::unsigned(insert_time)?,
                },
                wake: vec![],
            });
        }
        current_key(&tx, md.team.entity().as_bytes(), boxed.key)?;
        let last: i64 = tx.query_row(
            "SELECT last_sequence FROM rt_channels WHERE channel_id=?1",
            params![id],
            |r| r.get(0),
        )?;
        if send.expected_previous_sequence != 0
            && sql_integer(send.expected_previous_sequence)? != last
        {
            return Err(Error::RtMessageOrder);
        }
        if send.metadata.previous_sequence > 0 {
            let previous: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT message_id FROM rt_messages WHERE channel_id=?1 AND sequence=?2",
                    params![id, sql_integer(send.metadata.previous_sequence)?],
                    |r| r.get(0),
                )
                .optional()?;
            if previous.as_deref() != Some(send.metadata.previous_id.0.as_slice()) {
                return Err(Error::RtMessageOrder);
            }
        }
        let sequence = last.checked_add(1).ok_or(Error::IntegerRange)? as u64;
        let insert_time = now / 1000;
        let sender = proto(RtPartyId::new(proto(EntityId::from_bytes(
            actor.uid.clone(),
        ))?))?;
        let message = RtMessage {
            metadata: send.metadata.clone(),
            wrapper: send.wrapper.clone(),
            sequence,
            sender: Some(sender.clone()),
            insert_time,
        };
        let exact = proto(message.encoded())?;
        if envelope.len() > Limits::STORED_MESSAGE_BYTES
            || exact.len() > Limits::STORED_MESSAGE_BYTES
        {
            return Err(Error::Capacity("realtime message"));
        }
        tx.execute("INSERT INTO rt_messages(message_id,channel_id,sequence,sender,envelope,exact_message,insert_time) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![send.metadata.id.0.as_slice(),id,sql_integer(sequence)?,actor.uid,envelope,exact,sql_integer(insert_time)?])?;
        let last_message = RtLastMessage {
            sequence,
            kind: send.metadata.kind,
            insert_time,
            sender: Some(sender),
            further_user_attribution: None,
        };
        let activity = proto(last_message.encoded())?;
        if activity.len() > Limits::CHANNEL_ACTIVITY_BYTES {
            return Err(Error::Capacity("realtime channel activity"));
        }
        tx.execute(
            "UPDATE rt_channels SET last_sequence=?2, last_message=?3 WHERE channel_id=?1",
            params![id, sql_integer(sequence)?, activity],
        )?;
        let wake = stamp(&tx, actor, &md, sequence)?;
        tx.commit()?;
        Ok(RealtimeCommit {
            value: RtSendResult {
                sequence,
                insert_time,
            },
            wake,
        })
    }

    pub fn rt_reconcile_inbox(
        &mut self,
        actor: &RealtimeActor,
        app: RtAppId,
        now: u64,
    ) -> Result<RealtimeCommit<()>> {
        if app != RtAppId::Chat {
            return Err(Error::Invalid("realtime app"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_actor(&tx, actor, now)?;
        let channels = {
            let mut query = tx.prepare(
                "SELECT metadata,last_message FROM rt_channels WHERE app_id=?1 ORDER BY channel_id LIMIT ?2",
            )?;
            let rows = query.query_map(
                params![app as i64, (Limits::INBOX_RECONCILE_CHANNELS + 1) as i64],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)),
            )?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if channels.len() > Limits::INBOX_RECONCILE_CHANNELS {
            return Err(Error::Capacity("realtime inbox reconciliation"));
        }
        let mut changed = false;
        for (bytes, activity) in channels {
            let md = project_channel(&bytes, activity.as_deref())?;
            let readable = match membership_role(&tx, actor, md.team.entity().as_bytes())? {
                Some(access) => ChannelPolicy::new(&md)?.can_read(access),
                None => false,
            };
            changed |= if readable {
                insert_user_channel(&tx, &actor.uid, app, md.id, 0)?
            } else {
                remove_user_channel(&tx, &actor.uid, app, md.id)?
            };
        }
        tx.commit()?;
        Ok(RealtimeCommit {
            value: (),
            wake: changed
                .then(|| RealtimeWakeTarget {
                    host: actor.host.clone(),
                    uid: actor.uid.clone(),
                    app,
                })
                .into_iter()
                .collect(),
        })
    }

    pub fn rt_read_through(
        &mut self,
        actor: &RealtimeActor,
        arg: &RtReadThroughArgument,
        now: u64,
    ) -> Result<RealtimeCommit<()>> {
        proto(arg.validate())?;
        if arg.read.sequence == 0 {
            return Err(Error::Invalid("realtime read sequence"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let md = channel(&tx, &arg.read.channel.0)?;
        require_read(&tx, actor, &md, now)?;
        let message_exists = tx
            .query_row(
                "SELECT 1 FROM rt_messages WHERE channel_id=?1 AND sequence=?2",
                params![
                    arg.read.channel.0.as_slice(),
                    sql_integer(arg.read.sequence)?
                ],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !message_exists {
            return Err(Error::NotFound("realtime message"));
        }
        let current: Option<i64> = tx
            .query_row(
                "SELECT read_through FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
                params![actor.uid, arg.read.channel.0.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let changed = match current {
            None => insert_user_channel(
                &tx,
                &actor.uid,
                RtAppId::Chat,
                arg.read.channel,
                arg.read.sequence,
            )?,
            Some(current) if arg.read.sequence > crate::error::unsigned(current)? => {
                let version = next_inbox_version(&tx, &actor.uid, RtAppId::Chat)?;
                tx.execute(
                    "UPDATE rt_user_channels SET inbox_version=?3,read_through=?4 WHERE uid=?1 AND channel_id=?2",
                    params![
                        actor.uid,
                        arg.read.channel.0.as_slice(),
                        sql_integer(version)?,
                        sql_integer(arg.read.sequence)?
                    ],
                )?;
                true
            }
            Some(_) => false,
        };
        tx.commit()?;
        Ok(RealtimeCommit {
            value: (),
            wake: changed
                .then(|| RealtimeWakeTarget {
                    host: actor.host.clone(),
                    uid: actor.uid.clone(),
                    app: RtAppId::Chat,
                })
                .into_iter()
                .collect(),
        })
    }
}

impl ReadSnapshot<'_> {
    pub fn rt_check_actor(&self, actor: &RealtimeActor, now: u64) -> Result<()> {
        check_actor(self.connection(), actor, now)
    }
    pub fn rt_inbox_version(&self, actor: &RealtimeActor, app: RtAppId, now: u64) -> Result<u64> {
        if app != RtAppId::Chat {
            return Err(Error::Invalid("realtime app"));
        }
        check_actor(self.connection(), actor, now)?;
        self.connection()
            .query_row(
                "SELECT version FROM rt_user_inboxes WHERE uid=?1 AND app_id=?2",
                params![actor.uid, app as i64],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .map(crate::error::unsigned)
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub fn rt_changed_threads(
        &self,
        actor: &RealtimeActor,
        arg: &RtGetChangedThreadsArgument,
        now: u64,
    ) -> Result<RtInboxDelta> {
        proto(arg.validate())?;
        if arg.query.app != RtAppId::Chat {
            return Err(Error::Invalid("realtime app"));
        }
        let c = self.connection();
        check_actor(c, actor, now)?;
        let head = c
            .query_row(
                "SELECT version FROM rt_user_inboxes WHERE uid=?1 AND app_id=?2",
                params![actor.uid, arg.query.app as i64],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .map(crate::error::unsigned)
            .transpose()?
            .unwrap_or_default();
        if head <= arg.query.since {
            return Ok(RtInboxDelta {
                inbox_version: head,
                app: arg.query.app,
                channels: vec![],
            });
        }
        let maximum = if arg.query.maximum == 0 {
            Limits::DEFAULT_INBOX_ROWS
        } else {
            usize::try_from(arg.query.maximum.min(Limits::INBOX_ROWS as u64))
                .map_err(|_| Error::IntegerRange)?
        };
        let rows = {
            let mut query = c.prepare(
                "SELECT channel_id,inbox_version,read_through FROM rt_user_channels
                 WHERE uid=?1 AND app_id=?2 AND inbox_version>?3 AND inbox_version<=?4
                 ORDER BY inbox_version ASC LIMIT ?5",
            )?;
            let rows = query.query_map(
                params![
                    actor.uid,
                    arg.query.app as i64,
                    sql_integer(arg.query.since)?,
                    sql_integer(head)?,
                    (Limits::INBOX_SCAN_ROWS + 1) as i64
                ],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut roles = std::collections::BTreeMap::<Vec<u8>, Option<Role>>::new();
        let mut channels = Vec::new();
        for (scanned, (id, inbox_version, read_through)) in rows.into_iter().enumerate() {
            if channels.len() == maximum {
                break;
            }
            if scanned == Limits::INBOX_SCAN_ROWS {
                return Err(Error::Capacity("realtime inbox scan"));
            }
            let id: [u8; 16] = id
                .try_into()
                .map_err(|_| Error::Invalid("stored realtime channel"))?;
            let md = channel(c, &id)?;
            let team = md.team.entity().as_bytes();
            let access = if let Some(access) = roles.get(team) {
                *access
            } else {
                let access = membership_role(c, actor, team)?;
                roles.insert(team.to_vec(), access);
                access
            };
            let Some(access) = access else {
                continue;
            };
            let policy = ChannelPolicy::new(&md)?;
            if !policy.can_read(access) {
                continue;
            }
            channels.push(RtInboxChannel {
                metadata: policy
                    .project(access, md)
                    .ok_or(Error::AuthorizationChanged)?,
                inbox_version: crate::error::unsigned(inbox_version)?,
                read_through: crate::error::unsigned(read_through)?,
                hidden: false,
                muted: false,
            });
        }
        Ok(RtInboxDelta {
            inbox_version: head,
            app: arg.query.app,
            channels,
        })
    }
    pub fn rt_list_channels(
        &self,
        actor: &RealtimeActor,
        arg: &RtListChannelsArgument,
        now: u64,
    ) -> Result<RtChannelSet> {
        if arg.app != RtAppId::Chat {
            return Err(Error::Invalid("realtime app"));
        }
        let c = self.connection();
        let access = authorize(c, actor, arg.team.entity().as_bytes(), now)?;
        let (version, mtime) = c
            .query_row(
                "SELECT version,mtime FROM rt_channel_sets WHERE team_id=?1 AND app_id=1",
                params![arg.team.entity().as_bytes()],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        let version = crate::error::unsigned(version)?;
        let mtime = crate::error::unsigned(mtime)?;
        if arg.last > version {
            return Err(Error::RtRace);
        }
        let mut channels = Vec::new();
        if arg.last != version {
            let mut query=c.prepare("SELECT metadata,last_message FROM rt_channels WHERE team_id=?1 AND app_id=1 ORDER BY channel_id LIMIT ?2")?;
            let rows = query.query_map(
                params![
                    arg.team.entity().as_bytes(),
                    (Limits::CHANNELS_PER_TEAM + 1) as i64
                ],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)),
            )?;
            for (i, row) in rows.enumerate() {
                if i >= Limits::CHANNELS_PER_TEAM {
                    return Err(Error::Capacity("realtime channels"));
                }
                let (bytes, activity) = row?;
                let md = project_channel(&bytes, activity.as_deref())?;
                if let Some(md) = ChannelPolicy::new(&md)?.project(access, md) {
                    channels.push(md);
                }
            }
        }
        Ok(RtChannelSet {
            version,
            channels,
            mtime,
        })
    }
    pub fn rt_recents(
        &self,
        actor: &RealtimeActor,
        arg: &RtRecentsArgument,
        now: u64,
    ) -> Result<RtMessageList> {
        let md = channel(self.connection(), &arg.channel.0)?;
        require_read(self.connection(), actor, &md, now)?;
        let limit = if arg.limit == 0 {
            Limits::DEFAULT_RECENTS_ROWS
        } else {
            usize::try_from(arg.limit).map_err(|_| Error::IntegerRange)?
        };
        if limit > Limits::HISTORY_ROWS {
            return Err(Error::Capacity("realtime page"));
        }
        let mut budget = Budget::default();
        let messages = load_messages(
            self.connection(),
            &arg.channel.0,
            sql_integer(arg.stop_at)?,
            i64::MAX,
            false,
            false,
            limit,
            &mut budget,
        )?;
        Ok(RtMessageList { messages })
    }
    pub fn rt_thread(
        &self,
        actor: &RealtimeActor,
        arg: &RtGetThreadArgument,
        now: u64,
    ) -> Result<RtThreadPage> {
        let q = &arg.query;
        let md = channel(self.connection(), &q.channel.0)?;
        require_read(self.connection(), actor, &md, now)?;
        if q.ranges.len().saturating_add(q.sequences.len()) > RT_MAX_COLLECTION {
            return Err(Error::Capacity("realtime query"));
        }
        let mut budget = Budget::default();
        let mut ranges = Vec::new();
        let mut sequences = Vec::new();
        for range in &q.ranges {
            let start = sql_integer(range.start)?;
            let end = sql_integer(range.end)?;
            let messages = load_messages(
                self.connection(),
                &q.channel.0,
                start.min(end),
                start.max(end),
                start <= end,
                true,
                Limits::HISTORY_ROWS + 1,
                &mut budget,
            )?;
            ranges.push(RtMessageList { messages });
        }
        // Found-only entries, deterministic ordering; repeated requested seqs
        // don't manufacture duplicate message rows.
        let requested: std::collections::BTreeSet<_> = q.sequences.iter().copied().collect();
        for seq in requested {
            let bytes: Option<Vec<u8>> = self
                .connection()
                .query_row(
                    "SELECT exact_message FROM rt_messages WHERE channel_id=?1 AND sequence=?2",
                    params![q.channel.0.as_slice(), sql_integer(seq)?],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(bytes) = bytes {
                budget.consume(bytes.len())?;
                sequences.push(proto(RtMessage::decode(&bytes))?);
            }
        }
        Ok(RtThreadPage { ranges, sequences })
    }
}
#[derive(Default)]
struct Budget {
    rows: usize,
    bytes: usize,
}
impl Budget {
    fn consume(&mut self, bytes: usize) -> Result<()> {
        self.rows += 1;
        self.bytes = self.bytes.checked_add(bytes).ok_or(Error::IntegerRange)?;
        if self.rows > Limits::HISTORY_ROWS || self.bytes > Limits::HISTORY_MESSAGE_BYTES {
            return Err(Error::Capacity("realtime response"));
        }
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
fn load_messages(
    c: &Connection,
    id: &[u8; 16],
    low: i64,
    high: i64,
    ascending: bool,
    inclusive: bool,
    limit: usize,
    budget: &mut Budget,
) -> Result<Vec<RtMessage>> {
    let operator = if inclusive { ">=" } else { ">" };
    let direction = if ascending { "ASC" } else { "DESC" };
    let sql=format!("SELECT exact_message FROM rt_messages WHERE channel_id=?1 AND sequence {operator} ?2 AND sequence<=?3 ORDER BY sequence {direction} LIMIT ?4");
    let mut query = c.prepare(&sql)?;
    let rows = query.query_map(params![id.as_slice(), low, high, limit as i64], |r| {
        r.get::<_, Vec<u8>>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        let bytes = row?;
        budget.consume(bytes.len())?;
        out.push(proto(RtMessage::decode(&bytes))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, ReadDatabase};
    #[test]
    fn channel_policy_covers_discovery_read_write_and_redaction() {
        let f = Fixture::new();
        for tier in [RtChannelTier::Bottom, RtChannelTier::Admin] {
            let mut md = f.create.metadata.clone();
            md.tier = tier;
            md.roles.read = Role::ADMIN;
            md.roles.write = Role::OWNER;
            md.mtime = 99;
            let policy = ChannelPolicy::new(&md).unwrap();
            for role in [MIN_ROLE, Role::member(0), Role::ADMIN, Role::OWNER] {
                let visible = tier == RtChannelTier::Bottom || role >= Role::ADMIN;
                assert_eq!(policy.can_discover(role), visible);
                assert_eq!(policy.require_read(role).is_ok(), role >= Role::ADMIN);
                assert_eq!(policy.require_write(role).is_ok(), role == Role::OWNER);
                let projected = policy.project(role, md.clone());
                assert_eq!(projected.is_some(), visible);
                if let Some(projected) = projected {
                    assert_eq!(projected.unreadable, role < Role::ADMIN);
                    assert_eq!(projected.description.is_some(), role >= Role::ADMIN);
                    assert_eq!(
                        projected.mtime,
                        if role < Role::ADMIN { md.ctime } else { 99 }
                    );
                }
            }
        }
    }
    fn id(kind: u8, tag: u8) -> Vec<u8> {
        let mut b = vec![tag; 33];
        b[0] = kind;
        b
    }
    struct Fixture {
        _dir: tempfile::TempDir,
        db: Database,
        owner: RealtimeActor,
        member: RealtimeActor,
        create: RtCreateChannelArgument,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path().join("db"), Config::default()).unwrap();
            let host = id(ENTITY_HOST, 2);
            let team = id(ENTITY_NAMED_TEAM, 3);
            let actor = |tag| RealtimeActor {
                host: host.clone(),
                uid: id(ENTITY_USER, tag),
                credential: id(ENTITY_DEVICE, tag),
                certificate_expires_at: 1_000_000_000,
            };
            let owner = actor(7);
            let member = actor(8);
            for (a, kind) in [(&owner, 3), (&member, 1)] {
                let name = a.uid.clone();
                db.connection.execute("INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES (?1,1,?2)",params![name,a.uid]).unwrap();
                db.connection.execute("INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES (?1,?2,?2,1,zeroblob(16),0)",params![a.uid,name]).unwrap();
                db.connection.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES (?1,?2,1,3,0,zeroblob(32),zeroblob(17),X'00',X'00')",params![a.credential,a.uid]).unwrap();
                if kind == 3 {
                    db.connection.execute("INSERT INTO team_names(normalized_name,reservation_sequence,team_id) VALUES (?1,1,?1)",params![team]).unwrap();
                    db.connection.execute("INSERT INTO teams(team_id,team_kind,host_id,normalized_name,team_name_utf8,team_name_sequence,team_name_commitment_key,member_load_floor_type,member_load_floor_visibility,created_at) VALUES (?1,3,?2,?1,?1,1,zeroblob(16),1,0,0)",params![team,host]).unwrap();
                }
                db.connection.execute("INSERT INTO team_members(team_id,party_id,source_role_type,source_visibility,role_type,visibility,generation,verify_key,hepk_fingerprint) VALUES (?1,?2,3,0,?3,0,1,zeroblob(33),zeroblob(32))",params![team,a.uid,kind]).unwrap();
            }
            for (kind, visibility) in [(1, -0x4000), (1, 0), (2, 0), (3, 0)] {
                db.connection.execute("INSERT INTO team_shared_keys(team_id,role_type,visibility,generation,verify_key,exact_hepk) VALUES (?1,?2,?3,1,zeroblob(33),X'00')",params![team,kind,visibility]).unwrap();
            }
            let boxed = |role| RtBox {
                key: RoleAndGeneration {
                    role,
                    generation: 1,
                },
                boxed: SecretBox {
                    nonce: [1; 16],
                    ciphertext: vec![5; 20],
                },
            };
            let create = RtCreateChannelArgument {
                set_version: 1,
                metadata: RtChannelMetadata {
                    id: RtChannelId([1; 16]),
                    team: RtTeamId::new(EntityId::from_bytes(team).unwrap()).unwrap(),
                    app: RtAppId::Chat,
                    sequence: 1,
                    name: boxed(MIN_ROLE),
                    description: Some(boxed(Role::member(0))),
                    roles: RtRolePair {
                        read: Role::member(0),
                        write: Role::member(0),
                    },
                    last_message: None,
                    ctime: 0,
                    mtime: 0,
                    updated_at: 1,
                    tier: RtChannelTier::Bottom,
                    unreadable: false,
                },
            };
            Self {
                _dir: dir,
                db,
                owner,
                member,
                create,
            }
        }
        fn send(&self, tag: u8) -> RtSendArgument {
            RtSendArgument {
                send: RtSend {
                    metadata: RtMessageMetadata {
                        id: RtMessageId([tag; 16]),
                        previous_id: RtMessageId([0; 16]),
                        previous_sequence: 0,
                        send_time: 1000,
                        kind: RtMessageType::Basic,
                        further_user_attribution: None,
                    },
                    channel: self.create.metadata.id.short(),
                    wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                        key: RoleAndGeneration {
                            role: Role::member(0),
                            generation: 1,
                        },
                        ciphertext: RtCiphertext(vec![tag; 20]),
                    }),
                    expected_previous_sequence: 0,
                },
            }
        }
        fn reader(&self) -> ReadDatabase {
            ReadDatabase::open(&self.db.path, Config::default()).unwrap()
        }
        fn count(&self, table: &str) -> i64 {
            self.db
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }
    }
    #[test]
    fn exact_replay_conflict_order_history_and_restart() {
        let mut f = Fixture::new();
        assert_eq!(
            f.db.rt_create_channel(&f.owner, &f.create, 1_000_000)
                .unwrap()
                .wake
                .len(),
            2
        );
        let first = f.send(2);
        let receipt = f.db.rt_send(&f.owner, &first, 1_001_000).unwrap();
        assert_eq!(receipt.value.sequence, 1);
        let replay = f.db.rt_send(&f.owner, &first, 1_100_000).unwrap();
        assert_eq!(receipt.value, replay.value);
        assert!(replay.wake.is_empty());
        assert_eq!(f.count("rt_messages"), 1);
        assert!(matches!(
            f.db.rt_send(&f.member, &first, 1_200_000),
            Err(Error::ReceiptConflict)
        ));
        let mut changed = first.clone();
        changed.send.metadata.send_time += 1;
        assert!(matches!(
            f.db.rt_send(&f.owner, &changed, 1_200_000),
            Err(Error::ReceiptConflict)
        ));
        let mut second = f.send(3);
        second.send.expected_previous_sequence = 2;
        assert!(matches!(
            f.db.rt_send(&f.member, &second, 1_200_000),
            Err(Error::RtMessageOrder)
        ));
        second.send.expected_previous_sequence = 1;
        second.send.metadata.previous_id = first.send.metadata.id;
        second.send.metadata.previous_sequence = 1;
        assert_eq!(
            f.db.rt_send(&f.member, &second, 1_200_000)
                .unwrap()
                .value
                .sequence,
            2
        );
        let snapshot = f.reader();
        let snapshot = snapshot.snapshot().unwrap();
        let recent = snapshot
            .rt_recents(
                &f.member,
                &RtRecentsArgument {
                    channel: f.create.metadata.id,
                    stop_at: 1,
                    limit: 10,
                },
                1_300_000,
            )
            .unwrap();
        assert_eq!(
            recent
                .messages
                .iter()
                .map(|m| m.sequence)
                .collect::<Vec<_>>(),
            [2]
        );
        let page = snapshot
            .rt_thread(
                &f.member,
                &RtGetThreadArgument {
                    query: RtThreadQuery {
                        channel: f.create.metadata.id,
                        ranges: vec![
                            RtThreadRange { start: 2, end: 1 },
                            RtThreadRange { start: 1, end: 2 },
                        ],
                        sequences: vec![99, 1, 1],
                    },
                },
                1_300_000,
            )
            .unwrap();
        assert_eq!(
            page.ranges[0]
                .messages
                .iter()
                .map(|m| m.sequence)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(page.ranges[1].messages[0].sequence, 1);
        assert_eq!(page.sequences.len(), 1);
        drop(snapshot);
        let path = f.db.path.clone();
        drop(f.db);
        f.db = Database::open_existing(path, Config::default()).unwrap();
        assert_eq!(
            f.db.rt_send(&f.owner, &first, 1_400_000).unwrap().value,
            receipt.value
        );
        let read: i64 =
            f.db.connection
                .query_row(
                    "SELECT read_through FROM rt_user_channels WHERE uid=?1",
                    params![f.member.uid],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(read, 2);
    }
    #[test]
    fn current_membership_credentials_and_generation_control_every_operation() {
        let mut f = Fixture::new();
        f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
        let first = f.send(2);
        f.db.rt_send(&f.owner, &first, 100).unwrap();
        f.db.connection.execute("INSERT INTO team_shared_keys SELECT team_id,role_type,visibility,2,verify_key,exact_hepk,start_epoch FROM team_shared_keys WHERE role_type=1 AND visibility=0",[]).unwrap();
        assert!(f.db.rt_send(&f.owner, &first, 200).is_ok());
        assert!(matches!(
            f.db.rt_send(&f.owner, &f.send(3), 200),
            Err(Error::RtRace)
        ));
        f.db.connection
            .execute(
                "DELETE FROM team_members WHERE party_id=?1",
                params![f.member.uid],
            )
            .unwrap();
        assert!(matches!(
            f.db.rt_send(&f.member, &first, 200),
            Err(Error::AuthorizationChanged)
        ));
        assert!(matches!(
            f.reader().snapshot().unwrap().rt_recents(
                &f.member,
                &RtRecentsArgument {
                    channel: f.create.metadata.id,
                    stop_at: 0,
                    limit: 1
                },
                200
            ),
            Err(Error::AuthorizationChanged)
        ));
        f.db.connection
            .execute(
                "UPDATE devices SET active=0 WHERE device_id=?1",
                params![f.owner.credential],
            )
            .unwrap();
        assert!(matches!(
            f.db.rt_send(&f.owner, &first, 200),
            Err(Error::AuthorizationChanged)
        ));
        assert_eq!(f.count("rt_messages"), 1);
    }
    #[test]
    fn tier_visibility_cas_identity_collisions_and_limits() {
        let mut f = Fixture::new();
        let mut admin = f.create.clone();
        admin.metadata.tier = RtChannelTier::Admin;
        admin.metadata.roles = RtRolePair {
            read: Role::ADMIN,
            write: Role::ADMIN,
        };
        admin.metadata.name.key.role = Role::ADMIN;
        admin.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
        assert!(matches!(
            f.db.rt_create_channel(&f.member, &admin, 100),
            Err(Error::AuthorizationChanged)
        ));
        f.db.rt_create_channel(&f.owner, &admin, 100).unwrap();
        let mut restricted = f.create.clone();
        restricted.metadata.id.0 = [2; 16];
        restricted.set_version = 2;
        restricted.metadata.updated_at = 2;
        restricted.metadata.roles = RtRolePair {
            read: Role::ADMIN,
            write: Role::ADMIN,
        };
        restricted.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
        f.db.rt_create_channel(&f.owner, &restricted, 100).unwrap();
        let mut list = RtListChannelsArgument {
            team: restricted.metadata.team.clone(),
            app: RtAppId::Chat,
            last: 0,
        };
        let page = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_list_channels(&f.member, &list, 200)
            .unwrap();
        assert_eq!(page.channels.len(), 1);
        assert!(page.channels[0].unreadable);
        assert!(page.channels[0].description.is_none());
        list.last = 2;
        assert!(f
            .reader()
            .snapshot()
            .unwrap()
            .rt_list_channels(&f.owner, &list, 200)
            .unwrap()
            .channels
            .is_empty());
        let mut collision = restricted.clone();
        collision.metadata.id.0[15] ^= 1;
        assert!(matches!(
            f.db.rt_create_channel(&f.owner, &collision, 200),
            Err(Error::Duplicate(_))
        ));
        collision.metadata.id.0 = [3; 16];
        assert!(matches!(
            f.db.rt_create_channel(&f.owner, &collision, 200),
            Err(Error::RtRace)
        ));
        let mut wrong = f.owner.clone();
        wrong.host[1] ^= 1;
        assert!(matches!(
            f.db.rt_create_channel(&wrong, &collision, 200),
            Err(Error::AuthorizationChanged)
        ));
        wrong = f.owner.clone();
        wrong.certificate_expires_at = 200;
        assert!(matches!(
            f.db.rt_create_channel(&wrong, &collision, 200),
            Err(Error::AuthorizationChanged)
        ));
        assert!(f
            .reader()
            .snapshot()
            .unwrap()
            .rt_recents(
                &f.owner,
                &RtRecentsArgument {
                    channel: restricted.metadata.id,
                    stop_at: u64::MAX,
                    limit: 1
                },
                200
            )
            .is_err());
    }
    #[test]
    fn fanout_failure_rolls_back_message_sequence_and_receipt() {
        let mut f = Fixture::new();
        f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
        f.db.connection
            .execute(
                "UPDATE rt_user_inboxes SET version=?1 WHERE uid=?2",
                params![i64::MAX, f.member.uid],
            )
            .unwrap();
        let send = f.send(2);
        assert!(matches!(
            f.db.rt_send(&f.owner, &send, 200),
            Err(Error::IntegerRange)
        ));
        assert_eq!(f.count("rt_messages"), 0);
        let last: i64 =
            f.db.connection
                .query_row("SELECT last_sequence FROM rt_channels", [], |r| r.get(0))
                .unwrap();
        assert_eq!(last, 0);
        f.db.connection
            .execute("UPDATE rt_user_inboxes SET version=1", [])
            .unwrap();
        assert_eq!(
            f.db.rt_send(&f.owner, &send, 200).unwrap().value.sequence,
            1
        );
    }
    #[test]
    fn contended_writes_recheck_authority_and_allocate_contiguous_sequences() {
        let mut f = Fixture::new();
        f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
        let path = f.db.path.clone();
        let mut queued = Database::open_existing(&path, Config::default()).unwrap();
        let tx =
            f.db.connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
        tx.execute(
            "UPDATE team_members SET role_type=1,visibility=-1 WHERE party_id=?1",
            params![f.member.uid],
        )
        .unwrap();
        let actor = f.member.clone();
        let send = RtSendArgument {
            send: RtSend {
                metadata: RtMessageMetadata {
                    id: RtMessageId([9; 16]),
                    previous_id: RtMessageId([0; 16]),
                    previous_sequence: 0,
                    send_time: 1,
                    kind: RtMessageType::Basic,
                    further_user_attribution: None,
                },
                channel: f.create.metadata.id.short(),
                wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                    key: RoleAndGeneration {
                        role: Role::member(0),
                        generation: 1,
                    },
                    ciphertext: RtCiphertext(vec![1; 20]),
                }),
                expected_previous_sequence: 0,
            },
        };
        let (ready, started) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            queued.rt_send(&actor, &send, 200)
        });
        started.recv().unwrap();
        tx.commit().unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(Error::AuthorizationChanged)
        ));
        let mut workers = Vec::new();
        for tag in 2..10 {
            let mut db = Database::open_existing(&path, Config::default()).unwrap();
            let actor = f.owner.clone();
            let send = f.send(tag);
            workers.push(std::thread::spawn(move || {
                db.rt_send(&actor, &send, 200).unwrap().value.sequence
            }));
        }
        let mut seqs: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        seqs.sort();
        assert_eq!(seqs, (1..=8).collect::<Vec<_>>());
        assert_eq!(f.count("rt_messages"), 8);
    }
    #[test]
    fn restricted_activity_is_hidden_and_new_direct_members_can_discover() {
        let mut f = Fixture::new();
        let mut restricted = f.create.clone();
        restricted.metadata.roles = RtRolePair {
            read: Role::ADMIN,
            write: Role::ADMIN,
        };
        restricted.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
        f.db.rt_create_channel(&f.owner, &restricted, 1000).unwrap();
        let mut send = f.send(2);
        if let RtMessageWrapper::Encrypted(boxed) = &mut send.send.wrapper {
            boxed.key.role = Role::ADMIN;
        }
        f.db.rt_send(&f.owner, &send, 5000).unwrap();
        let list = RtListChannelsArgument {
            team: restricted.metadata.team.clone(),
            app: RtAppId::Chat,
            last: 0,
        };
        let hidden = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_list_channels(&f.member, &list, 6000)
            .unwrap();
        assert!(hidden.channels[0].unreadable);
        assert_eq!(hidden.channels[0].mtime, hidden.channels[0].ctime);
        assert!(hidden.channels[0].last_message.is_none());
        f.db.connection
            .execute(
                "UPDATE team_members SET role_type=2 WHERE party_id=?1",
                params![f.member.uid],
            )
            .unwrap();
        assert_eq!(
            f.db.connection
                .query_row(
                    "SELECT COUNT(*) FROM rt_user_channels WHERE uid=?1",
                    params![f.member.uid],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let visible = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_list_channels(&f.member, &list, 6000)
            .unwrap();
        assert!(!visible.channels[0].unreadable);
        assert_eq!(
            visible.channels[0].last_message.as_ref().unwrap().sequence,
            1
        );
        assert_eq!(
            f.reader()
                .snapshot()
                .unwrap()
                .rt_recents(
                    &f.member,
                    &RtRecentsArgument {
                        channel: restricted.metadata.id,
                        stop_at: 0,
                        limit: 1
                    },
                    6000
                )
                .unwrap()
                .messages
                .len(),
            1
        );
        f.db.connection
            .execute(
                "UPDATE team_members SET source_role_type=1 WHERE party_id=?1",
                params![f.member.uid],
            )
            .unwrap();
        assert!(matches!(
            f.reader()
                .snapshot()
                .unwrap()
                .rt_list_channels(&f.member, &list, 6000),
            Err(Error::AuthorizationChanged)
        ));
        let mut recovery = f.owner.clone();
        recovery.credential[0] = ENTITY_BACKUP_KEY;
        assert!(matches!(
            f.db.rt_send(&recovery, &send, 6000),
            Err(Error::AuthorizationChanged)
        ));
    }
    #[test]
    fn maximum_channel_metadata_can_accept_a_message_and_replay_ignores_precondition() {
        let mut f = Fixture::new();
        let mut create = f.create.clone();
        create.metadata.ctime = 1;
        create.metadata.mtime = 1;
        create
            .metadata
            .name
            .boxed
            .ciphertext
            .resize(Limits::CHANNEL_CONFIGURATION_BYTES - 500, 4);
        let size = create.metadata.encoded().unwrap().len();
        create.metadata.name.boxed.ciphertext.resize(
            create.metadata.name.boxed.ciphertext.len() + Limits::CHANNEL_CONFIGURATION_BYTES
                - size,
            4,
        );
        assert_eq!(
            create.metadata.encoded().unwrap().len(),
            Limits::CHANNEL_CONFIGURATION_BYTES
        );
        f.db.rt_create_channel(&f.owner, &create, 1000).unwrap();
        let mut send = f.send(2);
        let receipt = f.db.rt_send(&f.owner, &send, 2000).unwrap().value;
        let stored: Vec<u8> =
            f.db.connection
                .query_row(
                    "SELECT metadata FROM rt_channels WHERE channel_id=?1",
                    params![create.metadata.id.0.as_slice()],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(stored, create.metadata.encoded().unwrap());
        let projected = channel(&f.db.connection, &create.metadata.id.0).unwrap();
        assert_eq!(projected.mtime, 2);
        assert_eq!(projected.last_message.unwrap().sequence, receipt.sequence);
        send.send.expected_previous_sequence = u64::MAX;
        assert_eq!(f.db.rt_send(&f.owner, &send, 3000).unwrap().value, receipt);
        let mut too_large = create;
        too_large.metadata.id.0 = [3; 16];
        too_large.metadata.name.boxed.ciphertext.push(4);
        assert!(matches!(
            f.db.rt_create_channel(&f.owner, &too_large, 1000),
            Err(Error::Capacity(_))
        ));
    }
    #[test]
    fn concurrent_channel_creates_have_one_cas_winner() {
        let mut f = Fixture::new();
        f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
        let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut workers = Vec::new();
        for tag in [20, 21] {
            let mut db = Database::open_existing(&f.db.path, Config::default()).unwrap();
            let actor = f.owner.clone();
            let mut arg = f.create.clone();
            arg.metadata.id = RtChannelId([tag; 16]);
            arg.set_version = 2;
            arg.metadata.updated_at = 2;
            let gate = gate.clone();
            workers.push(std::thread::spawn(move || {
                gate.wait();
                db.rt_create_channel(&actor, &arg, 200)
            }));
        }
        let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(Error::RtRace)))
                .count(),
            1
        );
        assert_eq!(f.count("rt_channels"), 2);
    }
    #[test]
    fn inbox_delta_read_through_and_membership_reconciliation_are_monotonic() {
        let mut f = Fixture::new();
        f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
        f.db.rt_send(&f.owner, &f.send(2), 200).unwrap();
        let query = |since| RtGetChangedThreadsArgument {
            query: RtChangedThreads {
                app: RtAppId::Chat,
                since,
                maximum: 100,
            },
        };
        let initial = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_changed_threads(&f.member, &query(0), 300)
            .unwrap();
        assert_eq!(initial.inbox_version, 2);
        assert_eq!(initial.channels.len(), 1);
        assert_eq!(initial.channels[0].read_through, 0);
        let marked =
            f.db.rt_read_through(
                &f.member,
                &RtReadThroughArgument {
                    read: RtReadThrough {
                        channel: f.create.metadata.id,
                        sequence: 1,
                    },
                },
                400,
            )
            .unwrap();
        assert_eq!(marked.wake.len(), 1);
        let stale =
            f.db.rt_read_through(
                &f.member,
                &RtReadThroughArgument {
                    read: RtReadThrough {
                        channel: f.create.metadata.id,
                        sequence: 1,
                    },
                },
                500,
            )
            .unwrap();
        assert!(stale.wake.is_empty());
        let changed = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_changed_threads(&f.member, &query(2), 600)
            .unwrap();
        assert_eq!(changed.inbox_version, 3);
        assert_eq!(changed.channels[0].read_through, 1);
        assert!(matches!(
            f.db.rt_read_through(
                &f.member,
                &RtReadThroughArgument {
                    read: RtReadThrough {
                        channel: f.create.metadata.id,
                        sequence: 2,
                    },
                },
                700
            ),
            Err(Error::NotFound(_))
        ));

        let mut restricted = f.create.clone();
        restricted.metadata.id = RtChannelId([2; 16]);
        restricted.metadata.updated_at = 2;
        restricted.set_version = 2;
        restricted.metadata.roles = RtRolePair {
            read: Role::ADMIN,
            write: Role::ADMIN,
        };
        restricted.metadata.description.as_mut().unwrap().key.role = Role::ADMIN;
        f.db.rt_create_channel(&f.owner, &restricted, 800).unwrap();
        assert_eq!(
            f.db.connection
                .query_row(
                    "SELECT COUNT(*) FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
                    params![f.member.uid, restricted.metadata.id.0.as_slice()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        f.db.connection
            .execute(
                "UPDATE team_members SET role_type=2,visibility=0 WHERE party_id=?1",
                params![f.member.uid],
            )
            .unwrap();
        let reconciled =
            f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 900)
                .unwrap();
        assert_eq!(reconciled.wake.len(), 1);
        let after = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_changed_threads(&f.member, &query(3), 1000)
            .unwrap();
        assert_eq!(after.channels.len(), 1);
        assert_eq!(after.channels[0].metadata.id, restricted.metadata.id);
        f.db.connection
            .execute(
                "UPDATE team_members SET role_type=1,visibility=0 WHERE party_id=?1",
                params![f.member.uid],
            )
            .unwrap();
        let reconciled =
            f.db.rt_reconcile_inbox(&f.member, RtAppId::Chat, 1050)
                .unwrap();
        assert_eq!(reconciled.wake.len(), 1);
        let filtered = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_changed_threads(&f.member, &query(0), 1100)
            .unwrap();
        assert_eq!(filtered.inbox_version, 5);
        assert_eq!(filtered.channels.len(), 1);
        assert_eq!(filtered.channels[0].metadata.id, f.create.metadata.id);
        assert_eq!(
            f.db.connection
                .query_row(
                    "SELECT COUNT(*) FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
                    params![f.member.uid, restricted.metadata.id.0.as_slice()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn inbox_page_scans_past_inaccessible_rows() {
        let mut f = Fixture::new();
        f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
        let mut second = f.create.clone();
        second.metadata.id = RtChannelId([2; 16]);
        second.metadata.updated_at = 2;
        second.set_version = 2;
        f.db.rt_create_channel(&f.owner, &second, 200).unwrap();
        let mut hidden = f.create.metadata.clone();
        hidden.roles = RtRolePair {
            read: Role::ADMIN,
            write: Role::ADMIN,
        };
        hidden.description.as_mut().unwrap().key.role = Role::ADMIN;
        f.db.connection
            .execute(
                "UPDATE rt_channels SET metadata=?2 WHERE channel_id=?1",
                params![hidden.id.0.as_slice(), hidden.encoded().unwrap()],
            )
            .unwrap();
        let delta = f
            .reader()
            .snapshot()
            .unwrap()
            .rt_changed_threads(
                &f.member,
                &RtGetChangedThreadsArgument {
                    query: RtChangedThreads {
                        app: RtAppId::Chat,
                        since: 0,
                        maximum: 1,
                    },
                },
                300,
            )
            .unwrap();
        assert_eq!(delta.channels.len(), 1);
        assert_eq!(delta.channels[0].metadata.id, second.metadata.id);
        assert_eq!(delta.channels[0].inbox_version, 2);
    }

    #[test]
    fn response_byte_limits_fail_explicitly_without_truncating_history() {
        let mut f = Fixture::new();
        f.db.rt_create_channel(&f.owner, &f.create, 100).unwrap();
        for tag in 2..11 {
            let mut send = f.send(tag);
            if let RtMessageWrapper::Encrypted(boxed) = &mut send.send.wrapper {
                boxed.ciphertext.0.resize(RT_MAX_BODY_BYTES, tag);
            }
            f.db.rt_send(&f.owner, &send, 200).unwrap();
        }
        let reader = f.reader();
        let snapshot = reader.snapshot().unwrap();
        assert!(matches!(
            snapshot.rt_recents(
                &f.owner,
                &RtRecentsArgument {
                    channel: f.create.metadata.id,
                    stop_at: 0,
                    limit: 9
                },
                300
            ),
            Err(Error::Capacity(_))
        ));
        assert_eq!(
            snapshot
                .rt_recents(
                    &f.owner,
                    &RtRecentsArgument {
                        channel: f.create.metadata.id,
                        stop_at: 0,
                        limit: 8
                    },
                    300
                )
                .unwrap()
                .messages
                .len(),
            8
        );
        assert!(snapshot
            .rt_thread(
                &f.owner,
                &RtGetThreadArgument {
                    query: RtThreadQuery {
                        channel: f.create.metadata.id,
                        ranges: vec![RtThreadRange { start: 1, end: 9 }],
                        sequences: vec![]
                    }
                },
                300
            )
            .is_err());
    }
}
