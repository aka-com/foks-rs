//! Durable rollout policy. Provider availability never changes cohort membership or mode.
use crate::{error::sql_integer, Database, Error, ReadDatabase, ReadSnapshot, Result};
use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SsoRolloutMode {
    Migration = 0,
    Enforced = 1,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SsoProviderBlockReason {
    MissingConfiguration = 1,
    ConfigurationMismatch = 2,
    KeyUnavailable = 3,
    Operator = 4,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SsoPolicy {
    pub host: [u8; 33],
    pub rollout_id: [u8; 16],
    pub config_hash: [u8; 32],
    pub issuer: String,
    pub mode: SsoRolloutMode,
    pub blocked_reason: Option<SsoProviderBlockReason>,
    pub revision: u64,
    pub authorization_epoch: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SsoAccessDecision {
    NoPolicy,
    MigrationEligible,
    LinkedActive,
    LinkOnly,
    Denied,
}
impl SsoAccessDecision {
    pub fn permits_native(self) -> bool {
        matches!(
            self,
            Self::NoPolicy | Self::MigrationEligible | Self::LinkedActive
        )
    }
    pub fn permits_administration(self) -> bool {
        matches!(self, Self::NoPolicy | Self::LinkedActive)
    }
}
/// Stable authorization facts for a web session, independent of refresh CAS
/// revisions. Absence of a policy is explicit; migration eligibility alone
/// cannot establish a web-session authorization binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorizationBinding {
    Unconfigured,
    Linked {
        host: [u8; 33],
        config_hash: [u8; 32],
        policy_epoch: u64,
        account_generation: u64,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SsoRolloutStatus {
    pub policy: SsoPolicy,
    pub cohort: u64,
    pub linked: u64,
    pub unlinked: u64,
}
#[derive(Clone, Debug)]
pub enum SsoPolicyTransition {
    Enforce {
        expected_unlinked: u64,
        accept_lockout: bool,
    },
    Reenable,
    ReplaceProvider {
        config_hash: [u8; 32],
        issuer: String,
    },
}
impl Database {
    pub fn sso_authorization_binding(
        &self,
        uid: &[u8],
        now_ms: u64,
    ) -> Result<AuthorizationBinding> {
        authorization_binding(&self.connection, uid, now_ms)
    }

    /// Eligibility to claim one refresh is separate from permission to use ordinary services.
    pub fn sso_refresh_eligible(&self, uid: &[u8], now_ms: u64) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sso_access a JOIN sso_policy p ON p.host=a.host WHERE a.uid=?1 AND p.blocked_reason IS NULL AND a.config_hash=p.config_hash AND a.authorization_epoch=p.authorization_epoch AND a.state=0 AND a.interrupted=0 AND a.expires_at_ms<=?2)",params![uid,sql_integer(now_ms)?],|r|r.get(0))?)
    }

    pub fn sso_policy(&self, host: &[u8; 33]) -> Result<Option<SsoPolicy>> {
        read(&self.connection, host)
    }
    pub fn sso_rollout_status(&self, host: &[u8; 33]) -> Result<Option<SsoRolloutStatus>> {
        status(&self.connection, host)
    }
    pub fn sso_access_decision(&self, uid: &[u8], now_ms: u64) -> Result<SsoAccessDecision> {
        decision(&self.connection, uid, now_ms)
    }
    /// Creation is deliberately separate from restart/configuration reconciliation.
    pub fn sso_activate(
        &mut self,
        host: &[u8; 33],
        rollout_id: &[u8; 16],
        hash: &[u8; 32],
        issuer: &str,
        mode: SsoRolloutMode,
    ) -> Result<SsoPolicy> {
        if rollout_id == &[0; 16] || issuer.is_empty() || issuer.len() > 4096 {
            return Err(Error::Invalid("SSO rollout identity"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read(&tx, host)?.is_some() {
            return Err(Error::AuthorizationChanged);
        }
        let users: i64 = tx.query_row("SELECT count(*) FROM users", [], |r| r.get(0))?;
        if users != 0 && mode == SsoRolloutMode::Enforced {
            return Err(Error::Invalid("enforced activation requires an empty host"));
        }
        tx.execute("INSERT INTO sso_policy(host,rollout_id,config_hash,issuer,mode,blocked_reason,revision,authorization_epoch) VALUES(?1,?2,?3,?4,?5,NULL,1,1)", params![host,rollout_id,hash,issuer,mode as u8])?;
        tx.execute(
            "INSERT INTO sso_migration_cohort(host,uid) SELECT ?1,uid FROM users",
            [host],
        )?;
        let result = read(&tx, host)?.ok_or(Error::AuthorizationChanged)?;
        tx.commit()?;
        Ok(result)
    }
    /// Fail closed, without repeatedly changing the epoch on identical failed startups.
    pub fn sso_block_policy(
        &mut self,
        host: &[u8; 33],
        reason: SsoProviderBlockReason,
    ) -> Result<()> {
        self.connection.execute("UPDATE sso_policy SET blocked_reason=?2,revision=revision+1,authorization_epoch=authorization_epoch+1 WHERE host=?1 AND (blocked_reason IS NULL OR blocked_reason!=?2)", params![host,reason as u8])?;
        Ok(())
    }
    /// The complete observed policy is the CAS token, including rollout, hash and epoch.
    pub fn sso_transition_policy(
        &mut self,
        expected: &SsoPolicy,
        transition: &SsoPolicyTransition,
    ) -> Result<SsoPolicy> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read(&tx, &expected.host)?.as_ref() != Some(expected) {
            return Err(Error::AuthorizationChanged);
        }
        let mut next = expected.clone();
        next.revision = next.revision.checked_add(1).ok_or(Error::IntegerRange)?;
        match transition {
            SsoPolicyTransition::Enforce {
                expected_unlinked,
                accept_lockout,
            } => {
                if next.mode != SsoRolloutMode::Migration || next.blocked_reason.is_some() {
                    return Err(Error::AuthorizationChanged);
                }
                let counts = status(&tx, &next.host)?.ok_or(Error::AuthorizationChanged)?;
                if counts.unlinked != *expected_unlinked || (counts.unlinked > 0 && !accept_lockout)
                {
                    return Err(Error::AuthorizationChanged);
                }
                next.mode = SsoRolloutMode::Enforced;
            }
            SsoPolicyTransition::Reenable => {
                if next.blocked_reason.is_none() {
                    return Err(Error::AuthorizationChanged);
                }
                next.blocked_reason = None;
                next.authorization_epoch = next
                    .authorization_epoch
                    .checked_add(1)
                    .ok_or(Error::IntegerRange)?;
            }
            SsoPolicyTransition::ReplaceProvider {
                config_hash,
                issuer,
            } => {
                if issuer != &next.issuer || config_hash == &next.config_hash {
                    return Err(Error::Invalid(
                        "SSO provider replacement requires the same issuer and a new fingerprint",
                    ));
                }
                next.config_hash = *config_hash;
                next.blocked_reason = None;
                next.authorization_epoch = next
                    .authorization_epoch
                    .checked_add(1)
                    .ok_or(Error::IntegerRange)?;
            }
        }
        tx.execute("UPDATE sso_policy SET config_hash=?2,mode=?3,blocked_reason=?4,revision=?5,authorization_epoch=?6 WHERE host=?1", params![next.host,next.config_hash,next.mode as u8,next.blocked_reason.map(|f|f as u8),sql_integer(next.revision)?,sql_integer(next.authorization_epoch)?])?;
        tx.commit()?;
        Ok(next)
    }
}
impl ReadDatabase {
    pub fn sso_policy(&self, host: &[u8; 33]) -> Result<Option<SsoPolicy>> {
        read(&self.connection, host)
    }
    pub fn sso_access_decision(&self, uid: &[u8], now_ms: u64) -> Result<SsoAccessDecision> {
        decision(&self.connection, uid, now_ms)
    }
}
impl ReadSnapshot<'_> {
    pub fn sso_authorization_binding(
        &self,
        uid: &[u8],
        now_ms: u64,
    ) -> Result<AuthorizationBinding> {
        authorization_binding(self.connection(), uid, now_ms)
    }

    pub fn sso_rollout_status(&self, host: &[u8; 33]) -> Result<Option<SsoRolloutStatus>> {
        status(self.connection(), host)
    }
    pub fn sso_access_decision(&self, uid: &[u8], now_ms: u64) -> Result<SsoAccessDecision> {
        decision(self.connection(), uid, now_ms)
    }
}
pub(crate) fn read(c: &Connection, host: &[u8; 33]) -> Result<Option<SsoPolicy>> {
    Ok(c.query_row("SELECT rollout_id,config_hash,issuer,mode,blocked_reason,revision,authorization_epoch FROM sso_policy WHERE host=?1", [host], |r| {
        let mode = match r.get::<_,u8>(3)? { 0=>SsoRolloutMode::Migration,1=>SsoRolloutMode::Enforced,_=>return Err(rusqlite::Error::InvalidQuery) };
        let blocked_reason = match r.get::<_,Option<u8>>(4)? {None=>None,Some(1)=>Some(SsoProviderBlockReason::MissingConfiguration),Some(2)=>Some(SsoProviderBlockReason::ConfigurationMismatch),Some(3)=>Some(SsoProviderBlockReason::KeyUnavailable),Some(4)=>Some(SsoProviderBlockReason::Operator),_=>return Err(rusqlite::Error::InvalidQuery)};
        Ok(SsoPolicy { host:*host,rollout_id:r.get(0)?,config_hash:r.get(1)?,issuer:r.get(2)?,mode,blocked_reason,revision:r.get::<_,i64>(5)? as u64,authorization_epoch:r.get::<_,i64>(6)? as u64 })
    }).optional()?)
}
fn status(c: &Connection, host: &[u8; 33]) -> Result<Option<SsoRolloutStatus>> {
    // Callers needing a multi-query online view use ReadSnapshot; writer callers are serialized.
    let Some(policy) = read(c, host)? else {
        return Ok(None);
    };
    let (cohort,linked):(i64,i64) = c.query_row("SELECT count(*),coalesce(sum(EXISTS(SELECT 1 FROM sso_access a WHERE a.host=c.host AND a.uid=c.uid)),0) FROM sso_migration_cohort c WHERE c.host=?1", [host], |r|Ok((r.get(0)?,r.get(1)?)))?;
    Ok(Some(SsoRolloutStatus {
        policy,
        cohort: cohort as u64,
        linked: linked as u64,
        unlinked: (cohort - linked) as u64,
    }))
}
pub(crate) fn decision(c: &Connection, uid: &[u8], now_ms: u64) -> Result<SsoAccessDecision> {
    // One statement gives reader-pool callers the same coherent policy/access/cohort view
    // as writer callers. Never combine a pre-enforcement mode with post-enforcement data.
    let value: Option<u8> = c
        .query_row(
            "SELECT CASE WHEN p.blocked_reason IS NOT NULL THEN 4
            WHEN a.uid IS NOT NULL THEN CASE WHEN a.config_hash=p.config_hash
                AND a.authorization_epoch=p.authorization_epoch AND a.state=0
                AND a.interrupted=0 AND a.expires_at_ms>?2 THEN 2 ELSE 4 END
            WHEN c.uid IS NOT NULL THEN CASE WHEN p.mode=0 THEN 1 ELSE 3 END
            ELSE 4 END
         FROM sso_policy p LEFT JOIN sso_access a ON a.host=p.host AND a.uid=?1
         LEFT JOIN sso_migration_cohort c ON c.host=p.host AND c.uid=?1",
            params![uid, sql_integer(now_ms)?],
            |r| r.get(0),
        )
        .optional()?;
    use SsoAccessDecision::*;
    Ok(match value {
        None => NoPolicy,
        Some(1) => MigrationEligible,
        Some(2) => LinkedActive,
        Some(3) => LinkOnly,
        Some(4) => Denied,
        _ => return Err(Error::Invalid("SSO access decision")),
    })
}

/// One statement so configured absence, policy and linked access cannot race.
pub(crate) fn authorization_binding(
    c: &Connection,
    uid: &[u8],
    now_ms: u64,
) -> Result<AuthorizationBinding> {
    let row = c
        .query_row(
            "SELECT p.host,p.config_hash,p.authorization_epoch,a.authorization_generation,
            p.blocked_reason IS NULL AND a.uid IS NOT NULL AND a.config_hash=p.config_hash
            AND a.authorization_epoch=p.authorization_epoch AND a.state=0
            AND a.interrupted=0 AND a.expires_at_ms>?2
         FROM sso_policy p LEFT JOIN sso_access a ON a.host=p.host AND a.uid=?1",
            params![uid, sql_integer(now_ms)?],
            |r| {
                Ok((
                    r.get::<_, [u8; 33]>(0)?,
                    r.get::<_, [u8; 32]>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<bool>>(4)?.unwrap_or(false),
                ))
            },
        )
        .optional()?;
    match row {
        None => Ok(AuthorizationBinding::Unconfigured),
        Some((host, config_hash, policy_epoch, Some(account_generation), true)) => {
            Ok(AuthorizationBinding::Linked {
                host,
                config_hash,
                policy_epoch: crate::error::unsigned(policy_epoch)?,
                account_generation: crate::error::unsigned(account_generation)?,
            })
        }
        _ => Err(Error::AuthorizationChanged),
    }
}
