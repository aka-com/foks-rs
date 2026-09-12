use super::*;

impl Database {
    pub fn web_cleanup(&mut self, now: AdminMoment) -> Result<WebCleanup> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let report = cleanup(&tx, now)?;
        tx.commit()?;
        Ok(report)
    }
}

pub(super) fn cleanup(c: &Connection, now: AdminMoment) -> Result<WebCleanup> {
    let epoch = now.epoch;
    let utc = sql_integer(now.utc_us)?;
    let elapsed = sql_integer(now.elapsed_us)?;
    let nonces = c.execute(
        "DELETE FROM web_admin_nonces WHERE nonce_hash IN (
             SELECT nonce_hash FROM web_admin_nonces
             WHERE instance_epoch!=?1
                OR (result_id IS NULL AND admit_before_elapsed_us<=?2)
                OR receipt_retain_until_elapsed_us<=?2 LIMIT 128)",
        params![epoch, elapsed],
    )?;
    let confirmations = c.execute(
        "DELETE FROM web_login_confirmations WHERE binding_hash IN (
             SELECT binding_hash FROM web_login_confirmations
             WHERE instance_epoch!=?1 OR expires_at_us<=?2
                OR deadline_elapsed_us<=?3 LIMIT 128)",
        params![epoch, utc, elapsed],
    )?;
    // Delete children first and retain any parent with remaining children, so
    // cascading foreign keys cannot turn a bounded batch into unbounded work.
    let tickets = c.execute(
        "DELETE FROM web_login_tickets WHERE ticket_hash IN (
             SELECT t.ticket_hash FROM web_login_tickets t
             WHERE (instance_epoch!=?1 OR expires_at_us<=?2
                    OR deadline_elapsed_us<=?3 OR state!=0)
               AND NOT EXISTS(SELECT 1 FROM web_login_confirmations c
                              WHERE c.ticket_hash=t.ticket_hash) LIMIT 128)",
        params![epoch, utc, elapsed],
    )?;
    let sessions = c.execute(
        "DELETE FROM web_admin_sessions WHERE session_hash IN (
             SELECT s.session_hash FROM web_admin_sessions s
             WHERE (instance_epoch!=?1 OR expires_at_us<=?2
                    OR deadline_elapsed_us<=?3 OR revoked_at_us IS NOT NULL)
               AND NOT EXISTS(SELECT 1 FROM web_admin_nonces n
                              WHERE n.session_hash=s.session_hash) LIMIT 128)",
        params![epoch, utc, elapsed],
    )?;
    let audit_cutoff = now.utc_us.saturating_sub(90 * 24 * 60 * 60 * 1_000_000);
    let audit = c.execute(
        "DELETE FROM admin_audit WHERE event_id IN (
             SELECT event_id FROM admin_audit WHERE occurred_at_us<=?1
             ORDER BY occurred_at_us,event_id LIMIT 128)",
        [sql_integer(audit_cutoff)?],
    )?;
    Ok(WebCleanup {
        tickets: tickets as u64,
        confirmations: confirmations as u64,
        sessions: sessions as u64,
        nonces: nonces as u64,
        audit: audit as u64,
    })
}
