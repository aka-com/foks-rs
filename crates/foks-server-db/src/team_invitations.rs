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

/// Prepared local acceptance, rechecked together with the optional Requested link.
/// Opaque tokens deliberately have no Debug representation.
pub struct LocalInvitationAdmission {
    pub uid: Vec<u8>,
    pub credential: Vec<u8>,
    pub certificate_hash: [u8; 32],
    pub destination: Vec<u8>,
    pub joiner: foks_proto::EntityId,
    pub source_role: foks_proto::Role,
    pub source_admin: Option<[u8; 32]>,
    pub source_head: Option<[u8; 32]>,
    pub destination_head: Option<[u8; 32]>,
    pub receipt: [u8; 17],
    pub permission: [u8; 17],
}
pub(crate) fn insert_local_admission(
    c: &Connection,
    a: &LocalInvitationAdmission,
    now: u64,
) -> Result<()> {
    let actor = InvitationActor {
        uid: &a.uid,
        credential: &a.credential,
    };
    require_actor(c, &actor, now)?;
    let destination: Option<Vec<u8>> = c
        .query_row(
            "SELECT team_id FROM team_invitation_certificates WHERE certificate_hash=?1",
            [a.certificate_hash],
            |r| r.get(0),
        )
        .optional()?;
    if destination.as_deref() != Some(a.destination.as_slice())
        || a.receipt[0] != 57
        || a.permission[0] != 54
    {
        return Err(Error::AuthorizationChanged);
    }
    let role = a.source_role.protocol_value();
    let visibility = i64::from(a.source_role.visibility().unwrap_or_default());
    if role == 0 {
        return Err(Error::Invalid("joiner source role"));
    }
    if let Some(token) = a.source_admin {
        let authority = require_admin(c, &actor, &token, now)?;
        if authority.team_id != a.joiner.as_bytes() {
            return Err(Error::AuthorizationChanged);
        }
        let selected: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM team_shared_keys WHERE team_id=?1 AND role_type=?2 AND visibility=?3)",params![a.joiner.as_bytes(),sql_integer(role)?,visibility],|r|r.get(0))?;
        if !selected {
            return Err(Error::AuthorizationChanged);
        }
        for (team, expected) in [
            (a.joiner.as_bytes(), a.source_head),
            (a.destination.as_slice(), a.destination_head),
        ] {
            let head: Vec<u8> = c.query_row(
                "SELECT link_hash FROM team_chain_heads WHERE team_id=?1",
                [team],
                |r| r.get(0),
            )?;
            if expected.as_ref().map(|h| h.as_slice()) != Some(head.as_slice()) {
                return Err(Error::AuthorizationChanged);
            }
        }
    } else if a.joiner.as_bytes() != a.uid || a.source_role != foks_proto::Role::OWNER {
        return Err(Error::AuthorizationChanged);
    }
    let pending:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM team_local_join_requests WHERE team_id=?1 AND joiner_id=?2 AND source_role_type=?3 AND source_visibility=?4 AND state=0)",params![a.destination,a.joiner.as_bytes(),sql_integer(role)?,visibility],|r|r.get(0))?;
    if pending {
        return Err(Error::InvitationAlreadyPending);
    }
    let team_count: i64 = c.query_row(
        "SELECT (SELECT count(*) FROM team_local_join_requests WHERE team_id=?1 AND state=0)+(SELECT count(*) FROM team_remote_join_requests WHERE team_id=?1 AND state=0)",
        [&a.destination],
        |r| r.get(0),
    )?;
    let joiner_count: i64 = c.query_row(
        "SELECT count(*) FROM team_local_join_requests WHERE joiner_id=?1 AND state=0",
        [a.joiner.as_bytes()],
        |r| r.get(0),
    )?;
    let total: i64 = c.query_row("SELECT count(*) FROM team_local_join_requests", [], |r| {
        r.get(0)
    })?;
    if team_count >= 1000 || joiner_count >= 100 || total >= 1_000_000 {
        return Err(Error::QuotaExceeded);
    }
    let (floor, visibility): (i64, i64) = c.query_row(
        "SELECT member_load_floor_type,member_load_floor_visibility FROM teams WHERE team_id=?1",
        [&a.destination],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let floor = stored_role(crate::error::unsigned(floor)?, visibility)?;
    insert_local_permission(
        c,
        &foks_proto::LocalViewPermissionPayload {
            viewee: a.joiner.clone(),
            viewer: foks_proto::EntityId::from_bytes(a.destination.clone())
                .map_err(|_| Error::Invalid("destination team"))?,
            time: now / 1000,
            viewer_role: Some(floor),
        },
    )?;
    c.execute("INSERT INTO team_local_join_requests(receipt,team_id,joiner_id,source_role_type,source_visibility,state,permission,created_ms) VALUES(?1,?2,?3,?4,?5,0,?6,?7)",params![a.receipt,a.destination,a.joiner.as_bytes(),sql_integer(role)?,i64::from(a.source_role.visibility().unwrap_or_default()),a.permission,sql_integer(now/1000)?])?;
    Ok(())
}
fn stored_role(kind: u64, visibility: i64) -> Result<foks_proto::Role> {
    match kind {
        1 => Ok(foks_proto::Role::member(
            visibility
                .try_into()
                .map_err(|_| Error::Invalid("role visibility"))?,
        )),
        2 if visibility == 0 => Ok(foks_proto::Role::ADMIN),
        3 if visibility == 0 => Ok(foks_proto::Role::OWNER),
        _ => Err(Error::Invalid("source role")),
    }
}
impl Database {
    pub fn accept_local_invitation(
        &mut self,
        admission: &LocalInvitationAdmission,
        now: u64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_local_admission(&tx, admission, now)?;
        tx.commit()?;
        Ok(())
    }
    pub fn reject_local_invitation(
        &mut self,
        actor: InvitationActor<'_>,
        admin_hash: &[u8; 32],
        receipt: &[u8; 17],
        now: u64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let authority = require_admin(&tx, &actor, admin_hash, now)?;
        let state: Option<i64> = tx
            .query_row(
                "SELECT state FROM team_local_join_requests WHERE receipt=?1 AND team_id=?2",
                params![receipt, authority.team_id],
                |r| r.get(0),
            )
            .optional()?;
        match state {
            Some(0) => {
                tx.execute("UPDATE team_local_join_requests SET state=2,decision_ms=?2 WHERE receipt=?1 AND state=0",params![receipt,sql_integer(now/1000)?])?;
            }
            Some(2) => {}
            Some(_) => return Err(Error::InvitationDecisionConflict),
            None => return Err(Error::AuthorizationChanged),
        }
        tx.commit()?;
        Ok(())
    }
}
fn local_inbox(
    c: &Connection,
    team: &[u8],
    p: foks_proto::InboxPagination,
) -> Result<Vec<foks_proto::RawInboxRow>> {
    let limit = if p.limit == 0 { 100 } else { p.limit.min(1000) };
    let mut q=c.prepare("SELECT receipt,joiner_id,source_role_type,source_visibility,permission,created_ms FROM team_local_join_requests WHERE team_id=?1 AND state=0 AND (?2=0 OR created_ms>=?2) AND (?3=0 OR created_ms<=?3) ORDER BY created_ms DESC,receipt LIMIT ?4")?;
    let rows = q
        .query_map(
            params![
                team,
                sql_integer(p.start)?,
                sql_integer(p.end)?,
                sql_integer(limit)?
            ],
            |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(receipt, joiner, role, visibility, permission, time)| {
            Ok(foks_proto::RawInboxRow {
                time: crate::error::unsigned(time)?,
                state: foks_proto::JoinRequestState::Pending,
                receipt: foks_proto::TeamRsvp::new(
                    receipt.try_into().map_err(|_| Error::Invalid("receipt"))?,
                )
                .map_err(|_| Error::Invalid("receipt"))?,
                request: foks_proto::RawInboxRequest::Local {
                    joiner: foks_proto::EntityId::from_bytes(joiner)
                        .map_err(|_| Error::Invalid("joiner"))?,
                    source_role: stored_role(crate::error::unsigned(role)?, visibility)?,
                    permission: foks_proto::PermissionToken::new(
                        permission
                            .try_into()
                            .map_err(|_| Error::Invalid("permission"))?,
                    ),
                },
            })
        })
        .collect()
}
impl ReadSnapshot<'_> {
    pub fn local_invitation_inbox(
        &self,
        team: &[u8],
        pagination: foks_proto::InboxPagination,
    ) -> Result<Vec<foks_proto::RawInboxRow>> {
        local_inbox(self.connection(), team, pagination)
    }
}

