use super::*;

impl Database {
    pub fn web_issue_ticket(
        &mut self,
        v: &WebCredential,
        hash: &[u8; 32],
        id: &[u8; 16],
        now: AdminMoment,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (_, _, stamp) = credential(&tx, v, now)?;
        cleanup(&tx, now)?;
        capacity(&tx, "web_login_tickets", &v.uid, 3, 1024)?;
        let expires = now
            .utc_us
            .checked_add(WEB_TICKET_US)
            .ok_or(Error::IntegerRange)?
            .min(v.certificate_expires_at_us);
        let deadline = now
            .elapsed_us
            .checked_add(expires - now.utc_us)
            .ok_or(Error::IntegerRange)?;
        let (hash_stamp, epoch, generation) = stamp_columns(&stamp);
        tx.execute(
            "INSERT INTO web_login_tickets VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0)",
            params![
                hash,
                id,
                v.host,
                v.uid,
                v.credential,
                sql_integer(v.certificate_expires_at_us)?,
                now.epoch,
                hash_stamp,
                epoch,
                generation,
                sql_integer(now.utc_us)?,
                sql_integer(expires)?,
                sql_integer(deadline)?
            ],
        )?;
        audit(
            &tx,
            &v.host,
            Some(v),
            AdminAction::Ticket,
            id,
            None,
            now.utc_us,
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn web_stage(
        &mut self,
        ticket: &[u8; 32],
        binding: &[u8; 32],
        csrf: &[u8; 32],
        existing: Option<&[u8; 32]>,
        now: AdminMoment,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let a = load(&tx, ticket, false)?;
        let ctx = authorize(&tx, &a, now)?;
        // Any live authenticated context must explicitly sign out before a new entry.
        if existing.is_some_and(|h| {
            load(&tx, h, true)
                .and_then(|v| authorize(&tx, &v, now))
                .is_ok()
        }) {
            return Err(Error::ReceiptConflict);
        }
        cleanup(&tx, now)?;
        let(all,own):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(t.uid=?1),0) FROM web_login_confirmations c JOIN web_login_tickets t
             USING(ticket_hash)",[ctx.credential.uid],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if all >= 2048 || own >= 6 {
            return Err(Error::Capacity("browser confirmations"));
        }
        tx.execute(
            "INSERT INTO web_login_confirmations VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                binding,
                csrf,
                ticket,
                now.epoch,
                sql_integer(a.expires)?,
                sql_integer(a.deadline)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn web_redeem(&mut self, request: WebRedemption<'_>, now: AdminMoment) -> Result<()> {
        let WebRedemption {
            binding,
            csrf,
            session,
            session_csrf,
            id,
            existing,
        } = request;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (ticket, expected) = confirmation(&tx, binding, now)?;
        if !constant_eq(&expected, csrf) {
            return Err(Error::AuthorizationChanged);
        }
        let a = load(&tx, &ticket, false)?;
        let ctx = authorize(&tx, &a, now)?;
        if existing.is_some_and(|h| {
            load(&tx, h, true)
                .and_then(|v| authorize(&tx, &v, now))
                .is_ok()
        }) {
            return Err(Error::ReceiptConflict);
        }
        cleanup(&tx, now)?;
        capacity(&tx, "web_admin_sessions", &ctx.credential.uid, 5, 4096)?;
        if tx.execute(
            "UPDATE web_login_tickets SET state=1 WHERE ticket_hash=?1 AND state=0",
            [ticket],
        )? != 1
        {
            return Err(Error::ReceiptExpired);
        }
        tx.execute(
            "DELETE FROM web_login_confirmations WHERE ticket_hash=?1",
            [ticket],
        )?;
        let expires = now
            .utc_us
            .checked_add(WEB_SESSION_US)
            .ok_or(Error::IntegerRange)?
            .min(a.credential.certificate_expires_at_us);
        let deadline = now
            .elapsed_us
            .checked_add(expires - now.utc_us)
            .ok_or(Error::IntegerRange)?;
        let (hash_stamp, epoch, generation) = stamp_columns(&a.stamp);
        let v = &ctx.credential;
        tx.execute("INSERT INTO web_admin_sessions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,NULL,?14)",params![session,id,v.host,v.uid,v.credential,sql_integer(v.certificate_expires_at_us)?,now.epoch,hash_stamp,epoch,generation,sql_integer(now.utc_us)?,sql_integer(expires)?,sql_integer(deadline)?,session_csrf])?;
        audit(
            &tx,
            &v.host,
            Some(v),
            AdminAction::Login,
            id,
            None,
            now.utc_us,
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn web_revoke(
        &mut self,
        auth: &WebMutationAuth,
        target_uid: &[u8; 33],
        id: Option<&[u8; 16]>,
        now: AdminMoment,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ctx = mutation(&tx, auth, now, false)?;
        if target_uid != &ctx.credential.uid && !ctx.operator {
            return Err(Error::AuthorizationChanged);
        }
        local_user(&tx, &ctx.credential.host, target_uid)?;
        if let Some(id) = id {
            if tx.execute("UPDATE web_admin_sessions SET revoked_at_us=coalesce(revoked_at_us,?3) WHERE record_id=?1 AND uid=?2",params![id,target_uid,sql_integer(now.utc_us)?])?!=1{return Err(Error::NotFound("browser session"));}
        } else {
            tx.execute("UPDATE web_admin_sessions SET revoked_at_us=coalesce(revoked_at_us,?2) WHERE uid=?1",params![target_uid,sql_integer(now.utc_us)?])?;
            tx.execute(
                "UPDATE web_login_tickets SET state=2 WHERE uid=?1",
                [target_uid],
            )?;
            tx.execute("DELETE FROM web_login_confirmations WHERE ticket_hash IN (SELECT ticket_hash FROM web_login_tickets
             WHERE uid=?1)",[target_uid])?;
        }
        audit(
            &tx,
            &ctx.credential.host,
            Some(&ctx.credential),
            if id.is_some() {
                AdminAction::RevokeSession
            } else {
                AdminAction::RevokeAll
            },
            id.map(|v| v.as_slice()).unwrap_or(target_uid),
            None,
            now.utc_us,
        )?;
        tx.commit()?;
        Ok(())
    }
}
