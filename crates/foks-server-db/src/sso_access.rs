use crate::{
    error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result, SsoSession,
    SsoSessionState,
};
use rusqlite::{params, OptionalExtension as _, TransactionBehavior};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SsoAccessState {
    Active = 0,
    Refreshing = 1,
    ReauthenticationRequired = 2,
    ProviderUnavailable = 3,
}
impl SsoAccessState {
    fn decode(n: u8) -> rusqlite::Result<Self> {
        match n {
            0 => Ok(Self::Active),
            1 => Ok(Self::Refreshing),
            2 => Ok(Self::ReauthenticationRequired),
            3 => Ok(Self::ProviderUnavailable),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}
#[derive(Clone)]
pub struct SsoAccess {
    pub host: [u8; 33],
    pub uid: [u8; 33],
    pub issuer: String,
    pub subject: String,
    pub config_hash: [u8; 32],
    pub revision: u64,
    pub authorization_epoch: u64,
    pub authorization_generation: u64,
    pub interrupted: bool,
    pub state: SsoAccessState,
    pub expires_at_ms: u64,
    pub ciphertext: Vec<u8>,
}
impl std::fmt::Debug for SsoAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoAccess")
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
pub struct SsoAccountBinding {
    pub purpose: foks_proto::SsoPurpose,
    pub commitment: [u8; 32],
    pub flow: SsoSession,
    pub completed_ciphertext: Vec<u8>,
    pub access: SsoAccess,
    pub device: Vec<u8>,
    pub expected_user_sequence: Option<u64>,
}
impl Database {
    pub fn sso_access(&self, uid: &[u8]) -> Result<Option<SsoAccess>> {
        read(&self.connection, uid)
    }
    pub fn sso_require_access(&self, uid: &[u8], now_ms: u64) -> Result<()> {
        require_access(&self.connection, uid, now_ms)
    }
    pub fn sso_disable_policy(&mut self) -> Result<()> {
        self.connection
            .execute("UPDATE sso_policy SET blocked_reason=1,revision=revision+1,authorization_epoch=authorization_epoch+1 WHERE blocked_reason IS NULL OR blocked_reason!=1", [])?;
        Ok(())
    }
    pub fn sso_login(&mut self, binding: &SsoAccountBinding, now_ms: u64) -> Result<()> {
        if binding.flow.uid != Some(binding.access.uid) || binding.expected_user_sequence.is_none()
        {
            return Err(Error::Invalid("SSO login purpose"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        bind(&tx, binding, now_ms, false)?;
        tx.commit()?;
        Ok(())
    }
    pub fn sso_transition_access(
        &mut self,
        old: &SsoAccess,
        next: &SsoAccess,
        credential: &[u8],
        expected_user_sequence: u64,
        now_ms: u64,
    ) -> Result<()> {
        use SsoAccessState::*;
        if next.host != old.host
            || next.uid != old.uid
            || next.issuer != old.issuer
            || next.subject != old.subject
            || next.config_hash != old.config_hash
            || old.interrupted
            || next.interrupted
            || next.authorization_epoch != old.authorization_epoch
            || next.authorization_generation != old.authorization_generation
            || next.revision != old.revision.checked_add(1).ok_or(Error::IntegerRange)?
            || !matches!(
                (old.state, next.state),
                (Active, Refreshing | ReauthenticationRequired)
                    | (
                        Refreshing,
                        Active | ReauthenticationRequired | ProviderUnavailable
                    )
            )
            || (next.state == Active && next.expires_at_ms <= now_ms)
        {
            return Err(Error::Invalid("SSO access transition"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::sso::policy_epoch_matches(
            &tx,
            &old.host,
            &old.config_hash,
            old.authorization_epoch,
        )?;
        check_credential(&tx, &old.uid, credential, expected_user_sequence)?;
        let count=tx.execute("UPDATE sso_access SET state=?1,revision=?2,expires_at_ms=?3,ciphertext=?4 WHERE host=?5 AND uid=?6 AND revision=?7 AND state=?8 AND config_hash=?9 AND interrupted=0",params![next.state as u8,sql_integer(next.revision)?,sql_integer(next.expires_at_ms)?,next.ciphertext,old.host,old.uid,sql_integer(old.revision)?,old.state as u8,old.config_hash])?;
        if count != 1 {
            return Err(Error::AuthorizationChanged);
        }
        tx.commit()?;
        Ok(())
    }
    pub fn sso_abandon_refreshes(&mut self) -> Result<usize> {
        Ok(self.connection.execute(
            "UPDATE sso_access SET interrupted=1 WHERE state=1 AND interrupted=0",
            [],
        )?)
    }
}
impl ReadDatabase {
    pub fn sso_access(&self, uid: &[u8]) -> Result<Option<SsoAccess>> {
        read(&self.connection, uid)
    }
    pub fn sso_require_access(&self, uid: &[u8], now_ms: u64) -> Result<()> {
        require_access(&self.connection, uid, now_ms)
    }
}
impl ReadSnapshot<'_> {
    pub fn sso_require_access(&self, uid: &[u8], now_ms: u64) -> Result<()> {
        require_access(self.connection(), uid, now_ms)
    }
}
pub(crate) fn require_access(c: &rusqlite::Connection, uid: &[u8], now_ms: u64) -> Result<()> {
    if crate::sso_policy::decision(c, uid, now_ms)?.permits_native() {
        Ok(())
    } else {
        Err(Error::AuthorizationChanged)
    }
}

pub(crate) fn require_signup(
    c: &rusqlite::Connection,
    binding: Option<&SsoAccountBinding>,
) -> Result<()> {
    let configured: bool =
        c.query_row("SELECT EXISTS(SELECT 1 FROM sso_policy)", [], |r| r.get(0))?;
    if configured != binding.is_some() {
        return Err(Error::AuthorizationChanged);
    }
    Ok(())
}
pub(crate) fn bind(
    c: &rusqlite::Connection,
    b: &SsoAccountBinding,
    now_ms: u64,
    signup: bool,
) -> Result<()> {
    let a = &b.access;
    let f = &b.flow;
    use foks_proto::SsoPurpose;
    if signup != (b.purpose == SsoPurpose::Signup) {
        return Err(Error::AuthorizationChanged);
    }
    let seq = if signup {
        1
    } else {
        b.expected_user_sequence
            .ok_or(Error::AuthorizationChanged)?
    };
    check_credential(c, &a.uid, &b.device, seq)?;
    if b.purpose == SsoPurpose::LinkExisting {
        crate::sso_identity::require_owner(c, &a.uid, &b.device, Some(seq))?;
    }
    // Exact repeats are evidence only, and do not issue a new authorization generation.
    if crate::sso_identity::authorization_binding_exists(c, &a.host, &a.uid, &b.commitment, now_ms)?
    {
        return Ok(());
    }
    crate::sso::policy_epoch_matches(c, &a.host, &a.config_hash, a.authorization_epoch)?;
    if a.host != f.host
        || a.config_hash != f.config_hash
        || a.authorization_epoch != f.authorization_epoch
        || a.interrupted
        || f.interrupted
        || a.state != SsoAccessState::Active
        || a.expires_at_ms <= now_ms
        || f.expires_at_ms <= now_ms
        || f.state != SsoSessionState::Ready
        || (signup && (f.uid.is_some() || b.expected_user_sequence.is_some()))
    {
        return Err(Error::AuthorizationChanged);
    }
    let old = read(c, &a.uid)?;
    match (&old, b.purpose) {
        (None, SsoPurpose::Signup) if a.revision == 1 && a.authorization_generation == 1 => {}
        (None, SsoPurpose::LinkExisting) if a.revision == 1 && a.authorization_generation == 1 => {
            let eligible: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM sso_migration_cohort WHERE host=?1 AND uid=?2)",
                params![a.host, a.uid],
                |r| r.get(0),
            )?;
            if !eligible {
                return Err(Error::AuthorizationChanged);
            }
        }
        (Some(old), SsoPurpose::Reauthenticate)
            if old.issuer == a.issuer
                && old.subject == a.subject
                && a.authorization_generation
                    == old
                        .authorization_generation
                        .checked_add(1)
                        .ok_or(Error::IntegerRange)?
                && a.revision == old.revision.checked_add(1).ok_or(Error::IntegerRange)? => {}
        _ => return Err(Error::AuthorizationChanged),
    }
    let linked_uid: Option<Vec<u8>> = c
        .query_row(
            "SELECT uid FROM sso_access WHERE host=?1 AND issuer=?2 AND subject=?3",
            params![a.host, a.issuer, a.subject],
            |r| r.get(0),
        )
        .optional()?;
    if linked_uid.is_some_and(|uid| uid.as_slice() != a.uid) {
        return Err(Error::Duplicate("SSO provider identity"));
    }
    crate::sso_identity::reserve_authorization_binding(c, b, now_ms)?;
    if b.purpose == SsoPurpose::Reauthenticate {
        let old = old.as_ref().ok_or(Error::AuthorizationChanged)?;
        let changed=c.execute("UPDATE sso_access SET config_hash=?3,revision=?4,state=0,expires_at_ms=?5,ciphertext=?6,authorization_epoch=?7,authorization_generation=?8,interrupted=0 WHERE host=?1 AND uid=?2 AND revision=?9 AND issuer=?10 AND subject=?11",params![a.host,a.uid,a.config_hash,sql_integer(a.revision)?,sql_integer(a.expires_at_ms)?,a.ciphertext,sql_integer(a.authorization_epoch)?,sql_integer(a.authorization_generation)?,sql_integer(old.revision)?,a.issuer,a.subject])?;
        if changed != 1 {
            return Err(Error::AuthorizationChanged);
        }
    } else {
        // First linkage is create-only. A competing subject can never replace it.
        c.execute("INSERT INTO sso_access(host,uid,issuer,subject,config_hash,revision,state,expires_at_ms,ciphertext,authorization_epoch,authorization_generation) VALUES(?1,?2,?3,?4,?5,?6,0,?7,?8,?9,?10)",params![a.host,a.uid,a.issuer,a.subject,a.config_hash,sql_integer(a.revision)?,sql_integer(a.expires_at_ms)?,a.ciphertext,sql_integer(a.authorization_epoch)?,sql_integer(a.authorization_generation)?])?;
    }
    let changed=c.execute("UPDATE sso_sessions SET state=3,revision=revision+1,ciphertext=?1 WHERE host=?2 AND session_hash=?3 AND config_hash=?4 AND state=2 AND revision=?5 AND expires_at_ms>?6 AND interrupted=0",params![b.completed_ciphertext,f.host,f.session_hash,f.config_hash,sql_integer(f.revision)?,sql_integer(now_ms)?])?;
    if changed != 1 {
        return Err(Error::AuthorizationChanged);
    }
    Ok(())
}
fn check_credential(
    c: &rusqlite::Connection,
    uid: &[u8],
    credential: &[u8],
    sequence: u64,
) -> Result<()> {
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM devices d JOIN user_chain_heads h ON h.uid=d.uid WHERE d.uid=?1 AND (d.device_id=?2 OR d.subkey_id=?2) AND d.active=1 AND h.seqno=?3)",params![uid,credential,sql_integer(sequence)?],|r|r.get(0))?;
    if valid {
        Ok(())
    } else {
        Err(Error::AuthorizationChanged)
    }
}
pub(crate) fn read(c: &rusqlite::Connection, uid: &[u8]) -> Result<Option<SsoAccess>> {
    Ok(c.query_row("SELECT host,uid,issuer,subject,config_hash,revision,state,expires_at_ms,ciphertext,authorization_epoch,authorization_generation,interrupted FROM sso_access WHERE uid=?1",[uid],|r|Ok(SsoAccess{host:r.get(0)?,uid:r.get(1)?,issuer:r.get(2)?,subject:r.get(3)?,config_hash:r.get(4)?,revision:r.get::<_,i64>(5)? as u64,state:SsoAccessState::decode(r.get(6)?)?,expires_at_ms:r.get::<_,i64>(7)? as u64,ciphertext:r.get(8)?,authorization_epoch:r.get::<_,i64>(9)? as u64,authorization_generation:r.get::<_,i64>(10)? as u64,interrupted:r.get(11)?})).optional()?)
}