/// Called before replacing the roster, so metadata-only edits cannot approve
/// an existing member's self-invite. The chain remains admission authority.
pub(crate) fn approve_local_additions(
    c: &Connection,
    m: &crate::TeamMutation<'_>,
) -> Result<Vec<(foks_proto::EntityId, foks_proto::Role)>> {
    let mut approved = vec![];
    let host: Vec<u8> = c.query_row(
        "SELECT host_id FROM teams WHERE team_id=?1",
        [m.team_id],
        |r| r.get(0),
    )?;
    for member in m.members {
        if member.scoped_host_id.is_some_and(|h| h != host) {
            continue;
        }
        let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM team_members WHERE team_id=?1 AND party_id=?2 AND scoped_host_id IS NULL AND source_role_type=?3 AND source_visibility=?4)",params![m.team_id,member.party_id,sql_integer(member.source_role_type)?,member.source_visibility],|r|r.get(0))?;
        if present {
            continue;
        }
        let changed=c.execute("UPDATE team_local_join_requests SET state=1,decision_ms=?5,decision_sequence=?6,decision_link_hash=?7 WHERE team_id=?1 AND joiner_id=?2 AND source_role_type=?3 AND source_visibility=?4 AND state=0",params![m.team_id,member.party_id,sql_integer(member.source_role_type)?,member.source_visibility,sql_integer(m.now/1000)?,sql_integer(m.expected_sequence)?,m.link_hash])?;
        if changed > 0 {
            approved.push((
                foks_proto::EntityId::from_bytes(member.party_id.to_vec())
                    .map_err(|_| Error::Invalid("invitee party"))?,
                stored_role(member.source_role_type, member.source_visibility)?,
            ));
        }
    }
    Ok(approved)
}

