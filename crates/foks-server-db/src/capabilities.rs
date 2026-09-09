use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result};

type TeamTokenRow = (Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64, i64, i64);
type TeamTokenWithExpiryRow = (Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64, i64, i64, i64);
type AdminTokenRow = (Vec<u8>, Vec<u8>, i64, i64, i64, Option<Vec<u8>>);
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
    pub holder_id: Vec<u8>,
    pub ptk_role_type: u64,
    pub ptk_generation: u64,
    pub ptk_verify_key: Vec<u8>,
    pub expires_at: Option<u64>,
}

impl Database {
    pub fn resolve_team_admin_token(
        &self,
        token_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<TeamAdminAuthoritySnapshot>> {
        admin_token_query(&self.connection, token_hash, now, true)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn activate_stateless_team_view_challenge(
        &mut self,
        challenge_hash: &[u8; 32],
        activation_hash: &[u8; 32],
        token_hash: &[u8; 32],
        authority: &TeamViewAuthoritySnapshot,
        key_generation: &[u8; 16],
        expires_at: u64,
        now: u64,
    ) -> Result<Option<TeamViewAuthoritySnapshot>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        reclaim(&transaction, now)?;
        if let Some((stored_token, mut stored, stored_expiry, consumed, stored_activation)) =
            challenge_row(&transaction, challenge_hash)?
        {
            if consumed && stored_activation.as_deref() == Some(activation_hash) {
                if !team_view_token_active(&transaction, &stored_token, now)? {
                    return Ok(None);
                }
                stored.expires_at = Some(stored_expiry);
                return Ok(Some(stored));
            }
            return Err(Error::ReceiptConflict);
        }
        if expires_at <= now {
            return Ok(None);
        }
        crate::capability_keys::require_active_generation(&transaction, key_generation)?;
        ensure_current_authority(&transaction, authority)?;
        let mut scoped: i64 = transaction.query_row(
            "SELECT count(*) FROM team_view_tokens
             WHERE team_id = ?1 AND member_id = ?2",
            params![authority.team_id, authority.member_id],
            |row| row.get(0),
        )?;
        while usize::try_from(scoped).unwrap_or(usize::MAX)
            >= self.config.maximum_team_view_capabilities_per_pair
        {
            let oldest: Option<Vec<u8>> = transaction
                .query_row(
                    "SELECT token_hash FROM team_view_tokens
                     WHERE team_id = ?1 AND member_id = ?2
                     ORDER BY expires_at, token_hash
                     LIMIT 1",
                    params![authority.team_id, authority.member_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(oldest) = oldest else {
                return Err(Error::QuotaExceeded);
            };
            transaction.execute(
                "DELETE FROM team_view_tokens WHERE token_hash = ?1",
                [oldest],
            )?;
            scoped -= 1;
        }
        let global: i64 =
            transaction.query_row("SELECT count(*) FROM team_view_tokens", [], |row| {
                row.get(0)
            })?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_team_view_capabilities
        {
            return Err(Error::QuotaExceeded);
        }
        transaction.execute(
            "INSERT INTO team_view_challenges
             (challenge_hash, token_hash, team_id, member_id, member_host_id,
              source_role_type, source_visibility, source_generation, key_generation,
              expires_at, consumed, activation_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11)",
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
                sql_integer(expires_at)?,
                activation_hash,
            ],
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
                sql_integer(expires_at)?,
            ],
        )?;
        transaction.commit()?;
        let mut activated = authority.clone();
        activated.expires_at = Some(expires_at);
        Ok(Some(activated))
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
        team_id: &[u8],
        holder_id: &[u8],
        ptk_role_type: u64,
        ptk_generation: u64,
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
            "SELECT count(*) FROM team_admin_tokens WHERE team_id = ?1 AND holder_id = ?2",
            params![team_id, holder_id],
            |row| row.get(0),
        )?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_team_admin_capabilities
            || usize::try_from(scoped).unwrap_or(usize::MAX)
                >= self.config.maximum_team_admin_capabilities_per_pair
        {
            return Err(Error::QuotaExceeded);
        }
        transaction.execute(
            "INSERT INTO team_admin_tokens
             (token_hash, team_id, holder_id, ptk_role_type, ptk_visibility,
              ptk_generation, expires_at, activation_hash)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, NULL)",
            params![
                token_hash,
                team_id,
                holder_id,
                sql_integer(ptk_role_type)?,
                sql_integer(ptk_generation)?,
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
        expected_team_id: &[u8],
        expected_holder_id: &[u8],
        expected_ptk_role_type: u64,
        expected_ptk_generation: u64,
    ) -> Result<Option<TeamAdminAuthoritySnapshot>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(authority) = admin_token_query(&transaction, token_hash, now, false)? else {
            return Ok(None);
        };
        if authority.team_id != expected_team_id
            || authority.holder_id != expected_holder_id
            || authority.ptk_role_type != expected_ptk_role_type
            || authority.ptk_generation != expected_ptk_generation
        {
            return Ok(None);
        }
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
        let updated = transaction.execute(
            "UPDATE team_admin_tokens SET activation_hash = ?2
             WHERE token_hash = ?1 AND activation_hash IS NULL",
            params![token_hash, activation_hash],
        )?;
        if updated != 1 {
            return Err(Error::Invalid(
                "team-admin token already activated or invalid",
            ));
        }
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
        resolve_team_view_token(&self.connection, token_hash, now)
    }

