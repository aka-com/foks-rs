use super::*;

impl Database {
    pub fn web_prepare_invite(
        &mut self,
        auth: &WebMutationAuth,
        nonce: &[u8; 32],
        now: AdminMoment,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ctx = mutation(&tx, auth, now, true)?;
        cleanup(&tx, now)?;
        let(all,own):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(session_hash=?1 AND result_id IS NULL),0) FROM web_admin_nonces",[auth.session_hash],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if all >= 8192 || own >= 8 {
            return Err(Error::Capacity("invite forms"));
        }
        let admit = now
            .elapsed_us
            .checked_add(WEB_TICKET_US)
            .ok_or(Error::IntegerRange)?
            .min(ctx.deadline_elapsed_us);
        tx.execute(
            "INSERT INTO web_admin_nonces VALUES(?1,?2,?3,?4,?5,NULL)",
            params![
                nonce,
                auth.session_hash,
                now.epoch,
                sql_integer(admit)?,
                sql_integer(ctx.deadline_elapsed_us)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn web_issue_invite(
        &mut self,
        auth: &WebMutationAuth,
        invite: WebInvite,
        now: AdminMoment,
    ) -> Result<WebInviteResult> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ctx = mutation(&tx, auth, now, true)?;
        let (admit,retain,result):(i64,i64,Option<[u8;16]>)=tx.query_row("SELECT admit_before_elapsed_us,receipt_retain_until_elapsed_us,result_id FROM web_admin_nonces WHERE
             nonce_hash=?1 AND session_hash=?2 AND instance_epoch=?3",params![invite.nonce_hash,auth.session_hash,now.epoch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(Error::OperationExpired)?;
        if unsigned(retain)? <= now.elapsed_us {
            return Err(Error::OperationExpired);
        }
        if let Some(id) = result {
            return Ok(WebInviteResult::AlreadyCreated(id));
        }
        if unsigned(admit)? <= now.elapsed_us {
            return Err(Error::OperationExpired);
        }
        crate::invites::issue(
            &tx,
            &invite.id,
            &invite.code_hash,
            crate::InviteKind::Standard,
            Some(&ctx.credential.uid),
            Some(1),
            None,
            now.utc_us,
        )?;
        tx.execute(
            "UPDATE web_admin_nonces SET result_id=?2 WHERE nonce_hash=?1 AND result_id IS NULL",
            params![invite.nonce_hash, invite.id],
        )?;
        audit(
            &tx,
            &ctx.credential.host,
            Some(&ctx.credential),
            AdminAction::IssueInvite,
            &invite.id,
            Some(1),
            now.utc_us,
        )?;
        tx.commit()?;
        Ok(WebInviteResult::Created(invite.id))
    }
    pub fn web_disable_invite(
        &mut self,
        auth: &WebMutationAuth,
        id: &[u8; 16],
        revision: u64,
        now: AdminMoment,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ctx = mutation(&tx, auth, now, true)?;
        crate::invites::disable_id_cas(&tx, id, revision, now.utc_us)?;
        audit(
            &tx,
            &ctx.credential.host,
            Some(&ctx.credential),
            AdminAction::DisableInvite,
            id,
            Some(revision.checked_add(1).ok_or(Error::IntegerRange)?),
            now.utc_us,
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn web_set_policy(
        &mut self,
        auth: &WebMutationAuth,
        regime: InviteRegime,
        revision: u64,
        now: AdminMoment,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ctx = mutation(&tx, auth, now, true)?;
        crate::invites::set_regime_cas(&tx, regime, revision)?;
        audit(
            &tx,
            &ctx.credential.host,
            Some(&ctx.credential),
            AdminAction::SignupPolicy,
            &[],
            Some(revision.checked_add(1).ok_or(Error::IntegerRange)?),
            now.utc_us,
        )?;
        tx.commit()?;
        Ok(())
    }
}