/// Public guest admission owns only ciphertext and validated public witnesses.
pub struct RemoteInvitationAdmission {
    pub certificate_hash: [u8; 32],
    pub team: Vec<u8>,
    pub generation: u64,
    pub exact_hepk: Vec<u8>,
    pub exact_request: Vec<u8>,
    pub receipt: [u8; 17],
    pub destination_head: [u8; 32],
}
impl Database {
    pub fn accept_remote_invitation(
        &mut self,
        a: &RemoteInvitationAdmission,
        now: u64,
    ) -> Result<()> {
        if a.exact_request.len() > 16384 || a.receipt[0] != 56 {
            return Err(Error::Invalid("remote invitation shape"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM team_invitation_certificates c JOIN team_shared_keys k ON k.team_id=c.team_id AND k.generation=c.generation WHERE c.certificate_hash=?1 AND c.team_id=?2 AND c.generation=?3 AND k.role_type=2 AND k.visibility=0 AND k.exact_hepk=?4)",params![a.certificate_hash.as_slice(),a.team,sql_integer(a.generation)?,a.exact_hepk],|r|r.get(0))?;
        if !valid {
            return Err(Error::AuthorizationChanged);
        }
        let head: Vec<u8> = tx.query_row(
            "SELECT link_hash FROM team_chain_heads WHERE team_id=?1",
            [&a.team],
            |r| r.get(0),
        )?;
        if head != a.destination_head {
            return Err(Error::AuthorizationChanged);
        }
        let pending:i64=tx.query_row("SELECT (SELECT count(*) FROM team_remote_join_requests WHERE team_id=?1 AND state=0)+(SELECT count(*) FROM team_local_join_requests WHERE team_id=?1 AND state=0)",[&a.team],|r|r.get(0))?;
        let total: i64 =
            tx.query_row("SELECT count(*) FROM team_remote_join_requests", [], |r| {
                r.get(0)
            })?;
        if pending >= 1000 || total >= 65536 {
            return Err(Error::QuotaExceeded);
        }
        // No ciphertext deduplication: Go issues a new RSVP for every successful send.
        tx.execute("INSERT INTO team_remote_join_requests(receipt,team_id,certificate_hash,exact_request,state,created_ms) VALUES(?1,?2,?3,?4,0,?5)",params![a.receipt.as_slice(),a.team,a.certificate_hash.as_slice(),a.exact_request,sql_integer(now/1000)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn reject_remote_invitation(
        &mut self,
        actor: InvitationActor<'_>,
        admin_hash: &[u8; 32],
        receipt: &[u8; 17],
        now: u64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let a = require_admin(&tx, &actor, admin_hash, now)?;
        let state: Option<i64> = tx
            .query_row(
                "SELECT state FROM team_remote_join_requests WHERE team_id=?1 AND receipt=?2",
                params![a.team_id, receipt.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        match state {
            Some(0) => {
                tx.execute("UPDATE team_remote_join_requests SET state=2,decision_ms=?3 WHERE team_id=?1 AND receipt=?2 AND state=0",params![a.team_id,receipt.as_slice(),sql_integer(now/1000)?])?;
            }
            Some(2) => {}
            Some(_) => return Err(Error::InvitationDecisionConflict),
            None => return Err(Error::AuthorizationChanged),
        }
        tx.commit()?;
        Ok(())
    }
}
impl ReadSnapshot<'_> {
    pub fn remote_invitation_request(
        &self,
        team: &[u8],
        receipt: &[u8; 17],
    ) -> Result<Option<Vec<u8>>> {
        Ok(self.connection().query_row("SELECT exact_request FROM team_remote_join_requests WHERE team_id=?1 AND receipt=?2 AND state=0",params![team,receipt.as_slice()],|r|r.get(0)).optional()?)
    }
    pub fn remote_invitation_inbox(
        &self,
        team: &[u8],
        p: foks_proto::InboxPagination,
    ) -> Result<Vec<foks_proto::RawInboxRow>> {
        let limit = if p.limit == 0 { 100 } else { p.limit.min(1000) };
        let mut q=self.connection().prepare("SELECT receipt,exact_request,created_ms FROM team_remote_join_requests WHERE team_id=?1 AND state=0 AND (?2=0 OR created_ms>=?2) AND (?3=0 OR created_ms<=?3) ORDER BY created_ms DESC,receipt LIMIT ?4")?;
        let rows = q
            .query_map(
                params![
                    team,
                    sql_integer(p.start)?,
                    sql_integer(p.end)?,
                    sql_integer(limit)?
                ],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(receipt, request, time)| {
                Ok(foks_proto::RawInboxRow {
                    time: crate::error::unsigned(time)?,
                    state: foks_proto::JoinRequestState::Pending,
                    receipt: foks_proto::TeamRsvp::new(
                        receipt
                            .try_into()
                            .map_err(|_| Error::Invalid("remote receipt"))?,
                    )
                    .map_err(|_| Error::Invalid("remote receipt"))?,
                    request: foks_proto::RawInboxRequest::Remote(
                        foks_proto::RemoteJoinRequest::decode(&request)
                            .map_err(|_| Error::Invalid("remote request"))?,
                    ),
                })
            })
            .collect()
    }
}
/// Bind remote request decisions to the RSVP already carried by Go editTeam.
/// Independent federation edits may have an RSVP with no inbox row.
pub(crate) fn approve_remote_invitation(
    c: &Connection,
    team: &[u8],
    receipt: &[u8; 17],
    sequence: u64,
    hash: &[u8],
    now: u64,
) -> Result<()> {
    let state: Option<i64> = c
        .query_row(
            "SELECT state FROM team_remote_join_requests WHERE team_id=?1 AND receipt=?2",
            params![team, receipt.as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    match state {
        None => Ok(()),
        Some(0) => {
            c.execute("UPDATE team_remote_join_requests SET state=1,decision_ms=?3,decision_sequence=?4,decision_link_hash=?5 WHERE team_id=?1 AND receipt=?2 AND state=0",params![team,receipt.as_slice(),sql_integer(now/1000)?,sql_integer(sequence)?,hash])?;
            Ok(())
        }
        _ => Err(Error::InvitationDecisionConflict),
    }
}

impl Database {
    /// Deliver a proof for an already committed removal. Its secret MAC is
    /// checked by the recipient; server authority is the team-scoped commitment.
    pub fn post_team_removal(
        &mut self,
        actor: InvitationActor<'_>,
        token: &[u8; 32],
        removal: &foks_proto::TeamRemovalAndCommitment,
        now: u64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let admin = require_admin(&tx, &actor, token, now)?;
        let p = &removal.removal.payload;
        if admin.team_id != p.team.as_bytes() {
            return Err(Error::AuthorizationChanged);
        }
        let host: Vec<u8> = tx.query_row(
            "SELECT host_id FROM teams WHERE team_id=?1",
            [&admin.team_id],
            |r| r.get(0),
        )?;
        let prior: Option<Vec<u8>> = tx
            .query_row(
                "SELECT exact_removal FROM team_removal_proofs WHERE team_id=?1 AND commitment=?2",
                params![admin.team_id, removal.commitment.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let prior = prior.ok_or(Error::AuthorizationChanged)?;
        let prior = foks_proto::TeamRemovalProof::decode(&prior)
            .map_err(|_| Error::AuthorizationChanged)?;
        if host != p.host.as_bytes()
            || p.admin_host.as_bytes() != host
            || (p.admin.as_bytes() != actor.uid && p.admin != prior.payload.admin)
        {
            return Err(Error::AuthorizationChanged);
        }
        let (kind, visibility) = (
            p.source_role.protocol_value(),
            p.source_role.visibility().unwrap_or(0),
        );
        // This row is inserted atomically with the signed removal and captures
        // its original key box even after the member has rejoined.
        let changed = tx.execute(
            "UPDATE team_removal_proofs SET exact_removal=?7
            WHERE team_id=?1 AND commitment=?2 AND member_id=?3 AND member_host_id=?4
            AND source_role_type=?5 AND source_visibility=?6",
            params![
                admin.team_id,
                removal.commitment.as_slice(),
                p.member.as_bytes(),
                p.member_host.as_bytes(),
                sql_integer(kind)?,
                visibility,
                removal
                    .removal
                    .encoded()
                    .map_err(|_| Error::AuthorizationChanged)?
            ],
        )?;
        if changed != 1 {
            return Err(Error::AuthorizationChanged);
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn member_identity_includes_host_and_source_role() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("scope.db"), crate::Config::default()).unwrap();
        let team = [3u8; 33];
        db.connection.execute("INSERT INTO team_names(normalized_name,reservation_sequence,team_id) VALUES (?1,1,?1)", params![team.as_slice()]).unwrap();
        db.connection.execute("INSERT INTO teams(team_id,team_kind,host_id,normalized_name,team_name_utf8,team_name_sequence,team_name_commitment_key,member_load_floor_type,member_load_floor_visibility,created_at) VALUES (?1,3,zeroblob(33),?1,?1,1,zeroblob(16),1,0,0)", params![team.as_slice()]).unwrap();
        let insert = |host: Option<&[u8]>, source: i64| {
            db.connection.execute(
            "INSERT INTO team_members(team_id,party_id,scoped_host_id,source_role_type,source_visibility,role_type,visibility,generation,verify_key,hepk_fingerprint) VALUES (?1,zeroblob(33),?2,?3,0,1,0,1,zeroblob(33),zeroblob(32))",
            params![team.as_slice(), host, source])
        };
        assert!(insert(None, 3).is_ok());
        assert!(insert(None, 3).is_err());
        assert!(insert(Some(&[1; 33]), 3).is_ok());
        assert!(insert(Some(&[2; 33]), 3).is_ok());
        assert!(insert(Some(&[1; 33]), 2).is_ok());
        assert!(insert(Some(&[1; 33]), 3).is_err());
    }
}
