//! Invitation storage and transaction authority; no cryptographic or network I/O.
use crate::{
    error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result,
    TeamAdminAuthoritySnapshot, TeamGrantAuthority,
};
use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

pub struct InvitationActor<'a> {
    pub uid: &'a [u8],
    pub credential: &'a [u8],
}
pub struct CertificateUpload<'a> {
    pub hash: &'a [u8; 32],
    pub team: &'a [u8],
    pub generation: u64,
    pub verify_key: &'a [u8],
    pub exact_hepk: &'a [u8],
    pub exact: &'a [u8],
}
pub(crate) fn require_actor(c: &Connection, actor: &InvitationActor<'_>, now: u64) -> Result<()> {
    let active:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE uid=?1 AND active=1 AND (device_id=?2 OR subkey_id=?2))",params![actor.uid,actor.credential],|r|r.get(0))?;
    if !active {
        return Err(Error::AuthorizationChanged);
    }
    crate::sso_access::require_access(c, actor.uid, now / 1000)
}
pub(crate) fn require_admin(
    c: &Connection,
    actor: &InvitationActor<'_>,
    hash: &[u8; 32],
    now: u64,
) -> Result<TeamAdminAuthoritySnapshot> {
    require_actor(c, actor, now)?;
    let a = crate::capabilities::admin_token_query(c, hash, now, true)?
        .ok_or(Error::AuthorizationChanged)?;
    if a.holder_id != actor.uid {
        return Err(Error::AuthorizationChanged);
    }
    Ok(a)
}
fn lookup(c: &Connection, hash: &[u8; 32]) -> Result<Option<Vec<u8>>> {
    Ok(c.query_row(
        "SELECT exact_certificate FROM team_invitation_certificates WHERE certificate_hash=?1",
        [hash],
        |r| r.get(0),
    )
    .optional()?)
}
fn certificates(c: &Connection, team: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut q=c.prepare("SELECT exact_certificate FROM team_invitation_certificates WHERE team_id=?1 AND generation=(SELECT max(generation) FROM team_shared_keys WHERE team_id=?1 AND role_type=2 AND visibility=0) ORDER BY certificate_hash LIMIT 64")?;
    let rows = q
        .query_map([team], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
fn local_user_permission(c: &Connection, viewer: &[u8], target: &[u8]) -> Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM user_local_view_permissions WHERE viewer_uid=?1 AND target_id=?2)",params![viewer,target],|r|r.get(0))?)
}
macro_rules! invitation_reads {
    () => {
        pub fn invitation_certificate(&self, hash: &[u8; 32]) -> Result<Option<Vec<u8>>> {
            lookup(&self.connection, hash)
        }
        pub fn current_invitation_certificates(&self, team: &[u8]) -> Result<Vec<Vec<u8>>> {
            certificates(&self.connection, team)
        }
        pub fn local_user_view_permission(&self, viewer: &[u8], target: &[u8]) -> Result<bool> {
            local_user_permission(&self.connection, viewer, target)
        }
    };
}
impl Database {
    invitation_reads!();
}
impl ReadDatabase {
    invitation_reads!();
}
impl ReadSnapshot<'_> {
    pub fn invitation_certificate(&self, hash: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        lookup(self.connection(), hash)
    }
    pub fn current_invitation_certificates(&self, team: &[u8]) -> Result<Vec<Vec<u8>>> {
        certificates(self.connection(), team)
    }
    pub fn local_user_view_permission(&self, viewer: &[u8], target: &[u8]) -> Result<bool> {
        local_user_permission(self.connection(), viewer, target)
    }
}
impl Database {
    pub fn put_invitation_certificate(
        &mut self,
        actor: InvitationActor<'_>,
        admin_hash: &[u8; 32],
        upload: CertificateUpload<'_>,
        now: u64,
    ) -> Result<()> {
        if upload.exact.len() > 16384 {
            return Err(Error::Invalid("certificate size"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let admin = require_admin(&tx, &actor, admin_hash, now)?;
        if admin.team_id != upload.team {
            return Err(Error::AuthorizationChanged);
        }
        let matches:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM team_shared_keys WHERE team_id=?1 AND role_type=2 AND visibility=0 AND generation=?2 AND verify_key=?3 AND exact_hepk=?4)",params![upload.team,sql_integer(upload.generation)?,upload.verify_key,upload.exact_hepk],|r|r.get(0))?;
        if !matches {
            return Err(Error::AuthorizationChanged);
        }
        if let Some(exact) = lookup(&tx, upload.hash)? {
            if exact != upload.exact {
                return Err(Error::ReceiptConflict);
            }
            tx.commit()?;
            return Ok(());
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM team_invitation_certificates WHERE team_id=?1 AND generation=?2",
            params![upload.team, sql_integer(upload.generation)?],
            |r| r.get(0),
        )?;
        let global: i64 = tx.query_row(
            "SELECT count(*) FROM team_invitation_certificates",
            [],
            |r| r.get(0),
        )?;
        let total_team: i64 = tx.query_row(
            "SELECT count(*) FROM team_invitation_certificates WHERE team_id=?1",
            [upload.team],
            |r| r.get(0),
        )?;
        if count >= 64 || total_team >= 4096 || global >= 65536 {
            return Err(Error::QuotaExceeded);
        }
        tx.execute(
            "INSERT INTO team_invitation_certificates VALUES(?1,?2,?3,?4,?5)",
            params![
                upload.hash,
                upload.team,
                sql_integer(upload.generation)?,
                upload.exact,
                sql_integer(now)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn grant_local_view(
        &mut self,
        actor: InvitationActor<'_>,
        payload: &foks_proto::LocalViewPermissionPayload,
        authority: Option<&TeamGrantAuthority<'_>>,
        now: u64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_actor(&tx, &actor, now)?;
        match authority {
            None if payload.viewee.as_bytes() == actor.uid => {}
            Some(a) => {
                let matches:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM team_shared_keys k WHERE team_id=?1 AND role_type=?2 AND visibility=?3 AND generation=?4 AND verify_key=?5 AND generation=(SELECT max(generation) FROM team_shared_keys WHERE team_id=k.team_id AND role_type=k.role_type AND visibility=k.visibility))",params![payload.viewee.as_bytes(),sql_integer(a.role_type)?,a.visibility,sql_integer(a.generation)?,a.verify_key],|r|r.get(0))?;
                if !matches {
                    return Err(Error::AuthorizationChanged);
                }
            }
            _ => return Err(Error::AuthorizationChanged),
        }
        insert_local_permission(&tx, payload)?;
        tx.commit()?;
        Ok(())
    }
}
pub(crate) fn insert_local_permission(
    c: &Connection,
    p: &foks_proto::LocalViewPermissionPayload,
) -> Result<()> {
    let viewer = p.viewer.as_bytes();
    let target = p.viewee.as_bytes();
    let role = p.viewer_role.unwrap_or(foks_proto::Role::ADMIN);
    if role == foks_proto::Role::NONE {
        return Err(Error::Invalid("local viewer role"));
    }
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE uid=?1 UNION ALL SELECT 1 FROM teams WHERE team_id=?1)",[target],|r|r.get(0))?;
    if !exists {
        return Err(Error::AuthorizationChanged);
    }
    let viewer_exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE uid=?1 UNION ALL SELECT 1 FROM teams WHERE team_id=?1)",[viewer],|r|r.get(0))?;
    if !viewer_exists {
        return Err(Error::AuthorizationChanged);
    }
    let existing:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM user_local_view_permissions WHERE viewer_uid=?1 AND target_id=?2 UNION ALL SELECT 1 FROM team_local_view_permissions WHERE team_id=?1 AND target_id=?2)",params![viewer,target],|r|r.get(0))?;
    if !existing {
        let total:i64=c.query_row("SELECT (SELECT count(*) FROM user_local_view_permissions)+(SELECT count(*) FROM team_local_view_permissions)",[],|r|r.get(0))?;
        if total >= 1_000_000 {
            return Err(Error::QuotaExceeded);
        }
    }
    if p.viewer.entity_type() == foks_proto::ENTITY_USER {
        let exists = local_user_permission(c, viewer, target)?;
        let count: i64 = c.query_row(
            "SELECT count(*) FROM user_local_view_permissions WHERE target_id=?1",
            [target],
            |r| r.get(0),
        )?;
        if !exists && count >= 1024 {
            return Err(Error::QuotaExceeded);
        }
        c.execute(
            "INSERT INTO user_local_view_permissions VALUES(?1,?2) ON CONFLICT DO NOTHING",
            params![viewer, target],
        )?;
    } else {
        let existing:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM team_local_view_permissions WHERE team_id=?1 AND target_id=?2)",params![viewer,target],|r|r.get(0))?;
        let count: i64 = c.query_row(
            "SELECT count(*) FROM team_local_view_permissions WHERE target_id=?1",
            [target],
            |r| r.get(0),
        )?;
        if !existing && count >= 1024 {
            return Err(Error::QuotaExceeded);
        }
        c.execute("INSERT INTO team_local_view_permissions VALUES(?1,?2,?3,?4) ON CONFLICT(team_id,target_id) DO UPDATE SET minimum_role_type=excluded.minimum_role_type,minimum_role_visibility=excluded.minimum_role_visibility",params![viewer,target,sql_integer(role.protocol_value())?,i64::from(role.visibility().unwrap_or_default())])?;
    }
    Ok(())
}
