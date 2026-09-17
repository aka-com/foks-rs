//! Import authority is local, durable and revision-tracked. Proof verification lives
//! above this repository; only its complete account set can clear the profile gate.
use crate::{Error, HardStateStore, Result};
use rusqlite::{params, OptionalExtension as _};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ImportAccountKind {
    Software = 0,
    Yubi = 1,
    Bot = 2,
}
impl ImportAccountKind {
    fn parse(value: u8) -> rusqlite::Result<Self> {
        match value {
            0 => Ok(Self::Software),
            1 => Ok(Self::Yubi),
            2 => Ok(Self::Bot),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportAccount {
    pub alias: String,
    pub kind: ImportAccountKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportReadiness {
    pub archive_id: [u8; 16],
    pub attempt: [u8; 32],
    pub required: bool,
    pub accounts: Vec<ImportAccount>,
}
pub struct VerifiedImportAccount {
    pub account: ImportAccount,
    pub user_sequence: u64,
    pub merkle_epoch: u64,
}

impl HardStateStore {
    pub fn import_readiness(&self) -> Result<Option<ImportReadiness>> {
        let row = self
            .connection
            .query_row(
                "SELECT archive_id,attempt_nonce,required FROM import_readiness WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((archive_id, attempt, required)) = row else {
            return Ok(None);
        };
        let accounts = self
            .connection
            .prepare("SELECT alias,kind FROM import_accounts ORDER BY alias LIMIT 8193")?
            .query_map([], |r| {
                Ok(ImportAccount {
                    alias: r.get(0)?,
                    kind: ImportAccountKind::parse(r.get(1)?)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if accounts.len() > 8192 {
            return Err(Error::ImportReadiness("account limit exceeded"));
        }
        Ok(Some(ImportReadiness {
            archive_id,
            attempt,
            required,
            accounts,
        }))
    }
    pub fn requires_import_verification(&self) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT required FROM import_readiness WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }
    /// Called only after typed rekey under root maintenance exclusion. Re-import
    /// starts a new attempt; prior local verification never transfers authority.
    pub fn install_import_readiness(
        &mut self,
        archive_id: [u8; 16],
        attempt: [u8; 32],
        accounts: &[ImportAccount],
    ) -> Result<()> {
        if accounts.len() > 8192 {
            return Err(Error::ImportReadiness("account limit exceeded"));
        }
        let mut seen = BTreeSet::new();
        for account in accounts {
            if account.alias.is_empty()
                || account.alias.len() > 64
                || !account
                    .alias
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                || !seen.insert(&account.alias)
            {
                return Err(Error::ImportReadiness("invalid or duplicate account"));
            }
        }
        let tx = self.write_transaction()?;
        tx.execute("DELETE FROM import_accounts", [])?;
        tx.execute("INSERT INTO import_readiness(singleton,archive_id,attempt_nonce,required) VALUES(1,?1,?2,1) ON CONFLICT(singleton) DO UPDATE SET archive_id=excluded.archive_id,attempt_nonce=excluded.attempt_nonce,required=1",params![archive_id,attempt])?;
        for account in accounts {
            tx.execute(
                "INSERT INTO import_accounts(alias,kind) VALUES(?1,?2)",
                params![account.alias, account.kind as u8],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    /// The verifier supplies every account from this attempt in one successful
    /// pass. Partial, stale-nonce or duplicate reports cannot clear the gate.
    pub fn complete_import_verification(
        &mut self,
        attempt: [u8; 32],
        verified: &[VerifiedImportAccount],
    ) -> Result<()> {
        if verified.len() > 8192 {
            return Err(Error::ImportReadiness("account limit exceeded"));
        }
        let tx = self.write_transaction()?;
        let nonce: Option<[u8; 32]> = tx
            .query_row(
                "SELECT attempt_nonce FROM import_readiness WHERE singleton=1 AND required=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if nonce != Some(attempt) {
            return Err(Error::ImportReadiness("verification attempt changed"));
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM import_accounts", [], |r| r.get(0))?;
        if count != verified.len() as i64 {
            return Err(Error::ImportReadiness("verification omitted accounts"));
        }
        let mut seen = BTreeSet::new();
        for proof in verified {
            if !seen.insert(&proof.account.alias) {
                return Err(Error::ImportReadiness("verification duplicated an account"));
            }
            let changed=tx.execute("UPDATE import_accounts SET verified_user_sequence=?3,verified_merkle_epoch=?4 WHERE alias=?1 AND kind=?2",params![proof.account.alias,proof.account.kind as u8,crate::sqlite_integer("verified user sequence",proof.user_sequence)?,crate::sqlite_integer("verified Merkle epoch",proof.merkle_epoch)?])?;
            if changed != 1 {
                return Err(Error::ImportReadiness("verification account changed"));
            }
        }
        tx.execute(
            "UPDATE import_readiness SET required=0 WHERE singleton=1 AND attempt_nonce=?1",
            [attempt],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_is_durable_revision_tracked_and_cannot_clear_partially() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard.sqlite3");
        let mut db = HardStateStore::open(&path).unwrap();
        let before = db.metadata().unwrap();
        let accounts = vec![
            ImportAccount {
                alias: "one".into(),
                kind: ImportAccountKind::Software,
            },
            ImportAccount {
                alias: "two".into(),
                kind: ImportAccountKind::Yubi,
            },
        ];
        db.install_import_readiness([1; 16], [2; 32], &accounts)
            .unwrap();
        let after = db.metadata().unwrap();
        assert!(after.revision > before.revision);
        assert_ne!(after.write_token, before.write_token);
        assert_eq!(after.database_id, before.database_id);
        drop(db);
        let mut db = HardStateStore::open(&path).unwrap();
        assert!(db.requires_import_verification().unwrap());
        let proofs = accounts
            .into_iter()
            .map(|account| VerifiedImportAccount {
                account,
                user_sequence: 10,
                merkle_epoch: 20,
            })
            .collect::<Vec<_>>();
        assert!(db.complete_import_verification([3; 32], &proofs).is_err());
        assert!(db
            .complete_import_verification([2; 32], &proofs[..1])
            .is_err());
        assert!(db.requires_import_verification().unwrap());
        db.complete_import_verification([2; 32], &proofs).unwrap();
        assert!(!db.requires_import_verification().unwrap());
        db.install_import_readiness([1; 16], [4; 32], &[]).unwrap();
        assert!(db.requires_import_verification().unwrap());
        assert!(db.complete_import_verification([2; 32], &[]).is_err());
    }
}
