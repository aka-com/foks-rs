//! Provider-independent owner proofs and durable nonsecret binding evidence.
use crate::{error::sql_integer, Database, Error, Result};
use foks_proto::{
    IdentityChallenge, IdentityClaim, IdentityProof, IdentityStatus, SsoAccountState,
};
use rusqlite::{params, Connection, OptionalExtension as _, TransactionBehavior};

pub(crate) const RECEIPT_LIFETIME_MS: u64 = 30 * 24 * 60 * 60 * 1000;
impl Database {
    pub fn sso_issue_identity_challenge(
        &mut self,
        claim: &IdentityClaim,
        challenge: [u8; 32],
        admission_hash: [u8; 32],
        now_ms: u64,
    ) -> Result<IdentityChallenge> {
        let encoded = claim
            .encoded()
            .map_err(|_| Error::Invalid("identity claim"))?;
        let claim_hash = foks_crypto::prefixed_hash(0xf04b_a81e_c057_0002, &encoded);
        let expires_at_ms = now_ms.checked_add(60_000).ok_or(Error::IntegerRange)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM sso_identity_challenges WHERE challenge IN (SELECT challenge FROM sso_identity_challenges WHERE expires_at_ms<=?1 ORDER BY expires_at_ms LIMIT 128)",[sql_integer(now_ms)?])?;
        let (total, source): (i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(admission_hash=?1),0) FROM sso_identity_challenges",
            [admission_hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if total >= 1024 || source >= 8 {
            return Err(Error::Capacity("identity challenges"));
        }
        tx.execute(
            "INSERT INTO sso_identity_challenges VALUES(?1,?2,?3,?4)",
            params![
                challenge,
                claim_hash,
                admission_hash,
                sql_integer(expires_at_ms)?
            ],
        )?;
        tx.commit()?;
        Ok(IdentityChallenge {
            claim: claim.clone(),
            challenge,
            issued_at_ms: now_ms,
            expires_at_ms,
        })
    }
    /// Signature verification precedes writer mutation; current ownership is checked in the TX.
    pub fn sso_prove_identity(
        &mut self,
        host: &[u8; 33],
        proof: &IdentityProof,
        now_ms: u64,
    ) -> Result<IdentityStatus> {
        foks_crypto::verify_identity_proof(proof).map_err(|_| Error::AuthorizationChanged)?;
        let ch = &proof.challenge;
        let claim = &ch.claim;
        if claim.host.as_bytes() != host
            || ch.expires_at_ms <= now_ms
            || ch.expires_at_ms.checked_sub(ch.issued_at_ms) != Some(60_000)
        {
            return Err(Error::AuthorizationChanged);
        }
        let claim_hash = foks_crypto::prefixed_hash(
            0xf04b_a81e_c057_0002,
            &claim.encoded().map_err(|_| Error::AuthorizationChanged)?,
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_owner(&tx, claim.uid.as_bytes(), claim.signer.as_bytes(), None)?;
        let removed=tx.execute("DELETE FROM sso_identity_challenges WHERE challenge=?1 AND claim_hash=?2 AND expires_at_ms=?3 AND expires_at_ms>?4",params![ch.challenge,claim_hash,sql_integer(ch.expires_at_ms)?,sql_integer(now_ms)?])?;
        if removed != 1 {
            return Err(Error::AuthorizationChanged);
        }
        let p = crate::sso_policy::read(&tx, host)?;
        let a = crate::sso_access::read(&tx, claim.uid.as_bytes())?;
        let cohort: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sso_migration_cohort WHERE host=?1 AND uid=?2)",
            params![host, claim.uid.as_bytes()],
            |r| r.get(0),
        )?;
        let account_state = match (&p, &a, cohort) {
            (None, _, _) => SsoAccountState::DeviceOnly,
            (_, Some(_), _) => SsoAccountState::Linked,
            (Some(p), None, true) if p.mode == crate::SsoRolloutMode::Migration => {
                SsoAccountState::MigrationEligible
            }
            (Some(_), None, true) => SsoAccountState::LockedOut,
            _ => SsoAccountState::NotEligible,
        };
        let committed_receipt = match claim.receipt_commitment {
            Some(commitment)
                if receipt_exists(&tx, host, claim.uid.as_bytes(), &commitment, now_ms)? =>
            {
                Some(commitment)
            }
            _ => None,
        };
        let status = IdentityStatus {
            challenge: ch.challenge,
            account_state,
            rollout_id: p.as_ref().map(|p| p.rollout_id),
            rollout_mode: p.as_ref().map_or(0, |p| p.mode as u8 + 1),
            provider_fence: p.as_ref().and_then(|p| p.fence).map_or(0, |f| f as u8),
            issuer: p.as_ref().map_or_else(String::new, |p| p.issuer.clone()),
            authorization_epoch: p.as_ref().map_or(0, |p| p.authorization_epoch),
            authorization_generation: a.as_ref().map_or(0, |a| a.authorization_generation),
            access_available: crate::sso_policy::decision(&tx, claim.uid.as_bytes(), now_ms)?
                .permits_native(),
            committed_receipt,
        };
        tx.commit()?;
        Ok(status)
    }
}
pub(crate) fn require_owner(
    c: &Connection,
    uid: &[u8],
    credential: &[u8],
    sequence: Option<u64>,
) -> Result<()> {
    // Software device 4, Yubi parent 8, subkey 13. Bots and backup credentials are excluded.
    if !matches!(credential.first(), Some(4 | 8 | 13)) {
        return Err(Error::AuthorizationChanged);
    }
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM devices d JOIN user_chain_heads h ON h.uid=d.uid WHERE d.uid=?1 AND d.active=1 AND d.role_type=3 AND (d.device_id=?2 OR (d.subkey_id=?2 AND substr(d.device_id,1,1)=x'08')) AND (?3 IS NULL OR h.seqno=?3))",params![uid,credential,sequence.map(sql_integer).transpose()?],|r|r.get(0))?;
    if !valid {
        return Err(Error::AuthorizationChanged);
    }
    Ok(())
}
pub(crate) fn receipt_exists(
    c: &Connection,
    host: &[u8; 33],
    uid: &[u8],
    commitment: &[u8; 32],
    now_ms: u64,
) -> Result<bool> {
    Ok(c.query_row("SELECT 1 FROM sso_binding_receipts WHERE host=?1 AND uid=?2 AND commitment=?3 AND expires_at_ms>?4",params![host,uid,commitment,sql_integer(now_ms)?], |_|Ok(())).optional()?.is_some())
}
pub(crate) fn reserve_receipt(
    c: &Connection,
    b: &crate::SsoAccountBinding,
    now_ms: u64,
) -> Result<()> {
    let a = &b.access;
    c.execute("DELETE FROM sso_binding_receipts WHERE rowid IN (SELECT rowid FROM sso_binding_receipts WHERE host=?1 AND uid=?2 AND expires_at_ms<=?3 ORDER BY expires_at_ms LIMIT 128)",params![a.host,a.uid,sql_integer(now_ms)?])?;
    let count: i64 = c.query_row(
        "SELECT count(*) FROM sso_binding_receipts WHERE host=?1 AND uid=?2",
        params![a.host, a.uid],
        |r| r.get(0),
    )?;
    if count >= 4096 {
        return Err(Error::Capacity("SSO binding receipts"));
    }
    c.execute(
        "INSERT INTO sso_binding_receipts VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            a.host,
            a.uid,
            b.commitment,
            b.purpose as u8,
            sql_integer(a.authorization_epoch)?,
            sql_integer(a.authorization_generation)?,
            sql_integer(
                now_ms
                    .checked_add(RECEIPT_LIFETIME_MS)
                    .ok_or(Error::IntegerRange)?
            )?
        ],
    )?;
    Ok(())
}
