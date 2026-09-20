//! Durable membership reconciliation, read pointers and inbox fanout.
use super::*;

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
pub(super) fn stamp(
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
        c.execute("INSERT INTO rt_user_channels(uid,app_id,channel_id,inbox_version,read_through,accessible) VALUES (?1,1,?2,?3,?4,1)
            ON CONFLICT(uid,channel_id) DO UPDATE SET inbox_version=excluded.inbox_version, read_through=MAX(read_through,excluded.read_through), accessible=1",
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

fn inbox_state(
    c: &Connection,
    actor: &RealtimeActor,
    app: RtAppId,
) -> Result<(RealtimeInboxState, Option<Vec<u8>>)> {
    let row = c.query_row(
        "SELECT version,reconcile_dirty,reconcile_after FROM rt_user_inboxes WHERE uid=?1 AND app_id=?2",
        params![actor.uid, app as i64],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<Vec<u8>>>(2)?)),
    ).optional()?;
    let Some((version, dirty, cursor)) = row else {
        return Ok((
            RealtimeInboxState {
                version: 0,
                reconciliation: RealtimeReconcileState::Missing,
            },
            None,
        ));
    };
    if cursor.as_ref().is_some_and(|c| c.len() != 16) {
        return Err(Error::Invalid("stored realtime reconciliation cursor"));
    }
    let reconciliation = match (dirty, cursor.is_some()) {
        (1, _) => RealtimeReconcileState::Dirty,
        (0, true) => RealtimeReconcileState::Incomplete,
        (0, false) => RealtimeReconcileState::Clean,
        _ => return Err(Error::Invalid("stored realtime reconciliation flag")),
    };
    Ok((
        RealtimeInboxState {
            version: crate::error::unsigned(version)?,
            reconciliation,
        },
        cursor,
    ))
}

