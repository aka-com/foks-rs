use super::*;

impl Database {
    pub fn admin_set_grant(
        &mut self,
        host: &[u8; 33],
        uid: &[u8; 33],
        active: bool,
        reason: &str,
        now: u64,
    ) -> Result<HostAdminGrant> {
        if reason.is_empty() || reason.len() > 512 || reason.chars().any(char::is_control) {
            return Err(Error::Invalid("operator grant reason"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (_, username) = local_user(&tx, host, uid)?;
        tx.execute("INSERT INTO host_admin_grants VALUES(?1,?2,1,?3,?4,?4,?5) ON CONFLICT(host_id,uid) DO UPDATE SET
             active=excluded.active,revision=revision+1,changed_at_us=excluded.changed_at_us,reason=excluded.reason",params![host,uid,active,sql_integer(now)?,reason])?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM host_admin_grants WHERE host_id=?1 AND uid=?2",
            params![host, uid],
            |r| r.get(0),
        )?;
        audit(
            &tx,
            host,
            None,
            if active {
                AdminAction::Grant
            } else {
                AdminAction::RevokeGrant
            },
            uid,
            Some(unsigned(revision)?),
            now,
        )?;
        tx.commit()?;
        Ok(HostAdminGrant {
            host: *host,
            uid: *uid,
            username,
            revision: unsigned(revision)?,
            active,
            reason: reason.into(),
        })
    }
}
impl ReadDatabase {
    pub fn admin_operator_count(&self) -> Result<u64> {
        unsigned(self.connection.query_row(
            "SELECT count(*) FROM host_admin_grants WHERE active=1",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn admin_grants(&self, after: &[u8]) -> Result<Vec<HostAdminGrant>> {
        if ![0, 33].contains(&after.len()) {
            return Err(Error::Invalid("grant cursor"));
        }
        let mut q=self.connection.prepare("SELECT g.host_id,g.uid,u.username_utf8,g.revision,g.active,g.reason FROM host_admin_grants g JOIN
             users u ON u.uid=g.uid WHERE g.uid>?1 ORDER BY g.uid LIMIT 100")?;
        let rows = q
            .query_map([after], |r| {
                Ok(HostAdminGrant {
                    host: r.get(0)?,
                    uid: r.get(1)?,
                    username: String::from_utf8(r.get(2)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    revision: r.get::<_, i64>(3)? as u64,
                    active: r.get(4)?,
                    reason: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
