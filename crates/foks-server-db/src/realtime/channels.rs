//! Channel discovery and creation. Creation owns its writer transaction.
use super::*;

pub(super) fn channel(c: &Connection, id: &[u8; 16]) -> Result<RtChannelMetadata> {
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
pub(super) fn project_channel(bytes: &[u8], activity: Option<&[u8]>) -> Result<RtChannelMetadata> {
    let mut md = proto(RtChannelMetadata::decode(bytes))?;
    if let Some(bytes) = activity {
        let last = proto(RtLastMessage::decode(bytes))?;
        md.mtime = last.insert_time;
        md.last_message = Some(last);
    }
    Ok(md)
}
impl Database {
    pub fn rt_create_channel(
        &mut self,
        actor: &RealtimeActor,
        arg: &RtCreateChannelArgument,
        now: u64,
    ) -> Result<RealtimeCommit<()>> {
        self.rt_create_channel_with_format(actor, arg, now, 1)
    }

    // No remote extended creation route is enabled until its durable command
    // and send-confirmation contracts are implemented. Kept private to the RT subsystem.
    pub(super) fn rt_create_channel_with_format(
        &mut self,
        actor: &RealtimeActor,
        arg: &RtCreateChannelArgument,
        now: u64,
        format: i64,
    ) -> Result<RealtimeCommit<()>> {
        if !matches!(format, 1 | 2) {
            return Err(Error::Invalid("realtime channel format"));
        }
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
        tx.execute("INSERT INTO rt_channels(channel_id,short_id,team_id,app_id,metadata,format) VALUES (?1,?2,?3,1,?4,?5)",params![md.id.0.as_slice(),md.id.short().get(),md.team.entity().as_bytes(),exact,format])?;
        let wake = stamp(&tx, actor, &md, 0)?;
        tx.commit()?;
        Ok(RealtimeCommit { value: (), wake })
    }
}

impl ReadSnapshot<'_> {
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
            let mut query=c.prepare("SELECT metadata,last_message FROM rt_channels WHERE team_id=?1 AND app_id=1 AND format=1 ORDER BY channel_id LIMIT ?2")?;
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
}