fn remove_user_channel(
    c: &Connection,
    uid: &[u8],
    app: RtAppId,
    channel: RtChannelId,
) -> Result<bool> {
    let accessible = c
        .query_row(
            "SELECT accessible FROM rt_user_channels WHERE uid=?1 AND app_id=?2 AND channel_id=?3",
            params![uid, app as i64, channel.0.as_slice()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if accessible != Some(1) {
        return Ok(false);
    }
    let version = next_inbox_version(c, uid, app)?;
    c.execute(
        "UPDATE rt_user_channels SET inbox_version=?4,accessible=0 WHERE uid=?1 AND app_id=?2 AND channel_id=?3",
        params![
            uid,
            app as i64,
            channel.0.as_slice(),
            sql_integer(version)?
        ],
    )?;
    Ok(true)
}

fn insert_user_channel(
    c: &Connection,
    uid: &[u8],
    app: RtAppId,
    channel: RtChannelId,
    read_through: u64,
) -> Result<bool> {
    let accessible = c
        .query_row(
            "SELECT accessible FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
            params![uid, channel.0.as_slice()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if accessible == Some(1) {
        return Ok(false);
    }
    let version = next_inbox_version(c, uid, app)?;
    if accessible == Some(0) {
        c.execute(
            "UPDATE rt_user_channels SET inbox_version=?3,read_through=MAX(read_through,?4),accessible=1 WHERE uid=?1 AND channel_id=?2",
            params![
                uid,
                channel.0.as_slice(),
                sql_integer(version)?,
                sql_integer(read_through)?
            ],
        )?;
    } else {
        c.execute(
            "INSERT INTO rt_user_channels(uid,app_id,channel_id,inbox_version,read_through,accessible) VALUES (?1,?2,?3,?4,?5,1)",
            params![
                uid,
                app as i64,
                channel.0.as_slice(),
                sql_integer(version)?,
                sql_integer(read_through)?
            ],
        )?;
    }
    Ok(true)
}

impl Database {
    pub fn rt_reconcile_inbox(
        &mut self,
        actor: &RealtimeActor,
        app: RtAppId,
        now: u64,
    ) -> Result<RealtimeCommit<RealtimeReconcileReport>> {
        self.rt_reconcile_inbox_page(actor, app, now, Limits::INBOX_RECONCILE_CHANNELS)
    }

    // Private test seam; production always uses the fixed maximum.
    pub(super) fn rt_reconcile_inbox_page(
        &mut self,
        actor: &RealtimeActor,
        app: RtAppId,
        now: u64,
        page_size: usize,
    ) -> Result<RealtimeCommit<RealtimeReconcileReport>> {
        if page_size == 0 || page_size > Limits::INBOX_RECONCILE_CHANNELS {
            return Err(Error::Invalid("realtime reconciliation page size"));
        }
        if app != RtAppId::Chat {
            return Err(Error::Invalid("realtime app"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_actor(&tx, actor, now)?;
        tx.execute(
            "INSERT INTO rt_user_inboxes(uid,app_id,version) VALUES (?1,?2,0) ON CONFLICT DO NOTHING",
            params![actor.uid, app as i64],
        )?;
        let (state, cursor) = inbox_state(&tx, actor, app)?;
        if !state.needs_reconciliation() {
            tx.commit()?;
            return Ok(RealtimeCommit {
                value: RealtimeReconcileReport {
                    outcome: RealtimeReconcileOutcome::AlreadyClean,
                    restarted: false,
                    candidates: 0,
                    accessibility_changes: 0,
                },
                wake: vec![],
            });
        }
        let restarted = state.reconciliation == RealtimeReconcileState::Dirty;
        let after = if restarted { None } else { cursor };
        let mut channels = {
            let mut query = tx.prepare(
                "SELECT channel.channel_id,channel.metadata,channel.last_message
                 FROM team_members AS member
                 JOIN rt_channels AS channel ON channel.team_id=member.team_id
                 WHERE member.party_id=?2 AND channel.app_id=?1
                   AND (?3 IS NULL OR channel.channel_id>?3)
                 UNION
                 SELECT channel.channel_id,channel.metadata,channel.last_message
                 FROM rt_user_channels AS user_channel
                 JOIN rt_channels AS channel ON channel.channel_id=user_channel.channel_id
                 WHERE user_channel.uid=?2 AND user_channel.app_id=?1 AND user_channel.accessible=1
                   AND (?3 IS NULL OR channel.channel_id>?3)
                 ORDER BY channel_id LIMIT ?4",
            )?;
            let rows = query.query_map(
                params![app as i64, actor.uid, after, (page_size + 1) as i64],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Option<Vec<u8>>>(2)?,
                    ))
                },
            )?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let candidates = channels.len();
        let incomplete = candidates > page_size;
        channels.truncate(page_size);
        let next_cursor = if incomplete {
            channels.last().map(|channel| channel.0.clone())
        } else {
            None
        };
        let mut roles = std::collections::BTreeMap::<Vec<u8>, Option<Role>>::new();
        let mut changes = 0;
        for (_, bytes, activity) in channels {
            let md = project_channel(&bytes, activity.as_deref())?;
            let team = md.team.entity().as_bytes();
            let access = if let Some(access) = roles.get(team) {
                *access
            } else {
                let access = membership_role(&tx, actor, team)?;
                roles.insert(team.to_vec(), access);
                access
            };
            let readable = match access {
                Some(access) => ChannelPolicy::new(&md)?.can_read(access),
                None => false,
            };
            changes += usize::from(if readable {
                insert_user_channel(&tx, &actor.uid, app, md.id, 0)?
            } else {
                remove_user_channel(&tx, &actor.uid, app, md.id)?
            });
        }
        tx.execute(
            "UPDATE rt_user_inboxes SET reconcile_dirty=0,reconcile_memberships=NULL,reconcile_after=?3 WHERE uid=?1 AND app_id=?2",
            params![actor.uid, app as i64, next_cursor],
        )?;
        tx.commit()?;
        Ok(RealtimeCommit {
            value: RealtimeReconcileReport {
                outcome: if incomplete {
                    RealtimeReconcileOutcome::PageIncomplete
                } else {
                    RealtimeReconcileOutcome::PageComplete
                },
                restarted,
                candidates,
                accessibility_changes: changes,
            },
            wake: (changes != 0)
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
        require_basic(&tx, md.id)?;
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
        let current: Option<(i64, i64)> = tx
            .query_row(
                "SELECT read_through,accessible FROM rt_user_channels WHERE uid=?1 AND channel_id=?2",
                params![actor.uid, arg.read.channel.0.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let changed = match current {
            None | Some((_, 0)) => insert_user_channel(
                &tx,
                &actor.uid,
                RtAppId::Chat,
                arg.read.channel,
                arg.read.sequence,
            )?,
            Some((current, 1)) if arg.read.sequence > crate::error::unsigned(current)? => {
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
            Some((_, 1)) => false,
            Some(_) => return Err(Error::Invalid("stored realtime accessibility")),
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
    pub fn rt_inbox_state(
        &self,
        actor: &RealtimeActor,
        app: RtAppId,
        now: u64,
    ) -> Result<RealtimeInboxState> {
        if app != RtAppId::Chat {
            return Err(Error::Invalid("realtime app"));
        }
        check_actor(self.connection(), actor, now)?;
        Ok(inbox_state(self.connection(), actor, app)?.0)
    }

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
                "SELECT u.channel_id,u.inbox_version,u.read_through FROM rt_user_channels u
                 JOIN rt_channels c ON c.channel_id=u.channel_id
                 WHERE u.uid=?1 AND u.app_id=?2 AND u.accessible=1 AND c.format=1 AND u.inbox_version>?3 AND u.inbox_version<=?4
                 ORDER BY u.inbox_version ASC LIMIT ?5",
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
}
