use super::*;

impl ReadSnapshot<'_> {
    pub fn web_overview(
        &self,
        hash: &[u8; 32],
        now: AdminMoment,
    ) -> Result<(WebContext, Option<WebOverview>)> {
        let ctx = self.web_context(hash, now)?;
        let counts = if ctx.operator {
            Some(self.connection().query_row("SELECT (SELECT count(*) FROM users),(SELECT count(*) FROM host_admin_grants WHERE active=1),(SELECT
             count(*) FROM signup_invites),(SELECT count(*) FROM web_admin_sessions WHERE instance_epoch=?1 AND
             revoked_at_us IS NULL AND expires_at_us>?2 AND deadline_elapsed_us>?3)",params![now.epoch,sql_integer(now.utc_us)?,sql_integer(now.elapsed_us)?],|r|Ok(WebOverview{accounts:r.get::<_,i64>(0)? as u64,operators:r.get::<_,i64>(1)? as u64,invites:r.get::<_,i64>(2)? as u64,sessions:r.get::<_,i64>(3)? as u64}))?)
        } else {
            None
        };
        Ok((ctx, counts))
    }

    pub fn web_check_ticket(
        &self,
        hash: &[u8; 32],
        uid: &[u8; 33],
        now: AdminMoment,
    ) -> Result<()> {
        let a = load(self.connection(), hash, false)?;
        authorize(self.connection(), &a, now)?;
        if a.credential.uid != *uid {
            return Err(Error::WebWrongUser);
        }
        Ok(())
    }
    pub fn web_confirmation(&self, binding: &[u8; 32], now: AdminMoment) -> Result<WebContext> {
        let (ticket, _) = confirmation(self.connection(), binding, now)?;
        authorize(
            self.connection(),
            &load(self.connection(), &ticket, false)?,
            now,
        )
    }
    pub fn web_context(&self, hash: &[u8; 32], now: AdminMoment) -> Result<WebContext> {
        authorize(
            self.connection(),
            &load(self.connection(), hash, true)?,
            now,
        )
    }
    pub fn web_sessions(
        &self,
        hash: &[u8; 32],
        target: Option<&[u8; 33]>,
        after: &[u8],
        now: AdminMoment,
    ) -> Result<(WebContext, Vec<WebSessionMetadata>)> {
        if ![0, 16].contains(&after.len()) {
            return Err(Error::Invalid("session cursor"));
        }
        let ctx = self.web_context(hash, now)?;
        let uid = target.unwrap_or(&ctx.credential.uid);
        if uid != &ctx.credential.uid && !ctx.operator {
            return Err(Error::AuthorizationChanged);
        }
        local_user(self.connection(), &ctx.credential.host, uid)?;
        let mut q=self.connection().prepare("SELECT record_id,uid,credential_id,created_at_us,expires_at_us,revoked_at_us IS NOT NULL FROM
             web_admin_sessions WHERE uid=?1 AND instance_epoch=?2 AND record_id>?3 ORDER BY record_id LIMIT 100")?;
        let rows = q
            .query_map(params![uid, now.epoch, after], |r| {
                Ok(WebSessionMetadata {
                    id: r.get(0)?,
                    uid: r.get(1)?,
                    credential: r.get(2)?,
                    created_at_us: r.get::<_, i64>(3)? as u64,
                    expires_at_us: r.get::<_, i64>(4)? as u64,
                    revoked: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((ctx, rows))
    }
    pub fn web_invites(
        &self,
        hash: &[u8; 32],
        after: &[u8],
        now: AdminMoment,
    ) -> Result<(WebContext, InvitePolicy, Vec<InviteSnapshot>)> {
        let ctx = self.web_context(hash, now)?;
        if !ctx.operator {
            return Err(Error::AuthorizationChanged);
        }
        Ok((
            ctx,
            crate::invites::invite_policy(self.connection())?,
            crate::invites::page(self.connection(), after, 100)?,
        ))
    }
    pub fn web_audit(
        &self,
        hash: &[u8; 32],
        after: u64,
        now: AdminMoment,
    ) -> Result<(WebContext, Vec<AdminAuditEvent>)> {
        let ctx = self.web_context(hash, now)?;
        if !ctx.operator {
            return Err(Error::AuthorizationChanged);
        }
        let mut q=self.connection().prepare("SELECT event_id,actor_uid,action,target_id,resource_revision,occurred_at_us FROM admin_audit WHERE
             event_id>?1 ORDER BY event_id LIMIT 100")?;
        let rows = q
            .query_map([sql_integer(after)?], |r| {
                Ok(AdminAuditEvent {
                    id: r.get::<_, i64>(0)? as u64,
                    actor_uid: r.get(1)?,
                    action: r.get(2)?,
                    target: r.get(3)?,
                    revision: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                    occurred_at_us: r.get::<_, i64>(5)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((ctx, rows))
    }
}
