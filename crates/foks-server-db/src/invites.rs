use rusqlite::{params, Connection, OptionalExtension as _, Transaction};

use crate::{error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum InviteRegime {
    Required = 1,
    Optional = 2,
}

impl InviteRegime {
    pub(crate) fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Required),
            2 => Ok(Self::Optional),
            _ => Err(Error::Invalid("stored invite regime")),
        }
    }

    pub fn protocol_value(self) -> u64 {
        self as u64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum InviteKind {
    Standard = 1,
    MultiUse = 2,
}

impl InviteKind {
    pub(crate) fn from_sql(value: i64) -> Result<Self> {
        match value {
            1 => Ok(Self::Standard),
            2 => Ok(Self::MultiUse),
            _ => Err(Error::Invalid("stored invite kind")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvitePolicy {
    pub revision: u64,
    pub regime: InviteRegime,
}

#[derive(Clone, Copy, Debug)]
pub enum InviteConsumption<'a> {
    Empty,
    Code {
        code_hash: &'a [u8; 32],
        kind: InviteKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedInvite {
    pub invite_id: [u8; 16],
    pub kind: InviteKind,
    pub max_uses: Option<u64>,
    pub expires_at: Option<u64>,
    pub created_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InviteSnapshot {
    pub configuration_revision: u64,
    pub invite_id: [u8; 16],
    pub kind: InviteKind,
    pub active: bool,
    pub max_uses: Option<u64>,
    pub use_count: u64,
    pub expires_at: Option<u64>,
    pub created_at: u64,
    pub disabled_at: Option<u64>,
}

impl Database {
    pub fn invite_policy(&self) -> Result<InvitePolicy> {
        invite_policy(&self.connection)
    }

    pub fn set_invite_regime(&mut self, regime: InviteRegime) -> Result<()> {
        self.connection.execute(
            "UPDATE signup_policy SET invite_regime = ?1, revision=revision+1 WHERE singleton = 1",
            [regime as i64],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn issue_invite(
        &mut self,
        invite_id: &[u8; 16],
        code_hash: &[u8; 32],
        kind: InviteKind,
        issuer_uid: Option<&[u8]>,
        max_uses: Option<u64>,
        expires_at: Option<u64>,
        now: u64,
    ) -> Result<IssuedInvite> {
        issue(
            &self.connection,
            invite_id,
            code_hash,
            kind,
            issuer_uid,
            max_uses,
            expires_at,
            now,
        )
    }

    pub fn disable_invite(&mut self, code_hash: &[u8; 32], now: u64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE signup_invites SET state = 2, disabled_at = ?2, configuration_revision=configuration_revision+1
             WHERE code_hash = ?1 AND state = 1",
            params![code_hash, sql_integer(now)?],
        )? == 1)
    }

    pub fn invites(&self) -> Result<Vec<InviteSnapshot>> {
        invites(&self.connection)
    }
}

impl ReadDatabase {
    pub fn invite_policy(&self) -> Result<InvitePolicy> {
        invite_policy(&self.connection)
    }

    pub fn invite_available(
        &self,
        code_hash: &[u8; 32],
        kind: InviteKind,
        now: u64,
    ) -> Result<bool> {
        invite_available(&self.connection, code_hash, kind, now)
    }

    pub fn invites(&self) -> Result<Vec<InviteSnapshot>> {
        invites(&self.connection)
    }
}

impl ReadSnapshot<'_> {
    pub fn invite_policy(&self) -> Result<InvitePolicy> {
        invite_policy(self.connection())
    }
}

pub(crate) fn consume(
    transaction: &Transaction<'_>,
    invite: InviteConsumption<'_>,
    now: u64,
) -> Result<Option<[u8; 16]>> {
    match invite {
        InviteConsumption::Empty => {
            if invite_policy(transaction)?.regime != InviteRegime::Optional {
                return Err(Error::BadInvite);
            }
            Ok(None)
        }
        InviteConsumption::Code { code_hash, kind } => {
            let invite_id = transaction
                .query_row(
                    "UPDATE signup_invites
                 SET use_count = use_count + 1
                 WHERE code_hash = ?1 AND kind = ?2 AND state = 1
                   AND (expires_at IS NULL OR expires_at > ?3)
                   AND (max_uses IS NULL OR use_count < max_uses)
                 RETURNING invite_id",
                    params![code_hash, kind as i64, sql_integer(now)?],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()?
                .ok_or(Error::BadInvite)?;
            Ok(Some(
                invite_id
                    .try_into()
                    .map_err(|_| Error::Invalid("stored invite ID"))?,
            ))
        }
    }
}

pub(crate) fn record_redemption(
    transaction: &Transaction<'_>,
    invite_id: &[u8; 16],
    uid: &[u8],
    now: u64,
) -> Result<()> {
    let changed = transaction.execute(
        "INSERT INTO signup_invite_redemptions(invite_id, uid, redeemed_at)
         VALUES (?1, ?2, ?3)",
        params![invite_id, uid, sql_integer(now)?],
    )?;
    if changed != 1 {
        return Err(Error::BadInvite);
    }
    Ok(())
}

pub(crate) fn invite_policy(connection: &Connection) -> Result<InvitePolicy> {
    let (regime, revision) = connection.query_row(
        "SELECT invite_regime,revision FROM signup_policy WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get::<_, i64>(1)?)),
    )?;
    Ok(InvitePolicy {
        revision: crate::error::unsigned(revision)?,
        regime: InviteRegime::from_sql(regime)?,
    })
}

fn invite_available(
    connection: &Connection,
    code_hash: &[u8; 32],
    kind: InviteKind,
    now: u64,
) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM signup_invites
             WHERE code_hash = ?1 AND kind = ?2 AND state = 1
               AND (expires_at IS NULL OR expires_at > ?3)
               AND (max_uses IS NULL OR use_count < max_uses)",
            params![code_hash, kind as i64, sql_integer(now)?],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn invites(connection: &Connection) -> Result<Vec<InviteSnapshot>> {
    let mut statement = connection.prepare(
        "SELECT invite_id, kind, state, max_uses, use_count, expires_at, created_at, disabled_at, configuration_revision
         FROM signup_invites ORDER BY created_at, invite_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;
    rows.map(|row| {
        let (id, kind, state, max_uses, use_count, expires_at, created_at, disabled_at, revision) =
            row?;
        let invite_id = id
            .try_into()
            .map_err(|_| Error::Invalid("stored invite ID"))?;
        Ok(InviteSnapshot {
            configuration_revision: crate::error::unsigned(revision)?,
            invite_id,
            kind: InviteKind::from_sql(kind)?,
            active: state == 1,
            max_uses: max_uses.map(crate::error::unsigned).transpose()?,
            use_count: crate::error::unsigned(use_count)?,
            expires_at: expires_at.map(crate::error::unsigned).transpose()?,
            created_at: crate::error::unsigned(created_at)?,
            disabled_at: disabled_at.map(crate::error::unsigned).transpose()?,
        })
    })
    .collect()
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn issue(
    connection: &Connection,
    invite_id: &[u8; 16],
    code_hash: &[u8; 32],
    kind: InviteKind,
    issuer_uid: Option<&[u8]>,
    max_uses: Option<u64>,
    expires_at: Option<u64>,
    now: u64,
) -> Result<IssuedInvite> {
    if issuer_uid.is_some_and(|uid| uid.len() != 33)
        || expires_at.is_some_and(|expiry| expiry <= now)
        || matches!(kind, InviteKind::Standard) && max_uses != Some(1)
        || matches!(kind, InviteKind::MultiUse) && max_uses == Some(0)
    {
        return Err(Error::Invalid("invite issuance"));
    }
    connection.execute(
        "INSERT INTO signup_invites
             (invite_id, code_hash, kind, issuer_uid, state, max_uses, use_count,
              expires_at, created_at, disabled_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, 0, ?6, ?7, NULL)",
        params![
            invite_id,
            code_hash,
            kind as i64,
            issuer_uid,
            max_uses.map(sql_integer).transpose()?,
            expires_at.map(sql_integer).transpose()?,
            sql_integer(now)?,
        ],
    )?;
    Ok(IssuedInvite {
        invite_id: *invite_id,
        kind,
        max_uses,
        expires_at,
        created_at: now,
    })
}

pub(crate) fn set_regime_cas(c: &Connection, regime: InviteRegime, expected: u64) -> Result<()> {
    if c.execute("UPDATE signup_policy SET invite_regime=?1,revision=revision+1 WHERE singleton=1 AND revision=?2",params![regime as u8,sql_integer(expected)?])? !=1 { return Err(Error::OperationConflict); }
    Ok(())
}
pub(crate) fn disable_id_cas(c: &Connection, id: &[u8; 16], expected: u64, now: u64) -> Result<()> {
    if c.execute("UPDATE signup_invites SET state=2,disabled_at=?3,configuration_revision=configuration_revision+1 WHERE invite_id=?1 AND state=1 AND configuration_revision=?2",params![id,sql_integer(expected)?,sql_integer(now)?])? !=1 {return Err(Error::OperationConflict);}
    Ok(())
}
pub(crate) fn page(c: &Connection, after: &[u8], limit: usize) -> Result<Vec<InviteSnapshot>> {
    if limit == 0 || limit > 100 || ![0, 16].contains(&after.len()) {
        return Err(Error::Invalid("invite page"));
    }
    let mut q=c.prepare("SELECT invite_id,kind,state,max_uses,use_count,expires_at,created_at,disabled_at,configuration_revision FROM signup_invites WHERE invite_id>?1 ORDER BY invite_id LIMIT ?2")?;
    let rows = q.query_map(params![after, limit as i64], |r| {
        Ok(InviteSnapshot {
            invite_id: r.get(0)?,
            kind: match r.get::<_, u8>(1)? {
                1 => InviteKind::Standard,
                2 => InviteKind::MultiUse,
                _ => return Err(rusqlite::Error::InvalidQuery),
            },
            active: r.get::<_, u8>(2)? == 1,
            max_uses: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
            use_count: r.get::<_, i64>(4)? as u64,
            expires_at: r.get::<_, Option<i64>>(5)?.map(|v| v as u64),
            created_at: r.get::<_, i64>(6)? as u64,
            disabled_at: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
            configuration_revision: r.get::<_, i64>(8)? as u64,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