    pub fn team_admin_authority(
        &self,
        team_id: &[u8],
        holder_id: &[u8],
        ptk_role_type: u64,
        ptk_generation: u64,
    ) -> Result<Option<TeamAdminAuthoritySnapshot>> {
        admin_authority_query(
            &self.connection,
            team_id,
            holder_id,
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

impl ReadSnapshot<'_> {
    pub fn resolve_team_view_token(
        &self,
        token_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<TeamViewAuthoritySnapshot>> {
        resolve_team_view_token(self.connection(), token_hash, now)
    }

    pub fn resolve_team_admin_token(
        &self,
        token_hash: &[u8; 32],
        now: u64,
    ) -> Result<Option<TeamAdminAuthoritySnapshot>> {
        admin_token_query(self.connection(), token_hash, now, true)
    }
}

fn resolve_team_view_token(
    connection: &rusqlite::Connection,
    token_hash: &[u8; 32],
    now: u64,
) -> Result<Option<TeamViewAuthoritySnapshot>> {
    let token: Option<TeamTokenWithExpiryRow> = connection
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
        connection,
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

fn admin_authority_query(
    connection: &rusqlite::Connection,
    team_id: &[u8],
    holder_id: &[u8],
    ptk_role_type: u64,
    ptk_generation: u64,
) -> Result<Option<TeamAdminAuthoritySnapshot>> {
    let verify_key: Option<Vec<u8>> = connection
        .query_row(
            "SELECT k.verify_key
             FROM team_shared_keys k JOIN teams t ON t.team_id = k.team_id
             WHERE k.team_id = ?1 AND k.role_type = ?2 AND k.visibility = 0
               AND k.generation = ?3 AND ?2 IN (2, 3)
               AND k.generation = (
                 SELECT max(k2.generation) FROM team_shared_keys k2
                 WHERE k2.team_id = k.team_id AND k2.role_type = k.role_type
                   AND k2.visibility = k.visibility)",
            params![
                team_id,
                sql_integer(ptk_role_type)?,
                sql_integer(ptk_generation)?
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(verify_key.map(|verify_key| TeamAdminAuthoritySnapshot {
        team_id: team_id.to_vec(),
        holder_id: holder_id.to_vec(),
        ptk_role_type,
        ptk_generation,
        ptk_verify_key: verify_key,
        expires_at: None,
    }))
}

fn admin_token_query(
    connection: &rusqlite::Connection,
    token_hash: &[u8; 32],
    now: u64,
    require_active: bool,
) -> Result<Option<TeamAdminAuthoritySnapshot>> {
    let row: Option<AdminTokenRow> = connection
        .query_row(
            "SELECT team_id, holder_id, ptk_role_type, ptk_generation,
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
                ))
            },
        )
        .optional()?;
    let Some((team, holder, ptk_role, ptk_generation, expires, activation)) = row else {
        return Ok(None);
    };
    if require_active && activation.is_none() {
        return Ok(None);
    }
    let Some(mut current) = admin_authority_query(
        connection,
        &team,
        &holder,
        crate::error::unsigned(ptk_role)?,
        crate::error::unsigned(ptk_generation)?,
    )?
    else {
        return Ok(None);
    };
    current.expires_at = Some(crate::error::unsigned(expires)?);
    Ok(Some(current))
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
             FROM team_members m JOIN teams target ON target.team_id = m.team_id
             WHERE m.team_id = ?1 AND m.party_id = ?2
               AND target.host_id = ?3
               AND (m.scoped_host_id IS NULL OR m.scoped_host_id = ?3)
               AND m.source_role_type = ?4 AND m.source_visibility = ?5
               AND m.generation = ?6
               AND (
                 EXISTS (
                   SELECT 1 FROM shared_keys k
                   WHERE k.uid = m.party_id AND k.role_type = m.source_role_type
                     AND k.visibility = m.source_visibility
                     AND k.generation = m.generation AND k.verify_key = m.verify_key
                 )
                 OR EXISTS (
                   SELECT 1 FROM team_shared_keys k
                   JOIN teams source ON source.team_id = k.team_id
                   WHERE k.team_id = m.party_id AND source.host_id = ?3
                     AND k.role_type = m.source_role_type
                     AND k.visibility = m.source_visibility
                     AND k.generation = m.generation AND k.verify_key = m.verify_key
                 )
               )",
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

fn team_view_token_active(
    connection: &rusqlite::Connection,
    token_hash: &[u8],
    now: u64,
) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM team_view_tokens
                 WHERE token_hash = ?1 AND expires_at > ?2
             )",
            params![token_hash, sql_integer(now)?],
            |row| row.get(0),
        )
        .map_err(Into::into)
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
