use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, ReadDatabase, Result};

type TeamTokenRow = (Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64, i64, i64);
type TeamTokenWithExpiryRow = (Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64, i64, i64, i64);
type AdminTokenRow = (
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    i64,
    i64,
    i64,
    Vec<u8>,
    i64,
    Option<Vec<u8>>,
);
type StoredChallengeRow = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    i64,
    i64,
    Option<Vec<u8>>,
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamViewAuthoritySnapshot {
    pub team_id: Vec<u8>,
    pub member_id: Vec<u8>,
    pub member_host_id: Vec<u8>,
    pub source_role_type: u64,
    pub source_visibility: i64,
    pub source_generation: u64,
    pub source_verify_key: Vec<u8>,
    pub effective_role_type: u64,
    pub effective_visibility: i64,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamAdminAuthoritySnapshot {
    pub team_id: Vec<u8>,
    pub member_id: Vec<u8>,
    pub member_source_role_type: u64,
    pub member_source_visibility: i64,
    pub member_generation: u64,
    pub effective_role_type: u64,
    pub effective_visibility: i64,
    pub ptk_role_type: u64,
    pub ptk_generation: u64,
    pub ptk_verify_key: Vec<u8>,
    pub expires_at: Option<u64>,
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn issue_team_view_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        token_hash: &[u8; 32],
        authority: &TeamViewAuthoritySnapshot,
        key_generation: &[u8; 16],
        expires_at: u64,
        now: u64,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        reclaim(&transaction, now)?;
        let global: i64 = transaction.query_row(
            "SELECT (SELECT count(*) FROM team_view_challenges WHERE consumed = 0)
                    + (SELECT count(*) FROM team_view_tokens)",
            [],
            |row| row.get(0),
        )?;
        let scoped: i64 = transaction.query_row(
            "SELECT (SELECT count(*) FROM team_view_challenges
                     WHERE consumed = 0 AND team_id = ?1 AND member_id = ?2)
                    + (SELECT count(*) FROM team_view_tokens
                       WHERE team_id = ?1 AND member_id = ?2)",
            params![authority.team_id, authority.member_id],
            |row| row.get(0),
        )?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_team_view_capabilities
            || usize::try_from(scoped).unwrap_or(usize::MAX)
                >= self.config.maximum_team_view_capabilities_per_pair
        {
            return Err(Error::QuotaExceeded);
        }
        ensure_current_authority(&transaction, authority)?;
        transaction.execute(
            "INSERT INTO team_view_challenges
             (challenge_hash, token_hash, team_id, member_id, member_host_id,
              source_role_type, source_visibility, source_generation, key_generation,
              expires_at, consumed, activation_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, NULL)",
            params![
                challenge_hash,
                token_hash,
                authority.team_id,
                authority.member_id,
                authority.member_host_id,
                sql_integer(authority.source_role_type)?,
                authority.source_visibility,
                sql_integer(authority.source_generation)?,
                key_generation,
                sql_integer(expires_at)?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn activate_team_view_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        activation_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<TeamViewAuthoritySnapshot>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = challenge_row(&transaction, challenge_hash)?;
        let Some((token_hash, mut authority, expires_at, consumed, stored_activation)) = row else {
            return Ok(None);
        };
        if expires_at <= now {
            return Ok(None);
        }
        authority.expires_at = Some(expires_at);
        if consumed {
            if stored_activation.as_deref() == Some(activation_hash) {
                return Ok(Some(authority));
            }
            return Err(Error::ReceiptConflict);
        }
        ensure_current_authority(&transaction, &authority)?;
        transaction.execute(
            "UPDATE team_view_challenges SET consumed = 1, activation_hash = ?2
             WHERE challenge_hash = ?1 AND consumed = 0",
            params![challenge_hash, activation_hash],
        )?;
        transaction.execute(
            "INSERT INTO team_view_tokens
             (token_hash, team_id, member_id, member_host_id, source_role_type,
              source_visibility, source_generation, effective_role_type,
              effective_visibility, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                token_hash,
                authority.team_id,
                authority.member_id,
                authority.member_host_id,
                sql_integer(authority.source_role_type)?,
                authority.source_visibility,
                sql_integer(authority.source_generation)?,
                sql_integer(authority.effective_role_type)?,
                authority.effective_visibility,
                sql_integer(expires_at)?
            ],
        )?;
        transaction.commit()?;
        Ok(Some(authority))
    }

    pub fn team_view_token_is_current(
        &self,
        token_hash: &[u8; 32],
        team_id: &[u8],
        member_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        let token: Option<TeamTokenRow> = self
            .connection
            .query_row(
                "SELECT team_id, member_id, member_host_id, source_role_type,
                        source_visibility, source_generation, effective_role_type,
                        effective_visibility
                 FROM team_view_tokens WHERE token_hash = ?1 AND expires_at > ?2",
                params![token_hash, sql_integer(now)?],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            team,
            member,
            host,
            role,
            visibility,
            generation,
            effective,
            effective_visibility,
        )) = token
        else {
            return Ok(false);
        };
        if team != team_id || member != member_id {
            return Ok(false);
        }
        Ok(authority_query(
            &self.connection,
            &team,
            &member,
            &host,
            crate::error::unsigned(role)?,
            visibility,
            crate::error::unsigned(generation)?,
        )?
        .is_some_and(|current| {
            current.effective_role_type == u64::try_from(effective).unwrap_or(u64::MAX)
                && current.effective_visibility == effective_visibility
        }))
    }

    pub fn issue_team_admin_token(
        &mut self,
        token_hash: &[u8; 32],
        authority: &TeamAdminAuthoritySnapshot,
        expires_at: u64,
        now: u64,
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        reclaim(&transaction, now)?;
        let global: i64 =
            transaction.query_row("SELECT count(*) FROM team_admin_tokens", [], |row| {
                row.get(0)
            })?;
        let scoped: i64 = transaction.query_row(
            "SELECT count(*) FROM team_admin_tokens WHERE team_id = ?1 AND member_id = ?2",
            params![authority.team_id, authority.member_id],
            |row| row.get(0),
        )?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_team_admin_capabilities
            || usize::try_from(scoped).unwrap_or(usize::MAX)
                >= self.config.maximum_team_admin_capabilities_per_pair
        {
            return Err(Error::QuotaExceeded);
        }
        ensure_current_admin_authority(&transaction, authority)?;
        transaction.execute(
            "INSERT INTO team_admin_tokens
             (token_hash, team_id, member_id, member_source_role_type,
              member_source_visibility, member_generation, ptk_role_type, ptk_visibility,
              ptk_generation, ptk_verify_key, expires_at, activation_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10, NULL)",
            params![
                token_hash,
                authority.team_id,
                authority.member_id,
                sql_integer(authority.member_source_role_type)?,
                authority.member_source_visibility,
                sql_integer(authority.member_generation)?,
                sql_integer(authority.ptk_role_type)?,
                sql_integer(authority.ptk_generation)?,
                authority.ptk_verify_key,
                sql_integer(expires_at)?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn activate_team_admin_token(
        &mut self,
        token_hash: &[u8; 32],
        activation_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<TeamAdminAuthoritySnapshot>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(authority) = admin_token_query(&transaction, token_hash, now, false)? else {
            return Ok(None);
        };
        ensure_current_admin_authority(&transaction, &authority)?;
        let stored: Option<Vec<u8>> = transaction.query_row(
            "SELECT activation_hash FROM team_admin_tokens WHERE token_hash = ?1",
            [token_hash],
            |row| row.get(0),
        )?;
        if let Some(stored) = stored {
            if stored.as_slice() == activation_hash {
                return Ok(Some(authority));
            }
            return Err(Error::ReceiptConflict);
        }
        transaction.execute(
            "UPDATE team_admin_tokens SET activation_hash = ?2
             WHERE token_hash = ?1 AND activation_hash IS NULL",
            params![token_hash, activation_hash],
        )?;
        transaction.commit()?;
        Ok(Some(authority))
    }
}

impl ReadDatabase {
    pub fn team_view_authority(
        &self,
        team_id: &[u8],
        member_id: &[u8],
        member_host_id: &[u8],
        source_role_type: u64,
        source_visibility: i64,
        source_generation: u64,
    ) -> Result<Option<TeamViewAuthoritySnapshot>> {
        authority_query(
            &self.connection,
            team_id,
            member_id,
            member_host_id,
            source_role_type,
            source_visibility,
            source_generation,
        )
    }

    pub fn resolve_team_view_token(
        &self,
        token_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<TeamViewAuthoritySnapshot>> {
        let token: Option<TeamTokenWithExpiryRow> = self
            .connection
            .query_row(
                "SELECT team_id, member_id, member_host_id, source_role_type,
                        source_visibility, source_generation, effective_role_type,
                        effective_visibility, expires_at
                 FROM team_view_tokens WHERE token_hash = ?1 AND expires_at > ?2",
                params![token_hash, sql_integer(now)?],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            team,
            member,
            host,
            source_role,
            source_visibility,
            source_generation,
            effective_role,
            effective_visibility,
            expires_at,
        )) = token
        else {
            return Ok(None);
        };
        let current = authority_query(
            &self.connection,
            &team,
            &member,
            &host,
            crate::error::unsigned(source_role)?,
            source_visibility,
            crate::error::unsigned(source_generation)?,
        )?;
        Ok(current
            .filter(|authority| {
                authority.effective_role_type == u64::try_from(effective_role).unwrap_or(u64::MAX)
                    && authority.effective_visibility == effective_visibility
            })
            .map(|mut authority| {
                authority.expires_at = u64::try_from(expires_at).ok();
                authority
            }))
    }

    pub fn team_admin_authority(
        &self,
        team_id: &[u8],
        member_id: &[u8],
        ptk_role_type: u64,
        ptk_generation: u64,
    ) -> Result<Option<TeamAdminAuthoritySnapshot>> {
        admin_authority_query(
            &self.connection,
            team_id,
            member_id,
            ptk_role_type,
            ptk_generation,
        )
    }

    pub fn resolve_team_admin_token(
        &self,
        token_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<TeamAdminAuthoritySnapshot>> {
        admin_token_query(&self.connection, token_hash, now, true)
    }
}

fn admin_authority_query(
    connection: &rusqlite::Connection,
    team_id: &[u8],
    member_id: &[u8],
    ptk_role_type: u64,
    ptk_generation: u64,
) -> Result<Option<TeamAdminAuthoritySnapshot>> {
    let row: Option<(i64, i64, i64, i64, i64, Vec<u8>)> = connection
        .query_row(
            "SELECT m.source_role_type, m.source_visibility, m.generation,
                    m.role_type, m.visibility, k.verify_key
             FROM team_members m JOIN team_shared_keys k
               ON k.team_id = m.team_id AND k.role_type = ?3 AND k.visibility = 0
              AND k.generation = ?4
             WHERE m.team_id = ?1 AND m.party_id = ?2 AND m.scoped_host_id IS NULL
               AND m.role_type >= 2 AND ?3 >= 2 AND ?3 <= m.role_type",
            params![
                team_id,
                member_id,
                sql_integer(ptk_role_type)?,
                sql_integer(ptk_generation)?
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(source_role, source_visibility, generation, role, visibility, verify_key)| {
            Ok(TeamAdminAuthoritySnapshot {
                team_id: team_id.to_vec(),
                member_id: member_id.to_vec(),
                member_source_role_type: crate::error::unsigned(source_role)?,
                member_source_visibility: source_visibility,
                member_generation: crate::error::unsigned(generation)?,
                effective_role_type: crate::error::unsigned(role)?,
                effective_visibility: visibility,
                ptk_role_type,
                ptk_generation,
                ptk_verify_key: verify_key,
                expires_at: None,
            })
        },
    )
    .transpose()
}

fn admin_token_query(
    connection: &rusqlite::Connection,
    token_hash: &[u8; 32],
    now: u64,
    require_active: bool,
) -> Result<Option<TeamAdminAuthoritySnapshot>> {
    let row: Option<AdminTokenRow> = connection
        .query_row(
            "SELECT team_id, member_id, member_source_role_type, member_source_visibility,
                    member_generation, ptk_role_type, ptk_generation, ptk_verify_key,
                    expires_at, activation_hash
             FROM team_admin_tokens WHERE token_hash = ?1 AND expires_at > ?2",
            params![token_hash, sql_integer(now)?],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        team,
        member,
        source_role,
        source_visibility,
        generation,
        ptk_role,
        ptk_generation,
        ptk_verify,
        expires,
        activation,
    )) = row
    else {
        return Ok(None);
    };
    if require_active && activation.is_none() {
        return Ok(None);
    }
    let Some(mut current) = admin_authority_query(
        connection,
        &team,
        &member,
        crate::error::unsigned(ptk_role)?,
        crate::error::unsigned(ptk_generation)?,
    )?
    else {
        return Ok(None);
    };
    if current.member_source_role_type != crate::error::unsigned(source_role)?
        || current.member_source_visibility != source_visibility
        || current.member_generation != crate::error::unsigned(generation)?
        || current.ptk_verify_key != ptk_verify
    {
        return Ok(None);
    }
    current.expires_at = Some(crate::error::unsigned(expires)?);
    Ok(Some(current))
}

fn ensure_current_admin_authority(
    connection: &rusqlite::Connection,
    authority: &TeamAdminAuthoritySnapshot,
) -> Result<()> {
    if admin_authority_query(
        connection,
        &authority.team_id,
        &authority.member_id,
        authority.ptk_role_type,
        authority.ptk_generation,
    )?
    .as_ref()
    .is_none_or(|current| {
        current.member_source_role_type != authority.member_source_role_type
            || current.member_source_visibility != authority.member_source_visibility
            || current.member_generation != authority.member_generation
            || current.ptk_verify_key != authority.ptk_verify_key
    }) {
        return Err(Error::Invalid("stale team-admin authority"));
    }
    Ok(())
}

fn authority_query(
    connection: &rusqlite::Connection,
    team_id: &[u8],
    member_id: &[u8],
    member_host_id: &[u8],
    source_role_type: u64,
    source_visibility: i64,
    source_generation: u64,
) -> Result<Option<TeamViewAuthoritySnapshot>> {
    let row: Option<(Vec<u8>, i64, i64)> = connection
        .query_row(
            "SELECT m.verify_key, m.role_type, m.visibility
         FROM team_members m JOIN shared_keys k
           ON k.uid = m.party_id AND k.role_type = m.source_role_type
          AND k.visibility = m.source_visibility AND k.generation = m.generation
          AND k.verify_key = m.verify_key
         WHERE m.team_id = ?1 AND m.party_id = ?2
           AND (m.scoped_host_id IS NULL OR m.scoped_host_id = ?3)
           AND m.source_role_type = ?4 AND m.source_visibility = ?5
           AND m.generation = ?6",
            params![
                team_id,
                member_id,
                member_host_id,
                sql_integer(source_role_type)?,
                source_visibility,
                sql_integer(source_generation)?
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(verify_key, effective_role, effective_visibility)| {
        Ok(TeamViewAuthoritySnapshot {
            team_id: team_id.to_vec(),
            member_id: member_id.to_vec(),
            member_host_id: member_host_id.to_vec(),
            source_role_type,
            source_visibility,
            source_generation,
            source_verify_key: verify_key,
            effective_role_type: crate::error::unsigned(effective_role)?,
            effective_visibility,
            expires_at: None,
        })
    })
    .transpose()
}

fn ensure_current_authority(
    transaction: &rusqlite::Transaction<'_>,
    authority: &TeamViewAuthoritySnapshot,
) -> Result<()> {
    if authority_query(
        transaction,
        &authority.team_id,
        &authority.member_id,
        &authority.member_host_id,
        authority.source_role_type,
        authority.source_visibility,
        authority.source_generation,
    )?
    .as_ref()
    .is_none_or(|current| {
        current.source_verify_key != authority.source_verify_key
            || current.effective_role_type != authority.effective_role_type
            || current.effective_visibility != authority.effective_visibility
    }) {
        return Err(Error::Invalid("stale team-view authority"));
    }
    Ok(())
}

type ChallengeRow = (
    Vec<u8>,
    TeamViewAuthoritySnapshot,
    u64,
    bool,
    Option<Vec<u8>>,
);

fn challenge_row(
    connection: &rusqlite::Connection,
    challenge_hash: &[u8; 32],
) -> Result<Option<ChallengeRow>> {
    let row: Option<StoredChallengeRow> = connection
        .query_row(
            "SELECT token_hash, team_id, member_id, member_host_id, source_role_type,
                    source_visibility, source_generation, expires_at, activation_hash
             FROM team_view_challenges WHERE challenge_hash = ?1",
            [challenge_hash],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?;
    let Some((token, team, member, host, role, visibility, generation, expires, activation)) = row
    else {
        return Ok(None);
    };
    let Some(authority) = authority_query(
        connection,
        &team,
        &member,
        &host,
        crate::error::unsigned(role)?,
        visibility,
        crate::error::unsigned(generation)?,
    )?
    else {
        return Ok(None);
    };
    Ok(Some((
        token,
        authority,
        crate::error::unsigned(expires)?,
        activation.is_some(),
        activation,
    )))
}

fn reclaim(transaction: &rusqlite::Transaction<'_>, now: u64) -> Result<()> {
    transaction.execute(
        "DELETE FROM team_view_tokens WHERE expires_at <= ?1",
        [sql_integer(now)?],
    )?;
    transaction.execute(
        "DELETE FROM team_view_challenges WHERE expires_at <= ?1",
        [sql_integer(now)?],
    )?;
    transaction.execute(
        "DELETE FROM team_admin_tokens WHERE expires_at <= ?1",
        [sql_integer(now)?],
    )?;
    Ok(())
}
