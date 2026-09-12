//! Durable claims only: provider calls and encryption belong outside the SQLite writer.
use crate::{error::sql_integer, Database, Error, ReadDatabase, Result};
use rusqlite::{params, OptionalExtension as _, TransactionBehavior};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SsoSessionState {
    Waiting = 0,
    Exchanging = 1,
    Ready = 2,
    Completed = 3,
    Denied = 4,
    ExchangeUnknown = 5,
    Expired = 6,
    Rejected = 7,
}
impl SsoSessionState {
    pub fn permits(self, next: Self) -> bool {
        use SsoSessionState::*;
        matches!(
            (self, next),
            (Waiting, Exchanging | Denied | Expired)
                | (Exchanging, Ready | ExchangeUnknown | Expired | Rejected)
                | (Ready, Completed | Rejected | Expired)
        )
    }
    fn decode(n: u8) -> rusqlite::Result<Self> {
        use SsoSessionState::*;
        match n {
            0 => Ok(Waiting),
            1 => Ok(Exchanging),
            2 => Ok(Ready),
            3 => Ok(Completed),
            4 => Ok(Denied),
            5 => Ok(ExchangeUnknown),
            6 => Ok(Expired),
            7 => Ok(Rejected),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}
#[derive(Clone)]
pub struct SsoSession {
    pub host: [u8; 33],
    pub session_hash: [u8; 32],
    pub config_hash: [u8; 32],
    pub admission_hash: [u8; 32],
    pub uid: Option<[u8; 33]>,
    pub state: SsoSessionState,
    pub revision: u64,
    pub expires_at_ms: u64,
    pub ciphertext: Vec<u8>,
}
impl std::fmt::Debug for SsoSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsoSession")
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
impl Database {
    pub fn sso_require_policy(&self, host: &[u8; 33], hash: &[u8; 32]) -> Result<()> {
        policy_matches(&self.connection, host, hash)
    }
    pub fn sso_store_poll(
        &mut self,
        old: &SsoSession,
        ciphertext: &[u8],
        reservation: Option<(&[u8], &[u8; 17], u64)>,
        now_ms: u64,
    ) -> Result<()> {
        if old.state != SsoSessionState::Ready {
            return Err(Error::AuthorizationChanged);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        policy_matches(&tx, &old.host, &old.config_hash)?;
        let changed=tx.execute("UPDATE sso_sessions SET revision=revision+1,ciphertext=?1 WHERE host=?2 AND session_hash=?3 AND state=2 AND revision=?4 AND config_hash=?5 AND expires_at_ms>?6",params![ciphertext,old.host,old.session_hash,sql_integer(old.revision)?,old.config_hash,sql_integer(now_ms)?])?;
        if changed != 1 {
            return Err(Error::AuthorizationChanged);
        }
        if let Some((name, token, expires)) = reservation {
            if old.uid.is_some()
                || name.is_empty()
                || name.len() > self.config.maximum_name_bytes
                || expires <= now_ms
            {
                return Err(Error::Invalid("SSO name reservation"));
            }
            crate::names::reserve_on(
                &tx,
                name,
                token,
                1,
                sql_integer(now_ms.checked_mul(1000).ok_or(Error::IntegerRange)?)?,
                expires.checked_mul(1000).ok_or(Error::IntegerRange)?,
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn sso_set_policy(
        &mut self,
        host: &[u8; 33],
        hash: &[u8; 32],
        enabled: bool,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO sso_policy(host,config_hash,enabled) VALUES(?1,?2,?3) ON CONFLICT(host) DO UPDATE SET config_hash=excluded.config_hash, enabled=excluded.enabled",params![host,hash,enabled])?;
        Ok(())
    }
    pub fn sso_session(&self, host: &[u8; 33], id: &[u8; 32]) -> Result<Option<SsoSession>> {
        read_session(&self.connection, host, id)
    }
    pub fn sso_insert_session(&mut self, row: &SsoSession, now_ms: u64) -> Result<()> {
        if row.state != SsoSessionState::Waiting
            || row.revision != 1
            || row.expires_at_ms <= now_ms
            || row.expires_at_ms - now_ms > 600_000
        {
            return Err(Error::Invalid("SSO session lifetime/state"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        policy_matches(&tx, &row.host, &row.config_hash)?;
        // All records count until expiry, including denied sessions, to bound disk and churn.
        tx.execute(
            "DELETE FROM sso_sessions WHERE host=?1 AND expires_at_ms<=?2",
            params![row.host, sql_integer(now_ms)?],
        )?;
        let (host_count, identity_count): (i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(admission_hash=?2),0) FROM sso_sessions WHERE host=?1",
            params![row.host, row.admission_hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if host_count >= 1000 || identity_count >= 4 {
            return Err(Error::Capacity("SSO sessions"));
        }
        tx.execute("INSERT INTO sso_sessions(host,session_hash,config_hash,admission_hash,uid,state,revision,expires_at_ms,ciphertext) VALUES(?1,?2,?3,?4,?5,0,1,?6,?7)",params![row.host,row.session_hash,row.config_hash,row.admission_hash,row.uid,sql_integer(row.expires_at_ms)?,row.ciphertext])?;
        tx.commit()?;
        Ok(())
    }
    /// The caller encrypts for the new revision/state before submitting this CAS.
    pub fn sso_transition(
        &mut self,
        old: &SsoSession,
        next: SsoSessionState,
        ciphertext: &[u8],
        now_ms: u64,
    ) -> Result<()> {
        if !old.state.permits(next) || old.revision == u64::MAX {
            return Err(Error::Invalid("SSO transition"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        policy_matches(&tx, &old.host, &old.config_hash)?;
        let changed=tx.execute("UPDATE sso_sessions SET state=?1,revision=revision+1,ciphertext=?2 WHERE host=?3 AND session_hash=?4 AND config_hash=?5 AND state=?6 AND revision=?7 AND expires_at_ms>?8",params![next as u8,ciphertext,old.host,old.session_hash,old.config_hash,old.state as u8,sql_integer(old.revision)?,sql_integer(now_ms)?])?;
        if changed != 1 {
            return Err(Error::AuthorizationChanged);
        }
        tx.commit()?;
        Ok(())
    }
    /// Startup fences exchanges interrupted by a process exit. No authorization code replay.
    pub fn sso_abandon_exchanges(&mut self, host: &[u8; 33]) -> Result<usize> {
        // Keep ciphertext unchanged for forensic cleanup, but it cannot decrypt under the new
        // state/revision AAD and must never be opened or used to grant authority.
        Ok(self.connection.execute(
            "UPDATE sso_sessions SET state=5,revision=revision+1 WHERE host=?1 AND state=1",
            [host],
        )?)
    }
}
impl ReadDatabase {
    pub fn sso_session(&self, host: &[u8; 33], id: &[u8; 32]) -> Result<Option<SsoSession>> {
        read_session(&self.connection, host, id)
    }
}
pub(crate) fn policy_matches(
    c: &rusqlite::Connection,
    host: &[u8; 33],
    hash: &[u8; 32],
) -> Result<()> {
    let valid: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sso_policy WHERE host=?1 AND config_hash=?2 AND enabled=1)",
        params![host, hash],
        |r| r.get(0),
    )?;
    if valid {
        Ok(())
    } else {
        Err(Error::AuthorizationChanged)
    }
}
fn read_session(
    c: &rusqlite::Connection,
    host: &[u8; 33],
    id: &[u8; 32],
) -> Result<Option<SsoSession>> {
    Ok(c.query_row("SELECT config_hash,admission_hash,uid,state,revision,expires_at_ms,ciphertext FROM sso_sessions WHERE host=?1 AND session_hash=?2",params![host,id],|r|Ok(SsoSession{host:*host,session_hash:*id,config_hash:r.get(0)?,admission_hash:r.get(1)?,uid:r.get(2)?,state:SsoSessionState::decode(r.get(3)?)?,revision:r.get::<_,i64>(4)? as u64,expires_at_ms:r.get::<_,i64>(5)? as u64,ciphertext:r.get(6)?})).optional()?)
}
