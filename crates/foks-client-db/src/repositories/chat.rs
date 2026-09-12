use crate::{sqlite_integer, ChatLimits, Error, HardStateStore, Result};
use rusqlite::{params, OptionalExtension};

const ANCHOR_BY_SEQUENCE: &str = "
    SELECT sequence, message_id, digest
    FROM chat_anchors
    WHERE host_id = ?1
      AND uid = ?2
      AND team_id = ?3
      AND channel_id = ?4
      AND sequence = ?5";
const ANCHOR_BY_ID: &str = "
    SELECT sequence, message_id, digest
    FROM chat_anchors
    WHERE host_id = ?1
      AND uid = ?2
      AND team_id = ?3
      AND channel_id = ?4
      AND message_id = ?5";

fn decode_anchor(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatAnchor> {
    Ok(ChatAnchor {
        sequence: row.get::<_, i64>(0)? as u64,
        id: row.get(1)?,
        digest: row.get(2)?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatScope {
    pub host: Vec<u8>,
    pub uid: Vec<u8>,
    pub team: Vec<u8>,
    pub channel: [u8; 16],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ChatOperationState {
    Prepared = 0,
    Uncertain = 1,
    Confirmed = 2,
    Rejected = 3,
    Cancelled = 4,
}
impl ChatOperationState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Rejected | Self::Cancelled)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ChatOperationKind {
    Create = 0,
    Send = 1,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatOperation {
    pub id: [u8; 16],
    pub scope: ChatScope,
    pub kind: ChatOperationKind,
    pub state: ChatOperationState,
    pub request_hash: [u8; 32],
    pub scan_cursor: u64,
    pub receipt: Option<Vec<u8>>,
    pub rejection_code: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatAnchor {
    pub sequence: u64,
    pub id: [u8; 16],
    pub digest: [u8; 32],
}
/// Local submission identity and a keyed commitment to the original input.
#[derive(Clone, Copy)]
pub struct ChatSubmission {
    pub id: [u8; 16],
    pub input_mac: [u8; 32],
}

impl HardStateStore {
    pub fn chat_submission(
        &self,
        scope: &ChatScope,
        submission: &ChatSubmission,
    ) -> Result<Option<ChatOperation>> {
        let row: Option<([u8; 16], [u8; 32])> = self
            .connection
            .query_row(
                "SELECT operation_id, input_mac
                 FROM chat_submissions
                 WHERE host_id = ?1
                   AND uid = ?2
                   AND team_id = ?3
                   AND submission_id = ?4",
                params![scope.host, scope.uid, scope.team, submission.id.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match row {
            Some((id, mac)) if mac == submission.input_mac => self.chat_operation(&id),
            Some(_) => Err(Error::ChatConflict(
                "submission ID reused with different input",
            )),
            None => Ok(None),
        }
    }

    pub fn chat_record(&mut self, op: &ChatOperation) -> Result<()> {
        self.chat_record_submission(op, None)
    }
    pub fn chat_record_submission(
        &mut self,
        op: &ChatOperation,
        submission: Option<&ChatSubmission>,
    ) -> Result<()> {
        self.chat_record_submission_with_material(op, submission, || Ok(()))
    }

    /// Validate and stage the ledger rows before persisting protected material.
    /// The callback must durably store material before returning, must not access
    /// this database, and runs only for a new operation, under the writer lock.
    /// Failure rolls back the ledger. A crash after the callback can still leave
    /// orphan material; never delete it on an ambiguous commit outcome.
    pub fn chat_record_submission_with_material<E: From<Error>>(
        &mut self,
        op: &ChatOperation,
        submission: Option<&ChatSubmission>,
        persist_material: impl FnOnce() -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        if op.state != ChatOperationState::Prepared
            || op.receipt.is_some()
            || op.rejection_code.is_some()
        {
            return Err(Error::ChatOperationState("new operation is not prepared").into());
        }
        if let Some(old) = self.chat_operation(&op.id)? {
            if old != *op {
                return Err(Error::ChatConflict("operation ID reused").into());
            }
            if let Some(submission) = submission {
                if self.chat_submission(&op.scope, submission)? != Some(old) {
                    return Err(Error::ChatConflict("operation submission differs").into());
                }
            }
            return Ok(());
        }
        let tx = self.write_transaction()?;
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*)
             FROM chat_operations
             WHERE host_id = ?1
               AND uid = ?2
               AND team_id = ?3
               AND state IN (0, 1)",
                params![op.scope.host, op.scope.uid, op.scope.team],
                |row| row.get(0),
            )
            .map_err(Error::from)?;
        if count >= ChatLimits::PENDING_OPERATIONS as i64 {
            return Err(Error::ChatLimit("pending operation limit").into());
        }
        tx.execute(
            "INSERT INTO chat_operations (
                 operation_id, host_id, uid, team_id, channel_id, kind, state,
                 request_hash, scan_cursor, receipt, rejection_code
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, NULL, NULL)",
            params![
                op.id.as_slice(),
                op.scope.host,
                op.scope.uid,
                op.scope.team,
                op.scope.channel.as_slice(),
                op.kind as u8,
                op.request_hash.as_slice(),
                sqlite_integer("chat cursor", op.scan_cursor)?
            ],
        )
        .map_err(Error::from)?;
        if let Some(submission) = submission {
            tx.execute(
                "INSERT INTO chat_submissions (
                     host_id, uid, team_id, submission_id, input_mac, operation_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    op.scope.host,
                    op.scope.uid,
                    op.scope.team,
                    submission.id.as_slice(),
                    submission.input_mac.as_slice(),
                    op.id.as_slice()
                ],
            )
            .map_err(Error::from)?;
        }
        persist_material()?;
        tx.commit().map_err(Error::from)?;
        Ok(())
    }
    pub fn chat_operation(&self, id: &[u8; 16]) -> Result<Option<ChatOperation>> {
        Ok(self
            .connection
            .query_row(
                "SELECT host_id, uid, team_id, channel_id, kind, state,
                        request_hash, scan_cursor, receipt, rejection_code
                 FROM chat_operations
                 WHERE operation_id = ?1",
                [id.as_slice()],
                |row| {
                    Ok(ChatOperation {
                        id: *id,
                        scope: ChatScope {
                            host: row.get(0)?,
                            uid: row.get(1)?,
                            team: row.get(2)?,
                            channel: row.get(3)?,
                        },
                        kind: if row.get::<_, i64>(4)? == 0 {
                            ChatOperationKind::Create
                        } else {
                            ChatOperationKind::Send
                        },
                        state: match row.get::<_, i64>(5)? {
                            0 => ChatOperationState::Prepared,
                            1 => ChatOperationState::Uncertain,
                            2 => ChatOperationState::Confirmed,
                            3 => ChatOperationState::Rejected,
                            _ => ChatOperationState::Cancelled,
                        },
                        request_hash: row.get(6)?,
                        scan_cursor: row.get::<_, i64>(7)? as u64,
                        receipt: row.get(8)?,
                        rejection_code: row.get(9)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn chat_pending(&self, host: &[u8], uid: &[u8], team: &[u8]) -> Result<Vec<ChatOperation>> {
        let mut statement = self.connection.prepare(
            "SELECT operation_id
             FROM chat_operations
             WHERE host_id = ?1
               AND uid = ?2
               AND team_id = ?3
               AND state IN (0, 1)
             ORDER BY operation_id
             LIMIT ?4",
        )?;
        let ids = statement
            .query_map(
                params![host, uid, team, (ChatLimits::PENDING_OPERATIONS + 1) as i64],
                |row| row.get::<_, [u8; 16]>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if ids.len() > ChatLimits::PENDING_OPERATIONS {
            return Err(Error::ChatLimit("pending operation limit"));
        }
        ids.into_iter()
            .map(|id| {
                self.chat_operation(&id)?
                    .ok_or(Error::ChatNotFound("operation record not found"))
            })
            .collect()
    }
    /// This commits before a possible network write; a crash means uncertain.
    pub fn chat_begin(&mut self, id: &[u8; 16]) -> Result<()> {
        let count = self.connection.execute(
            "UPDATE chat_operations
             SET state = 1
             WHERE operation_id = ?1
               AND state = 0",
            [id.as_slice()],
        )?;
        if count != 1 {
            return Err(Error::ChatOperationState("operation already attempted"));
        }
        Ok(())
    }
    pub fn chat_progress(&mut self, id: &[u8; 16], cursor: u64) -> Result<()> {
        let count = self.connection.execute(
            "UPDATE chat_operations
             SET scan_cursor = MAX(scan_cursor, ?2)
             WHERE operation_id = ?1
               AND state = 1",
            params![id.as_slice(), sqlite_integer("chat cursor", cursor)?],
        )?;
        if count != 1 {
            return Err(Error::ChatOperationState("operation is not uncertain"));
        }
        Ok(())
    }
    pub fn chat_confirm(&mut self, id: &[u8; 16], receipt: &[u8]) -> Result<()> {
        let count = self.connection.execute(
            "UPDATE chat_operations
             SET state = 2, receipt = ?2
             WHERE operation_id = ?1
               AND (state = 1 OR (state = 2 AND receipt = ?2))",
            params![id.as_slice(), receipt],
        )?;
        if count != 1 {
            return Err(Error::ChatConflict("conflicting confirmation"));
        }
        Ok(())
    }
    /// Only known pre-commit rejections may use this transition.
    pub fn chat_reject(&mut self, id: &[u8; 16], code: i64) -> Result<()> {
        let count = self.connection.execute(
            "UPDATE chat_operations
             SET state = 3, rejection_code = ?2
             WHERE operation_id = ?1
               AND state = 1",
            params![id.as_slice(), code],
        )?;
        if count != 1 {
            return Err(Error::ChatOperationState("operation is not uncertain"));
        }
        Ok(())
    }
    /// Cancellation makes no claim about ambiguous delivery, so only Prepared
    /// operations may be cancelled. Repeating a cancellation is harmless.
    pub fn chat_cancel(&mut self, id: &[u8; 16]) -> Result<()> {
        let count = self.connection.execute(
            "UPDATE chat_operations
             SET state = 4
             WHERE operation_id = ?1
               AND state IN (0, 4)",
            [id.as_slice()],
        )?;
        if count != 1 {
            return Err(Error::ChatOperationState(
                "only prepared operations may be cancelled",
            ));
        }
        Ok(())
    }
    pub fn chat_anchor(&self, s: &ChatScope, seq: u64) -> Result<Option<ChatAnchor>> {
        Ok(self
            .connection
            .query_row(
                ANCHOR_BY_SEQUENCE,
                params![
                    s.host,
                    s.uid,
                    s.team,
                    s.channel.as_slice(),
                    sqlite_integer("chat sequence", seq)?
                ],
                decode_anchor,
            )
            .optional()?)
    }

    /// Accept a whole verified page atomically, then prune oldest evidence.
    pub fn chat_accept(&mut self, s: &ChatScope, anchors: &[ChatAnchor]) -> Result<()> {
        self.chat_accept_page(s, anchors, anchors)
    }
    /// All observations must agree with retained evidence, even when their
    /// content is unsupported. Only authenticated messages become new anchors.
    pub fn chat_accept_page(
        &mut self,
        s: &ChatScope,
        observed: &[ChatAnchor],
        anchors: &[ChatAnchor],
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let mut by_sequence = tx.prepare(ANCHOR_BY_SEQUENCE)?;
        let mut by_id = tx.prepare(ANCHOR_BY_ID)?;
        let mut insert = tx.prepare(
            "INSERT INTO chat_anchors (
                 host_id, uid, team_id, channel_id, sequence, message_id, digest
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT DO NOTHING",
        )?;
        let mut sequences = std::collections::BTreeMap::new();
        let mut ids = std::collections::BTreeMap::new();
        for a in observed {
            if sequences.insert(a.sequence, a).is_some_and(|old| old != a)
                || ids.insert(a.id, a).is_some_and(|old| old != a)
            {
                return Err(Error::ChatConflict("message mapping or envelope changed"));
            }
            for old in [
                by_sequence
                    .query_row(
                        params![
                            s.host,
                            s.uid,
                            s.team,
                            s.channel.as_slice(),
                            sqlite_integer("chat sequence", a.sequence)?
                        ],
                        decode_anchor,
                    )
                    .optional()?,
                by_id
                    .query_row(
                        params![s.host, s.uid, s.team, s.channel.as_slice(), a.id.as_slice()],
                        decode_anchor,
                    )
                    .optional()?,
            ]
            .into_iter()
            .flatten()
            {
                if old != *a {
                    return Err(Error::ChatConflict("message mapping or envelope changed"));
                }
            }
        }
        for a in anchors {
            if sequences.get(&a.sequence).copied() != Some(a) {
                return Err(Error::ChatConflict("anchor was not checked"));
            }
            insert.execute(params![
                s.host,
                s.uid,
                s.team,
                s.channel.as_slice(),
                sqlite_integer("chat sequence", a.sequence)?,
                a.id.as_slice(),
                a.digest.as_slice()
            ])?;
        }
        drop(by_sequence);
        drop(by_id);
        drop(insert);
        let cutoff: Option<i64> = tx
            .query_row(
                "SELECT sequence
                 FROM chat_anchors
                 WHERE host_id = ?1
                   AND uid = ?2
                   AND team_id = ?3
                   AND channel_id = ?4
                 ORDER BY sequence DESC
                 LIMIT 1 OFFSET ?5",
                params![
                    s.host,
                    s.uid,
                    s.team,
                    s.channel.as_slice(),
                    (ChatLimits::RETAINED_ANCHORS - 1) as i64
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(cutoff) = cutoff {
            tx.execute(
                "DELETE FROM chat_anchors
                 WHERE host_id = ?1
                   AND uid = ?2
                   AND team_id = ?3
                   AND channel_id = ?4
                   AND sequence < ?5",
                params![s.host, s.uid, s.team, s.channel.as_slice(), cutoff],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope(tag: u8) -> ChatScope {
        ChatScope {
            host: vec![1; 33],
            uid: vec![tag; 33],
            team: vec![3; 33],
            channel: [4; 16],
        }
    }
    fn prepared(id: u128) -> ChatOperation {
        ChatOperation {
            id: id.to_be_bytes(),
            scope: scope(2),
            kind: ChatOperationKind::Send,
            state: ChatOperationState::Prepared,
            request_hash: [8; 32],
            scan_cursor: 0,
            receipt: None,
            rejection_code: None,
        }
    }

    #[test]
    fn preparation_failure_rolls_back_both_rows_and_can_be_retried_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard");
        let mut db = HardStateStore::open(&path).unwrap();
        let op = prepared(1);
        let submission = ChatSubmission {
            id: [9; 16],
            input_mac: [10; 32],
        };
        let revision = db.metadata().unwrap().revision;
        let result = db.chat_record_submission_with_material(&op, Some(&submission), || {
            Err(Error::ChatConflict("injected protected write failure"))
        });
        assert!(result.is_err());
        drop(db);
        let mut db = HardStateStore::open(&path).unwrap();
        assert_eq!(db.metadata().unwrap().revision, revision);
        assert!(db.chat_operation(&op.id).unwrap().is_none());
        assert!(db
            .chat_submission(&op.scope, &submission)
            .unwrap()
            .is_none());
        db.chat_record_submission_with_material::<Error>(&op, Some(&submission), || Ok(()))
            .unwrap();
        assert_eq!(
            db.chat_submission(&op.scope, &submission).unwrap(),
            Some(op)
        );
    }

    #[test]
    fn rejected_and_duplicate_preparations_do_not_write_material() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = HardStateStore::open(&dir.path().join("hard")).unwrap();
        let op = prepared(1);
        let submission = ChatSubmission {
            id: [9; 16],
            input_mac: [10; 32],
        };
        db.chat_record_submission(&op, Some(&submission)).unwrap();
        db.chat_record_submission_with_material::<Error>(&op, Some(&submission), || {
            panic!("duplicate write")
        })
        .unwrap();
        let changed = ChatSubmission {
            id: [11; 16],
            ..submission
        };
        assert!(db
            .chat_record_submission_with_material::<Error>(&op, Some(&changed), || panic!(
                "unbound submission"
            ))
            .is_err());
        assert!(db
            .chat_record_submission_with_material::<Error>(
                &prepared(2),
                Some(&submission),
                || panic!("conflicting write")
            )
            .is_err());
        assert!(db.chat_operation(&prepared(2).id).unwrap().is_none());
        for id in 2..=ChatLimits::PENDING_OPERATIONS {
            db.chat_record(&prepared(id as u128)).unwrap();
        }
        assert!(matches!(
            db.chat_record_submission_with_material::<Error>(
                &prepared(u128::MAX),
                None,
                || panic!("over-capacity write")
            ),
            Err(Error::ChatLimit(_))
        ));
    }

    #[test]
    fn conflict_queries_use_exact_index_keys() {
        let dir = tempfile::tempdir().unwrap();
        let db = HardStateStore::open(&dir.path().join("hard")).unwrap();
        for (sql, key) in [
            (ANCHOR_BY_SEQUENCE, "sequence=?"),
            (ANCHOR_BY_ID, "message_id=?"),
        ] {
            let plan: String = db
                .connection
                .query_row(
                    &format!("EXPLAIN QUERY PLAN {sql}"),
                    params![
                        vec![1u8; 33],
                        vec![2u8; 33],
                        vec![3u8; 33],
                        vec![4u8; 16],
                        1
                    ],
                    |row| row.get(3),
                )
                .unwrap();
            assert!(plan.contains(key), "{plan}");
        }
    }
    #[test]
    fn durable_chat_boundaries_and_scope_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard");
        let mut db = HardStateStore::open(&path).unwrap();
        let initial = db.metadata().unwrap().revision;
        let op = ChatOperation {
            id: [7; 16],
            scope: scope(2),
            kind: ChatOperationKind::Send,
            state: ChatOperationState::Prepared,
            request_hash: [8; 32],
            scan_cursor: 2,
            receipt: None,
            rejection_code: None,
        };
        let submission = ChatSubmission {
            id: [9; 16],
            input_mac: [10; 32],
        };
        db.chat_record_submission(&op, Some(&submission)).unwrap();
        assert_eq!(
            db.chat_submission(&op.scope, &submission).unwrap(),
            Some(op.clone())
        );
        assert!(db
            .chat_submission(&scope(9), &submission)
            .unwrap()
            .is_none());
        let changed = ChatSubmission {
            input_mac: [11; 32],
            ..submission
        };
        assert!(matches!(
            db.chat_submission(&op.scope, &changed),
            Err(Error::ChatConflict(_))
        ));
        assert!(db.metadata().unwrap().revision > initial);
        let mut conflicting = op.clone();
        conflicting.id = [12; 16];
        let revision = db.metadata().unwrap().revision;
        assert!(db
            .chat_record_submission(&conflicting, Some(&submission))
            .is_err());
        assert!(db.chat_operation(&conflicting.id).unwrap().is_none());
        assert_eq!(db.metadata().unwrap().revision, revision);
        assert!(db
            .chat_pending(&op.scope.host, &scope(9).uid, &op.scope.team)
            .unwrap()
            .is_empty());
        db.chat_begin(&op.id).unwrap();
        drop(db);
        let mut db = HardStateStore::open(&path).unwrap();
        assert_eq!(
            db.chat_operation(&op.id).unwrap().unwrap().state,
            ChatOperationState::Uncertain
        );
        assert!(db.chat_begin(&op.id).is_err());
        assert_eq!(
            db.chat_submission(&op.scope, &submission)
                .unwrap()
                .unwrap()
                .id,
            op.id
        );
        db.chat_progress(&op.id, 100).unwrap();
        db.chat_progress(&op.id, 50).unwrap();
        assert_eq!(db.chat_operation(&op.id).unwrap().unwrap().scan_cursor, 100);
        db.chat_confirm(&op.id, b"receipt").unwrap();
        db.chat_confirm(&op.id, b"receipt").unwrap();
        assert!(db.chat_confirm(&op.id, b"other").is_err());
        assert!(db.chat_progress(&op.id, 200).is_err());
        assert!(db
            .chat_pending(&op.scope.host, &op.scope.uid, &op.scope.team)
            .unwrap()
            .is_empty());
    }
    #[test]
    fn unsupported_observations_check_retained_evidence_without_becoming_anchors() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = HardStateStore::open(&dir.path().join("hard")).unwrap();
        let s = scope(2);
        let a = ChatAnchor {
            sequence: 1,
            id: [1; 16],
            digest: [2; 32],
        };
        db.chat_accept(&s, std::slice::from_ref(&a)).unwrap();
        let mut changed = a.clone();
        changed.digest = [3; 32];
        assert!(db.chat_accept_page(&s, &[changed], &[]).is_err());
        let mut moved = a;
        moved.sequence = 2;
        assert!(db.chat_accept_page(&s, &[moved], &[]).is_err());
        let unknown = ChatAnchor {
            sequence: 2,
            id: [4; 16],
            digest: [5; 32],
        };
        db.chat_accept_page(&s, &[unknown], &[]).unwrap();
        assert!(db.chat_anchor(&s, 2).unwrap().is_none());
    }
    #[test]
    fn anchors_are_atomic_scoped_and_retained_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hard");
        let mut db = HardStateStore::open(&path).unwrap();
        let s = scope(2);
        let a = ChatAnchor {
            sequence: 1,
            id: [1; 16],
            digest: [2; 32],
        };
        db.chat_accept(&s, std::slice::from_ref(&a)).unwrap();
        let rev = db.metadata().unwrap().revision;
        let b = ChatAnchor {
            sequence: 2,
            id: [3; 16],
            digest: [4; 32],
        };
        let mut conflict = a.clone();
        conflict.digest = [9; 32];
        assert!(db.chat_accept(&s, &[b.clone(), conflict]).is_err());
        assert_eq!(db.metadata().unwrap().revision, rev);
        assert!(db.chat_anchor(&s, 2).unwrap().is_none());
        let mut moved = a.clone();
        moved.sequence = 2;
        assert!(db.chat_accept(&s, &[moved]).is_err());
        db.chat_accept(&scope(9), &[b]).unwrap();
        drop(db);
        let db = HardStateStore::open(&path).unwrap();
        assert_eq!(db.chat_anchor(&s, 1).unwrap(), Some(a));
        assert!(db.chat_anchor(&s, 2).unwrap().is_none());
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    #[test]
    fn anchor_retention_is_bounded_and_measured() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = HardStateStore::open(&dir.path().join("hard")).unwrap();
        let scope = ChatScope {
            host: vec![1; 33],
            uid: vec![2; 33],
            team: vec![3; 33],
            channel: [4; 16],
        };
        let anchors: Vec<_> = (1..=ChatLimits::RETAINED_ANCHORS as u64 + 1)
            .map(|sequence| {
                let mut id = [0; 16];
                id[..8].copy_from_slice(&sequence.to_be_bytes());
                ChatAnchor {
                    sequence,
                    id,
                    digest: [7; 32],
                }
            })
            .collect();
        let start = std::time::Instant::now();
        db.chat_accept(&scope, &anchors).unwrap();
        assert!(db.chat_anchor(&scope, 1).unwrap().is_none());
        assert!(db.chat_anchor(&scope, 2).unwrap().is_some());
        let count: i64 = db
            .connection
            .query_row("SELECT COUNT(*) FROM chat_anchors", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, ChatLimits::RETAINED_ANCHORS as i64);
        let pages: i64 = db
            .connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        let page_size: i64 = db
            .connection
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap();
        eprintln!(
            "10k retained chat anchors: {:?}, database bytes {}",
            start.elapsed(),
            pages * page_size
        );
    }
}
