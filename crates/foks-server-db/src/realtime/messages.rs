//! Immutable message append, exact receipts and bounded history queries.
use super::*;

impl Database {
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
        require_basic(&tx, md.id)?;
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
}

impl ReadSnapshot<'_> {
    pub fn rt_recents(
        &self,
        actor: &RealtimeActor,
        arg: &RtRecentsArgument,
        now: u64,
    ) -> Result<RtMessageList> {
        let md = channel(self.connection(), &arg.channel.0)?;
        require_read(self.connection(), actor, &md, now)?;
        require_basic(self.connection(), md.id)?;
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
        require_basic(self.connection(), md.id)?;
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
