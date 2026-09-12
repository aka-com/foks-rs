//! Browser authority is checked in the transaction, never from cookie fields or cached grants.
use crate::error::{sql_integer, unsigned};
use crate::{
    Database, Error, InvitePolicy, InviteRegime, InviteSnapshot, ReadDatabase, ReadSnapshot,
    Result, SsoAuthorizationStamp,
};
use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

pub const WEB_TICKET_US: u64 = 60_000_000;
pub const WEB_SESSION_US: u64 = 300_000_000;
#[derive(Clone, Copy, Debug)]
pub struct AdminMoment {
    pub epoch: [u8; 16],
    pub utc_us: u64,
    pub elapsed_us: u64,
}
#[derive(Clone)]
pub struct WebCredential {
    pub host: [u8; 33],
    pub uid: [u8; 33],
    pub credential: [u8; 33],
    pub certificate_expires_at_us: u64,
}
#[derive(Clone)]
pub struct WebContext {
    pub record_id: [u8; 16],
    pub credential: WebCredential,
    pub username: String,
    pub host_name: String,
    pub operator: bool,
    pub deadline_elapsed_us: u64,
}
#[derive(Clone, Debug)]
pub struct HostAdminGrant {
    pub host: [u8; 33],
    pub uid: [u8; 33],
    pub username: String,
    pub revision: u64,
    pub active: bool,
    pub reason: String,
}
#[derive(Clone, Debug)]
pub struct WebSessionMetadata {
    pub id: [u8; 16],
    pub uid: [u8; 33],
    pub credential: [u8; 33],
    pub created_at_us: u64,
    pub expires_at_us: u64,
    pub revoked: bool,
}
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum AdminAction {
    Grant = 1,
    RevokeGrant = 2,
    Ticket = 3,
    Login = 4,
    RevokeSession = 5,
    RevokeAll = 6,
    IssueInvite = 7,
    DisableInvite = 8,
    SignupPolicy = 9,
}
#[derive(Clone, Debug)]
pub struct AdminAuditEvent {
    pub id: u64,
    pub actor_uid: Option<[u8; 33]>,
    pub action: u8,
    pub target: Vec<u8>,
    pub revision: Option<u64>,
    pub occurred_at_us: u64,
}
#[derive(Clone)]
pub struct WebMutationAuth {
    pub session_hash: [u8; 32],
    pub csrf_hash: [u8; 32],
}
/// Distinct credential domains are named at redemption call sites.
pub struct WebRedemption<'a> {
    pub binding: &'a [u8; 32],
    pub csrf: &'a [u8; 32],
    pub session: &'a [u8; 32],
    pub session_csrf: &'a [u8; 32],
    pub id: &'a [u8; 16],
    pub existing: Option<&'a [u8; 32]>,
}
#[derive(Clone, Copy)]
pub struct WebInvite {
    pub nonce_hash: [u8; 32],
    pub id: [u8; 16],
    pub code_hash: [u8; 32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebInviteResult {
    Created([u8; 16]),
    AlreadyCreated([u8; 16]),
}
#[derive(Default, Clone, Copy, Debug)]
pub struct WebCleanup {
    pub tickets: u64,
    pub confirmations: u64,
    pub sessions: u64,
    pub nonces: u64,
    pub audit: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct WebOverview {
    pub accounts: u64,
    pub operators: u64,
    pub invites: u64,
    pub sessions: u64,
}
struct Authority {
    id: [u8; 16],
    credential: WebCredential,
    epoch: [u8; 16],
    stamp: SsoAuthorizationStamp,
    expires: u64,
    deadline: u64,
    usable: bool,
    csrf: Option<[u8; 32]>,
}
fn local_user(c: &Connection, host: &[u8; 33], uid: &[u8; 33]) -> Result<(String, String)> {
    if host[0] != foks_proto::ENTITY_HOST || uid[0] != foks_proto::ENTITY_USER {
        return Err(Error::AuthorizationChanged);
    }
    c.query_row("SELECT h.canonical_name,u.username_utf8 FROM host_metadata h CROSS JOIN users u WHERE h.singleton=1
             AND h.host_id=?1 AND u.uid=?2",params![host,uid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?))).optional()?.map(|(h,u)|Ok::<_,Error>((h,String::from_utf8(u).map_err(|_|Error::Invalid("stored username"))?))).transpose()?.ok_or(Error::AuthorizationChanged)
}
fn credential(
    c: &Connection,
    v: &WebCredential,
    now: AdminMoment,
) -> Result<(String, String, SsoAuthorizationStamp)> {
    if v.certificate_expires_at_us <= now.utc_us
        || ![foks_proto::ENTITY_DEVICE, foks_proto::ENTITY_SUBKEY].contains(&v.credential[0])
    {
        return Err(Error::AuthorizationChanged);
    }
    let parent = crate::certificates::active_credential_owner(c, &v.uid, &v.credential)?
        .ok_or(Error::AuthorizationChanged)?;
    if ![foks_proto::ENTITY_DEVICE, foks_proto::ENTITY_YUBI].contains(&parent[0]) {
        return Err(Error::AuthorizationChanged);
    }
    let (host, name) = local_user(c, &v.host, &v.uid)?;
    let stamp = crate::sso_policy::authorization_stamp(c, &v.uid, now.utc_us / 1000)?;
    if matches!(&stamp,SsoAuthorizationStamp::Linked {host,..} if host!=&v.host) {
        return Err(Error::AuthorizationChanged);
    }
    Ok((host, name, stamp))
}
fn grant_active(c: &Connection, host: &[u8; 33], uid: &[u8; 33]) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM host_admin_grants WHERE host_id=?1 AND uid=?2 AND active=1)",
        params![host, uid],
        |r| r.get(0),
    )?)
}
fn load(c: &Connection, hash: &[u8; 32], session: bool) -> Result<Authority> {
    let sql = if session {
        "SELECT
             record_id,host_id,uid,credential_id,certificate_expires_at_us,instance_epoch,sso_config_hash,sso_policy_epoch,sso_authorization_generation,expires_at_us,deadline_elapsed_us,revoked_at_us
             IS NULL,csrf_hash FROM web_admin_sessions WHERE session_hash=?1"
    } else {
        "SELECT
             record_id,host_id,uid,credential_id,certificate_expires_at_us,instance_epoch,sso_config_hash,sso_policy_epoch,sso_authorization_generation,expires_at_us,deadline_elapsed_us,state=0,NULL
             FROM web_login_tickets WHERE ticket_hash=?1"
    };
    c.query_row(sql, [hash], |r| {
        let host = r.get(1)?;
        let config: Option<[u8; 32]> = r.get(6)?;
        let stamp = match config {
            None => SsoAuthorizationStamp::Unconfigured,
            Some(config_hash) => SsoAuthorizationStamp::Linked {
                host,
                config_hash,
                policy_epoch: r.get::<_, i64>(7)? as u64,
                account_generation: r.get::<_, i64>(8)? as u64,
            },
        };
        Ok(Authority {
            id: r.get(0)?,
            credential: WebCredential {
                host,
                uid: r.get(2)?,
                credential: r.get(3)?,
                certificate_expires_at_us: r.get::<_, i64>(4)? as u64,
            },
            epoch: r.get(5)?,
            stamp,
            expires: r.get::<_, i64>(9)? as u64,
            deadline: r.get::<_, i64>(10)? as u64,
            usable: r.get(11)?,
            csrf: r.get(12)?,
        })
    })
    .optional()?
    .ok_or(Error::ReceiptExpired)
}
fn authorize(c: &Connection, a: &Authority, now: AdminMoment) -> Result<WebContext> {
    if !a.usable || a.epoch != now.epoch || a.expires <= now.utc_us || a.deadline <= now.elapsed_us
    {
        return Err(Error::ReceiptExpired);
    }
    let (host_name, username, stamp) = credential(c, &a.credential, now)?;
    if stamp != a.stamp {
        return Err(Error::AuthorizationChanged);
    }
    Ok(WebContext {
        record_id: a.id,
        credential: a.credential.clone(),
        username,
        host_name,
        operator: grant_active(c, &a.credential.host, &a.credential.uid)?,
        deadline_elapsed_us: a.deadline,
    })
}
fn mutation(
    c: &Connection,
    auth: &WebMutationAuth,
    now: AdminMoment,
    operator: bool,
) -> Result<WebContext> {
    let a = load(c, &auth.session_hash, true)?;
    // Fixed-size cryptographic verification hashes are compared in constant time.
    if !a.csrf.is_some_and(|v| constant_eq(&v, &auth.csrf_hash)) {
        return Err(Error::AuthorizationChanged);
    }
    let ctx = authorize(c, &a, now)?;
    if operator && !ctx.operator {
        return Err(Error::AuthorizationChanged);
    }
    Ok(ctx)
}
fn constant_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(a.ct_eq(b))
}
fn stamp_columns(s: &SsoAuthorizationStamp) -> (Option<[u8; 32]>, Option<i64>, Option<i64>) {
    match s {
        SsoAuthorizationStamp::Unconfigured => (None, None, None),
        SsoAuthorizationStamp::Linked {
            config_hash,
            policy_epoch,
            account_generation,
            ..
        } => (
            Some(*config_hash),
            Some(*policy_epoch as i64),
            Some(*account_generation as i64),
        ),
    }
}
fn audit(
    c: &Connection,
    host: &[u8; 33],
    actor: Option<&WebCredential>,
    action: AdminAction,
    target: &[u8],
    revision: Option<u64>,
    now: u64,
) -> Result<()> {
    c.execute("DELETE FROM admin_audit WHERE event_id IN (SELECT event_id FROM admin_audit WHERE occurred_at_us<=?1
             ORDER BY occurred_at_us,event_id LIMIT 128)",[sql_integer(now.saturating_sub(90*24*60*60*1_000_000))?])?;
    let count: i64 = c.query_row("SELECT count(*) FROM admin_audit", [], |r| r.get(0))?;
    if count >= 100_000 {
        c.execute("DELETE FROM admin_audit WHERE event_id IN (SELECT event_id FROM admin_audit ORDER BY event_id LIMIT 128)",[])?;
    }
    c.execute("INSERT INTO
             admin_audit(host_id,actor_uid,actor_credential_id,action,target_id,resource_revision,occurred_at_us)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",params![host,actor.map(|a|a.uid),actor.map(|a|a.credential),action as u8,target,revision.map(sql_integer).transpose()?,sql_integer(now)?])?;
    Ok(())
}
fn capacity(c: &Connection, table: &str, uid: &[u8; 33], per_uid: i64, total: i64) -> Result<()> {
    // Table names are closed internal constants, never caller input.
    let (all, own): (i64, i64) = c.query_row(
        &format!("SELECT count(*),coalesce(sum(uid=?1),0) FROM {table}"),
        [uid],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if all >= total || own >= per_uid {
        return Err(Error::Capacity("browser records"));
    }
    Ok(())
}

fn confirmation(
    c: &Connection,
    binding: &[u8; 32],
    now: AdminMoment,
) -> Result<([u8; 32], [u8; 32])> {
    c.query_row(
        "SELECT ticket_hash,csrf_hash FROM web_login_confirmations WHERE binding_hash=?1 AND
             instance_epoch=?2 AND expires_at_us>?3 AND deadline_elapsed_us>?4",
        params![
            binding,
            now.epoch,
            sql_integer(now.utc_us)?,
            sql_integer(now.elapsed_us)?
        ],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .ok_or(Error::ReceiptExpired)
}

mod cleanup;
mod grants;
mod invites;
mod read;
mod sessions;
use cleanup::cleanup;
