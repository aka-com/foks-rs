use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

use crate::{error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result};

type StoredPermission = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteUserViewGrant {
    pub token_hash: [u8; 32],
    pub token_nonce: [u8; 24],
    pub token_ciphertext: Vec<u8>,
    pub key_generation: [u8; 16],
    pub expires_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteUserViewPermission {
    pub target_user_id: Vec<u8>,
    pub viewer_party_id: Vec<u8>,
    pub viewer_host_id: Vec<u8>,
    pub token_hash: [u8; 32],
    pub token_nonce: [u8; 24],
    pub token_ciphertext: Vec<u8>,
    pub key_generation: [u8; 16],
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteUserViewPermissionOutcome {
    Inserted(RemoteUserViewPermission),
    Existing(RemoteUserViewPermission),
    Renewed(RemoteUserViewPermission),
    Reissued(RemoteUserViewPermission),
}

pub type RemoteTeamViewGrant = RemoteUserViewGrant;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteTeamViewPermission {
    pub target_team_id: Vec<u8>,
    pub viewer_party_id: Vec<u8>,
    pub viewer_host_id: Vec<u8>,
    pub token_hash: [u8; 32],
    pub token_nonce: [u8; 24],
    pub token_ciphertext: Vec<u8>,
    pub key_generation: [u8; 16],
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteTeamViewPermissionOutcome {
    Inserted(RemoteTeamViewPermission),
    Existing(RemoteTeamViewPermission),
    Renewed(RemoteTeamViewPermission),
    Reissued(RemoteTeamViewPermission),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamGrantAuthority<'a> {
    pub role_type: u64,
    pub visibility: i64,
    pub generation: u64,
    pub verify_key: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRemoteMemberViewToken {
    pub member_party_id: Vec<u8>,
    pub member_host_id: Vec<u8>,
    pub ptk_generation: u64,
    pub ptk_role_type: u64,
    pub ptk_visibility: i64,
    pub exact_secret_box: Vec<u8>,
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn issue_remote_user_view_permission(
        &mut self,
        target_user_id: &[u8],
        credential_id: &[u8],
        viewer_party_id: &[u8],
        viewer_host_id: &[u8],
        grant: &RemoteUserViewGrant,
        now: u64,
    ) -> Result<RemoteUserViewPermissionOutcome> {
        validate_grant(
            target_user_id,
            &[foks_proto::ENTITY_USER],
            viewer_party_id,
            viewer_host_id,
            grant,
            now,
        )?;
        if credential_id.len() != 33 && credential_id.len() != 34 {
            return Err(Error::Invalid("federation credential ID"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let authorized: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM devices
                 WHERE uid = ?1 AND active = 1 AND (device_id = ?2 OR subkey_id = ?2)",
                params![target_user_id, credential_id],
                |row| row.get(0),
            )
            .optional()?;
        if authorized != Some(1) {
            return Err(Error::AuthorizationChanged);
        }
        crate::capability_keys::require_active_generation(&transaction, &grant.key_generation)?;
        if let Some(existing) = permission_for_scope(
            &transaction,
            target_user_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )? {
            if renewable_grant(
                existing.token_hash,
                existing.key_generation,
                existing.expires_at,
                grant,
            ) {
                transaction.execute(
                    "UPDATE federation_user_view_permissions SET
                         token_nonce = ?4, token_ciphertext = ?5, key_generation = ?6,
                         updated_at = ?7, expires_at = ?8
                     WHERE target_user_id = ?1 AND viewer_party_id = ?2
                       AND viewer_host_id = ?3 AND state = 1 AND expires_at > ?7",
                    params![
                        target_user_id,
                        viewer_party_id,
                        viewer_host_id,
                        grant.token_nonce,
                        grant.token_ciphertext,
                        grant.key_generation,
                        sql_integer(now)?,
                        sql_integer(grant.expires_at)?,
                    ],
                )?;
                let renewed = permission_for_scope(
                    &transaction,
                    target_user_id,
                    viewer_party_id,
                    viewer_host_id,
                    now,
                )?
                .ok_or(Error::Invalid("renewed federation permission disappeared"))?;
                transaction.commit()?;
                return Ok(RemoteUserViewPermissionOutcome::Renewed(renewed));
            }
            transaction.commit()?;
            return Ok(RemoteUserViewPermissionOutcome::Existing(existing));
        }

        let global: i64 = transaction.query_row(
            "SELECT count(*) FROM federation_user_view_permissions
             WHERE state = 1 AND expires_at > ?1",
            [sql_integer(now)?],
            |row| row.get(0),
        )?;
        let scoped: i64 = transaction.query_row(
            "SELECT count(*) FROM federation_user_view_permissions
             WHERE target_user_id = ?1 AND state = 1 AND expires_at > ?2",
            params![target_user_id, sql_integer(now)?],
            |row| row.get(0),
        )?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_remote_user_view_permissions
            || usize::try_from(scoped).unwrap_or(usize::MAX)
                >= self.config.maximum_remote_user_view_permissions_per_user
        {
            return Err(Error::QuotaExceeded);
        }

        let existed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM federation_user_view_permissions
                           WHERE target_user_id = ?1 AND viewer_party_id = ?2
                             AND viewer_host_id = ?3)",
            params![target_user_id, viewer_party_id, viewer_host_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO federation_user_view_permissions
                 (target_user_id, viewer_party_id, viewer_host_id, token_hash,
                  token_nonce, token_ciphertext, key_generation, state, issued_at,
                  updated_at, expires_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8, ?9, NULL)
             ON CONFLICT(target_user_id, viewer_party_id, viewer_host_id) DO UPDATE SET
                 token_hash = excluded.token_hash,
                 token_nonce = excluded.token_nonce,
                 token_ciphertext = excluded.token_ciphertext,
                 key_generation = excluded.key_generation,
                 state = 1,
                 issued_at = excluded.issued_at,
                 updated_at = excluded.updated_at,
                 expires_at = excluded.expires_at,
                 revoked_at = NULL",
            params![
                target_user_id,
                viewer_party_id,
                viewer_host_id,
                grant.token_hash,
                grant.token_nonce,
                grant.token_ciphertext,
                grant.key_generation,
                sql_integer(now)?,
                sql_integer(grant.expires_at)?
            ],
        )?;
        let permission = permission_for_scope(
            &transaction,
            target_user_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )?
        .ok_or(Error::Invalid("inserted federation permission disappeared"))?;
        transaction.commit()?;
        Ok(if existed {
            RemoteUserViewPermissionOutcome::Reissued(permission)
        } else {
            RemoteUserViewPermissionOutcome::Inserted(permission)
        })
    }

    pub fn remote_user_view_token_is_current(
        &self,
        token_hash: &[u8; 32],
        target_user_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        token_is_current(&self.connection, token_hash, target_user_id, now)
    }

    pub fn revoke_remote_user_view_permission(
        &mut self,
        target_user_id: &[u8],
        viewer_party_id: &[u8],
        viewer_host_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        revoke_permission(
            &mut self.connection,
            "federation_user_view_permissions",
            "target_user_id",
            target_user_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn issue_remote_team_view_permission(
        &mut self,
        target_team_id: &[u8],
        credential_id: &[u8],
        viewer_party_id: &[u8],
        viewer_host_id: &[u8],
        authority: &TeamGrantAuthority<'_>,
        grant: &RemoteTeamViewGrant,
        now: u64,
    ) -> Result<RemoteTeamViewPermissionOutcome> {
        validate_grant(
            target_team_id,
            &[
                foks_proto::ENTITY_NAMED_TEAM,
                foks_proto::ENTITY_AD_HOC_TEAM,
            ],
            viewer_party_id,
            viewer_host_id,
            grant,
            now,
        )?;
        if credential_id.len() != 33 && credential_id.len() != 34 {
            return Err(Error::Invalid("federation credential ID"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let authorized: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT k.verify_key
                 FROM devices d
                 JOIN team_members m ON m.party_id = d.uid
                 JOIN teams t ON t.team_id = m.team_id
                    AND (m.scoped_host_id IS NULL OR m.scoped_host_id = t.host_id)
                 JOIN team_shared_keys k ON k.team_id = m.team_id
                    AND k.role_type = ?3 AND k.visibility = ?4 AND k.generation = ?5
                 WHERE d.active = 1 AND (d.device_id = ?1 OR d.subkey_id = ?1)
                   AND m.team_id = ?2 AND m.role_type >= 2
                   AND k.generation = (
                     SELECT max(k2.generation) FROM team_shared_keys k2
                     WHERE k2.team_id = k.team_id AND k2.role_type = k.role_type
                       AND k2.visibility = k.visibility)",
                params![
                    credential_id,
                    target_team_id,
                    sql_integer(authority.role_type)?,
                    authority.visibility,
                    sql_integer(authority.generation)?
                ],
                |row| row.get(0),
            )
            .optional()?;
        if authorized.as_deref() != Some(authority.verify_key) {
            return Err(Error::AuthorizationChanged);
        }
        crate::capability_keys::require_active_generation(&transaction, &grant.key_generation)?;
        if let Some(existing) = team_permission_for_scope(
            &transaction,
            target_team_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )? {
            if renewable_grant(
                existing.token_hash,
                existing.key_generation,
                existing.expires_at,
                grant,
            ) {
                transaction.execute(
                    "UPDATE federation_team_view_permissions SET
                         token_nonce = ?4, token_ciphertext = ?5, key_generation = ?6,
                         updated_at = ?7, expires_at = ?8
                     WHERE target_team_id = ?1 AND viewer_party_id = ?2
                       AND viewer_host_id = ?3 AND state = 1 AND expires_at > ?7",
                    params![
                        target_team_id,
                        viewer_party_id,
                        viewer_host_id,
                        grant.token_nonce,
                        grant.token_ciphertext,
                        grant.key_generation,
                        sql_integer(now)?,
                        sql_integer(grant.expires_at)?,
                    ],
                )?;
                let renewed = team_permission_for_scope(
                    &transaction,
                    target_team_id,
                    viewer_party_id,
                    viewer_host_id,
                    now,
                )?
                .ok_or(Error::Invalid(
                    "renewed federation team permission disappeared",
                ))?;
                transaction.commit()?;
                return Ok(RemoteTeamViewPermissionOutcome::Renewed(renewed));
            }
            transaction.commit()?;
            return Ok(RemoteTeamViewPermissionOutcome::Existing(existing));
        }
        let global: i64 = transaction.query_row(
            "SELECT count(*) FROM federation_team_view_permissions
             WHERE state = 1 AND expires_at > ?1",
            [sql_integer(now)?],
            |row| row.get(0),
        )?;
        let scoped: i64 = transaction.query_row(
            "SELECT count(*) FROM federation_team_view_permissions
             WHERE target_team_id = ?1 AND state = 1 AND expires_at > ?2",
            params![target_team_id, sql_integer(now)?],
            |row| row.get(0),
        )?;
        if usize::try_from(global).unwrap_or(usize::MAX)
            >= self.config.maximum_active_remote_team_view_permissions
            || usize::try_from(scoped).unwrap_or(usize::MAX)
                >= self.config.maximum_remote_team_view_permissions_per_team
        {
            return Err(Error::QuotaExceeded);
        }
        let existed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM federation_team_view_permissions
                           WHERE target_team_id = ?1 AND viewer_party_id = ?2
                             AND viewer_host_id = ?3)",
            params![target_team_id, viewer_party_id, viewer_host_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO federation_team_view_permissions
                 (target_team_id, viewer_party_id, viewer_host_id, token_hash,
                  token_nonce, token_ciphertext, key_generation, state, issued_at,
                  updated_at, expires_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8, ?9, NULL)
             ON CONFLICT(target_team_id, viewer_party_id, viewer_host_id) DO UPDATE SET
                 token_hash = excluded.token_hash,
                 token_nonce = excluded.token_nonce,
                 token_ciphertext = excluded.token_ciphertext,
                 key_generation = excluded.key_generation,
                 state = 1, issued_at = excluded.issued_at,
                 updated_at = excluded.updated_at, expires_at = excluded.expires_at,
                 revoked_at = NULL",
            params![
                target_team_id,
                viewer_party_id,
                viewer_host_id,
                grant.token_hash,
                grant.token_nonce,
                grant.token_ciphertext,
                grant.key_generation,
                sql_integer(now)?,
                sql_integer(grant.expires_at)?
            ],
        )?;
        let permission = team_permission_for_scope(
            &transaction,
            target_team_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )?
        .ok_or(Error::Invalid(
            "inserted federation team permission disappeared",
        ))?;
        transaction.commit()?;
        Ok(if existed {
            RemoteTeamViewPermissionOutcome::Reissued(permission)
        } else {
            RemoteTeamViewPermissionOutcome::Inserted(permission)
        })
    }

    pub fn remote_team_view_token_is_current(
        &self,
        token_hash: &[u8; 32],
        target_team_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        team_token_is_current(&self.connection, token_hash, target_team_id, now)
    }

    pub fn revoke_remote_team_view_permission(
        &mut self,
        target_team_id: &[u8],
        viewer_party_id: &[u8],
        viewer_host_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        revoke_permission(
            &mut self.connection,
            "federation_team_view_permissions",
            "target_team_id",
            target_team_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )
    }
}

impl ReadDatabase {
    pub fn current_remote_user_view_permission(
        &self,
        target_user_id: &[u8],
        viewer_party_id: &[u8],
        viewer_host_id: &[u8],
        now: u64,
    ) -> Result<Option<RemoteUserViewPermission>> {
        permission_for_scope(
            &self.connection,
            target_user_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )
    }

    pub fn current_remote_team_view_permission(
        &self,
        target_team_id: &[u8],
        viewer_party_id: &[u8],
        viewer_host_id: &[u8],
        now: u64,
    ) -> Result<Option<RemoteTeamViewPermission>> {
        team_permission_for_scope(
            &self.connection,
            target_team_id,
            viewer_party_id,
            viewer_host_id,
            now,
        )
    }

    pub fn remote_user_view_token_is_current(
        &self,
        token_hash: &[u8; 32],
        target_user_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        token_is_current(&self.connection, token_hash, target_user_id, now)
    }

    pub fn remote_team_view_token_is_current(
        &self,
        token_hash: &[u8; 32],
        target_team_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        team_token_is_current(&self.connection, token_hash, target_team_id, now)
    }
}

impl ReadSnapshot<'_> {
    pub fn remote_user_view_token_is_current(
        &self,
        token_hash: &[u8; 32],
        target_user_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        token_is_current(self.connection(), token_hash, target_user_id, now)
    }

    pub fn remote_team_view_token_is_current(
        &self,
        token_hash: &[u8; 32],
        target_team_id: &[u8],
        now: u64,
    ) -> Result<bool> {
        team_token_is_current(self.connection(), token_hash, target_team_id, now)
    }

    pub fn remote_member_view_token(
        &self,
        target_team_id: &[u8],
        member_party_id: &[u8],
        member_host_id: &[u8],
    ) -> Result<Option<StoredRemoteMemberViewToken>> {
        self.connection()
            .query_row(
                "SELECT ptk_generation, ptk_role_type, ptk_visibility, exact_secret_box
                 FROM team_remote_member_view_tokens
                 WHERE target_team_id = ?1 AND member_party_id = ?2 AND member_host_id = ?3",
                params![target_team_id, member_party_id, member_host_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|(generation, role_type, visibility, exact_secret_box)| {
                Ok(StoredRemoteMemberViewToken {
                    member_party_id: member_party_id.to_vec(),
                    member_host_id: member_host_id.to_vec(),
                    ptk_generation: crate::error::unsigned(generation)?,
                    ptk_role_type: crate::error::unsigned(role_type)?,
                    ptk_visibility: visibility,
                    exact_secret_box,
                })
            })
            .transpose()
    }
}

fn validate_grant(
    target_id: &[u8],
    target_types: &[u8],
    viewer_party_id: &[u8],
    viewer_host_id: &[u8],
    grant: &RemoteUserViewGrant,
    now: u64,
) -> Result<()> {
    if target_id.len() != 33
        || !target_types.contains(&target_id[0])
        || viewer_party_id.len() != 33
        || !matches!(
            viewer_party_id[0],
            foks_proto::ENTITY_USER
                | foks_proto::ENTITY_NAMED_TEAM
                | foks_proto::ENTITY_AD_HOC_TEAM
        )
        || viewer_host_id.len() != 33
        || viewer_host_id[0] != foks_proto::ENTITY_HOST
        || grant.token_ciphertext.len() != 33
        || grant.expires_at <= now
    {
        return Err(Error::Invalid("remote view permission"));
    }
    Ok(())
}

fn renewable_grant(
    token_hash: [u8; 32],
    key_generation: [u8; 16],
    expires_at: u64,
    grant: &RemoteUserViewGrant,
) -> bool {
    grant.token_hash == token_hash
        && grant.expires_at >= expires_at
        && (grant.key_generation != key_generation || grant.expires_at > expires_at)
}

fn permission_for_scope(
    connection: &Connection,
    target_user_id: &[u8],
    viewer_party_id: &[u8],
    viewer_host_id: &[u8],
    now: u64,
) -> Result<Option<RemoteUserViewPermission>> {
    let stored: Option<StoredPermission> = connection
        .query_row(
            "SELECT token_hash, token_nonce, token_ciphertext, key_generation,
                    issued_at, expires_at, state
             FROM federation_user_view_permissions
             WHERE target_user_id = ?1 AND viewer_party_id = ?2 AND viewer_host_id = ?3",
            params![target_user_id, viewer_party_id, viewer_host_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((hash, nonce, ciphertext, generation, issued_at, expires_at, state)) = stored else {
        return Ok(None);
    };
    let expires_at = crate::error::unsigned(expires_at)?;
    if state != 1 || expires_at <= now {
        return Ok(None);
    }
    Ok(Some(RemoteUserViewPermission {
        target_user_id: target_user_id.to_vec(),
        viewer_party_id: viewer_party_id.to_vec(),
        viewer_host_id: viewer_host_id.to_vec(),
        token_hash: hash
            .try_into()
            .map_err(|_| Error::Invalid("stored federation token hash"))?,
        token_nonce: nonce
            .try_into()
            .map_err(|_| Error::Invalid("stored federation token nonce"))?,
        token_ciphertext: ciphertext,
        key_generation: generation
            .try_into()
            .map_err(|_| Error::Invalid("stored federation key generation"))?,
        issued_at: crate::error::unsigned(issued_at)?,
        expires_at,
    }))
}

fn token_is_current(
    connection: &Connection,
    token_hash: &[u8; 32],
    target_user_id: &[u8],
    now: u64,
) -> Result<bool> {
    if target_user_id.len() != 33 {
        return Ok(false);
    }
    Ok(connection
        .query_row(
            "SELECT 1 FROM federation_user_view_permissions
             WHERE token_hash = ?1 AND target_user_id = ?2
               AND state = 1 AND expires_at > ?3",
            params![token_hash, target_user_id, sql_integer(now)?],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        == Some(1))
}

fn team_permission_for_scope(
    connection: &Connection,
    target_team_id: &[u8],
    viewer_party_id: &[u8],
    viewer_host_id: &[u8],
    now: u64,
) -> Result<Option<RemoteTeamViewPermission>> {
    let stored: Option<StoredPermission> = connection
        .query_row(
            "SELECT token_hash, token_nonce, token_ciphertext, key_generation,
                    issued_at, expires_at, state
             FROM federation_team_view_permissions
             WHERE target_team_id = ?1 AND viewer_party_id = ?2 AND viewer_host_id = ?3",
            params![target_team_id, viewer_party_id, viewer_host_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((hash, nonce, ciphertext, generation, issued_at, expires_at, state)) = stored else {
        return Ok(None);
    };
    let expires_at = crate::error::unsigned(expires_at)?;
    if state != 1 || expires_at <= now {
        return Ok(None);
    }
    Ok(Some(RemoteTeamViewPermission {
        target_team_id: target_team_id.to_vec(),
        viewer_party_id: viewer_party_id.to_vec(),
        viewer_host_id: viewer_host_id.to_vec(),
        token_hash: hash
            .try_into()
            .map_err(|_| Error::Invalid("stored federation team token hash"))?,
        token_nonce: nonce
            .try_into()
            .map_err(|_| Error::Invalid("stored federation team token nonce"))?,
        token_ciphertext: ciphertext,
        key_generation: generation
            .try_into()
            .map_err(|_| Error::Invalid("stored federation team key generation"))?,
        issued_at: crate::error::unsigned(issued_at)?,
        expires_at,
    }))
}

fn team_token_is_current(
    connection: &Connection,
    token_hash: &[u8; 32],
    target_team_id: &[u8],
    now: u64,
) -> Result<bool> {
    if target_team_id.len() != 33 {
        return Ok(false);
    }
    Ok(connection
        .query_row(
            "SELECT 1 FROM federation_team_view_permissions
             WHERE token_hash = ?1 AND target_team_id = ?2
               AND state = 1 AND expires_at > ?3",
            params![token_hash, target_team_id, sql_integer(now)?],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        == Some(1))
}

fn revoke_permission(
    connection: &mut Connection,
    table: &'static str,
    target_column: &'static str,
    target_id: &[u8],
    viewer_party_id: &[u8],
    viewer_host_id: &[u8],
    now: u64,
) -> Result<bool> {
    if target_id.len() != 33 || viewer_party_id.len() != 33 || viewer_host_id.len() != 33 {
        return Err(Error::Invalid("remote view revocation"));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let sql = format!(
        "UPDATE {table} SET state = 0, revoked_at = ?1, updated_at = ?1
         WHERE {target_column} = ?2 AND viewer_party_id = ?3 AND viewer_host_id = ?4
           AND state = 1 AND issued_at <= ?1"
    );
    let changed = transaction.execute(
        &sql,
        params![
            sql_integer(now)?,
            target_id,
            viewer_party_id,
            viewer_host_id
        ],
    )?;
    transaction.commit()?;
    Ok(changed == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_database() -> (tempfile::TempDir, Database, Vec<u8>, Vec<u8>) {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(
            temporary.path().join("server.sqlite3"),
            crate::Config::default(),
        )
        .unwrap();
        let mut uid = vec![1; 33];
        uid[0] = foks_proto::ENTITY_USER;
        let mut device = vec![2; 33];
        device[0] = foks_proto::ENTITY_DEVICE;
        database
            .connection
            .execute(
                "INSERT INTO names
                 (normalized_name, reservation_token, reservation_sequence, expires_at, uid)
                 VALUES (x'61', NULL, 1, NULL, ?1)",
                [&uid],
            )
            .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO users
                 (uid, normalized_name, username_utf8, username_sequence,
                  username_commitment_key, created_at)
                 VALUES (?1, x'61', x'61', 1, zeroblob(16), 1)",
                [&uid],
            )
            .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO devices
                 (device_id, uid, active, role_type, visibility, subkey_id,
                  hepk_fingerprint, exact_hepk, exact_name)
                 VALUES (?1, ?2, 1, 3, 0, NULL, zeroblob(32), x'01', x'01')",
                params![device, uid],
            )
            .unwrap();
        (temporary, database, uid, device)
    }

    fn grant(fill: u8, expires_at: u64) -> RemoteUserViewGrant {
        RemoteUserViewGrant {
            token_hash: [fill; 32],
            token_nonce: [fill; 24],
            token_ciphertext: vec![fill; 33],
            key_generation: [fill; 16],
            expires_at,
        }
    }

    fn entity(entity_type: u8, fill: u8) -> Vec<u8> {
        let mut value = vec![fill; 33];
        value[0] = entity_type;
        value
    }

    fn activate_generation(database: &mut Database, generation: [u8; 16], now: u64) {
        let file = format!(
            "capability.{}.key",
            generation
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        if database.capability_key_generations().unwrap().is_empty() {
            database
                .connection
                .execute(
                    "INSERT INTO capability_key_generations
                     (generation_id, encrypted_file_name, state, created_at, retire_after)
                     VALUES (?1, ?2, 1, ?3, NULL)",
                    params![generation, file, sql_integer(now).unwrap()],
                )
                .unwrap();
        } else {
            database
                .rotate_capability_key(generation, &file, now)
                .unwrap();
        }
    }

    #[test]
    fn remote_user_permission_is_idempotent_authorized_and_expiry_bound() {
        let (_temporary, mut database, uid, device) = seeded_database();
        activate_generation(&mut database, [5; 16], 1);
        let mut viewer = vec![3; 33];
        viewer[0] = foks_proto::ENTITY_USER;
        let mut host = vec![4; 33];
        host[0] = foks_proto::ENTITY_HOST;
        let first = database
            .issue_remote_user_view_permission(&uid, &device, &viewer, &host, &grant(5, 100), 10)
            .unwrap();
        assert!(matches!(
            first,
            RemoteUserViewPermissionOutcome::Inserted(_)
        ));
        let mut competing = grant(6, 200);
        competing.key_generation = [5; 16];
        let repeated = database
            .issue_remote_user_view_permission(&uid, &device, &viewer, &host, &competing, 11)
            .unwrap();
        let RemoteUserViewPermissionOutcome::Existing(repeated) = repeated else {
            panic!("live scope must be idempotent")
        };
        assert_eq!(repeated.token_hash, [5; 32]);
        activate_generation(&mut database, [7; 16], 12);
        let mut renewal = grant(7, 200);
        renewal.token_hash = [5; 32];
        let RemoteUserViewPermissionOutcome::Renewed(renewed) = database
            .issue_remote_user_view_permission(&uid, &device, &viewer, &host, &renewal, 12)
            .unwrap()
        else {
            panic!("same bearer must be renewably rewrapped")
        };
        assert_eq!(renewed.token_hash, [5; 32]);
        assert_eq!(renewed.key_generation, [7; 16]);
        assert_eq!(renewed.expires_at, 200);
        let stale_generation = grant(5, 250);
        assert!(matches!(
            database.issue_remote_user_view_permission(
                &uid,
                &device,
                &viewer,
                &host,
                &stale_generation,
                13,
            ),
            Err(Error::Invalid("inactive capability key generation"))
        ));
        assert!(database
            .remote_user_view_token_is_current(&[5; 32], &uid, 199)
            .unwrap());
        assert!(!database
            .remote_user_view_token_is_current(&[5; 32], &uid, 200)
            .unwrap());

        let mut reissue = grant(6, 300);
        reissue.key_generation = [7; 16];
        let reissued = database
            .issue_remote_user_view_permission(&uid, &device, &viewer, &host, &reissue, 200)
            .unwrap();
        assert!(matches!(
            reissued,
            RemoteUserViewPermissionOutcome::Reissued(_)
        ));
        assert!(database
            .remote_user_view_token_is_current(&[6; 32], &uid, 299)
            .unwrap());
        assert!(database
            .revoke_remote_user_view_permission(&uid, &viewer, &host, 250)
            .unwrap());
        assert!(!database
            .remote_user_view_token_is_current(&[6; 32], &uid, 250)
            .unwrap());
        assert!(!database
            .revoke_remote_user_view_permission(&uid, &viewer, &host, 251)
            .unwrap());

        let mut wrong = device;
        wrong[1] ^= 1;
        assert!(matches!(
            database.issue_remote_user_view_permission(
                &uid,
                &wrong,
                &viewer,
                &host,
                &grant(7, 400),
                251
            ),
            Err(Error::AuthorizationChanged)
        ));
    }

    #[test]
    fn remote_team_permission_rechecks_membership_and_latest_ptk() {
        let (_temporary, mut database, uid, device) = seeded_database();
        activate_generation(&mut database, [13; 16], 1);
        let team = entity(foks_proto::ENTITY_AD_HOC_TEAM, 8);
        let host = entity(foks_proto::ENTITY_HOST, 9);
        let viewer = entity(foks_proto::ENTITY_NAMED_TEAM, 10);
        let viewer_host = entity(foks_proto::ENTITY_HOST, 11);
        let first_ptk = entity(foks_proto::ENTITY_PTK_VERIFY, 12);
        database
            .connection
            .execute(
                "INSERT INTO teams
                 (team_id, team_kind, host_id, normalized_name, team_name_utf8,
                  team_name_sequence, team_name_commitment_key, created_at)
                 VALUES (?1, 20, ?2, NULL, x'2d', 0, NULL, 1)",
                params![team, host],
            )
            .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO team_members
                 (team_id, party_id, scoped_host_id, source_role_type, source_visibility,
                  role_type, visibility, generation, verify_key, hepk_fingerprint,
                  removal_key_commitment)
                 VALUES (?1, ?2, NULL, 3, 0, 3, 0, 1, ?3, zeroblob(32), NULL)",
                params![team, uid, first_ptk],
            )
            .unwrap();
        database
            .connection
            .execute(
                "INSERT INTO team_shared_keys
                 (team_id, role_type, visibility, generation, verify_key, exact_hepk)
                 VALUES (?1, 3, 0, 1, ?2, x'01')",
                params![team, first_ptk],
            )
            .unwrap();
        let first_authority = TeamGrantAuthority {
            role_type: 3,
            visibility: 0,
            generation: 1,
            verify_key: &first_ptk,
        };

        let inserted = database
            .issue_remote_team_view_permission(
                &team,
                &device,
                &viewer,
                &viewer_host,
                &first_authority,
                &grant(13, 100),
                10,
            )
            .unwrap();
        assert!(matches!(
            inserted,
            RemoteTeamViewPermissionOutcome::Inserted(_)
        ));
        let mut competing = grant(14, 200);
        competing.key_generation = [13; 16];
        let repeated = database
            .issue_remote_team_view_permission(
                &team,
                &device,
                &viewer,
                &viewer_host,
                &first_authority,
                &competing,
                11,
            )
            .unwrap();
        let RemoteTeamViewPermissionOutcome::Existing(repeated) = repeated else {
            panic!("live team scope must be idempotent")
        };
        assert_eq!(repeated.token_hash, [13; 32]);
        activate_generation(&mut database, [18; 16], 12);
        let mut renewal = grant(18, 200);
        renewal.token_hash = [13; 32];
        let RemoteTeamViewPermissionOutcome::Renewed(renewed) = database
            .issue_remote_team_view_permission(
                &team,
                &device,
                &viewer,
                &viewer_host,
                &first_authority,
                &renewal,
                12,
            )
            .unwrap()
        else {
            panic!("same team bearer must be renewably rewrapped")
        };
        assert_eq!(renewed.token_hash, [13; 32]);
        assert_eq!(renewed.key_generation, [18; 16]);
        assert_eq!(renewed.expires_at, 200);
        let stale_generation = grant(13, 250);
        assert!(matches!(
            database.issue_remote_team_view_permission(
                &team,
                &device,
                &viewer,
                &viewer_host,
                &first_authority,
                &stale_generation,
                13,
            ),
            Err(Error::Invalid("inactive capability key generation"))
        ));

        let second_ptk = entity(foks_proto::ENTITY_PTK_VERIFY, 15);
        database
            .connection
            .execute(
                "INSERT INTO team_shared_keys
                 (team_id, role_type, visibility, generation, verify_key, exact_hepk)
                 VALUES (?1, 3, 0, 2, ?2, x'02')",
                params![team, second_ptk],
            )
            .unwrap();
        assert!(matches!(
            database.issue_remote_team_view_permission(
                &team,
                &device,
                &viewer,
                &viewer_host,
                &first_authority,
                &grant(16, 300),
                200,
            ),
            Err(Error::AuthorizationChanged)
        ));
        let second_authority = TeamGrantAuthority {
            role_type: 3,
            visibility: 0,
            generation: 2,
            verify_key: &second_ptk,
        };
        let mut reissue = grant(16, 300);
        reissue.key_generation = [18; 16];
        assert!(matches!(
            database
                .issue_remote_team_view_permission(
                    &team,
                    &device,
                    &viewer,
                    &viewer_host,
                    &second_authority,
                    &reissue,
                    200,
                )
                .unwrap(),
            RemoteTeamViewPermissionOutcome::Reissued(_)
        ));
        assert!(database
            .revoke_remote_team_view_permission(&team, &viewer, &viewer_host, 250)
            .unwrap());
        assert!(!database
            .remote_team_view_token_is_current(&[16; 32], &team, 250)
            .unwrap());

        database
            .connection
            .execute(
                "UPDATE team_members SET role_type = 1 WHERE team_id = ?1 AND party_id = ?2",
                params![team, uid],
            )
            .unwrap();
        assert!(matches!(
            database.issue_remote_team_view_permission(
                &team,
                &device,
                &viewer,
                &viewer_host,
                &second_authority,
                &grant(17, 400),
                300,
            ),
            Err(Error::AuthorizationChanged)
        ));
    }
}
