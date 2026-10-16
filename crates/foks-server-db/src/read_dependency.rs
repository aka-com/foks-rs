//! Schema-owned read invalidation, separate from hosted authority intent receipts.
//! A schema proof belongs to the Database/ReadDatabase that created it. Snapshots
//! borrow that proof; callers cannot attach a proof from a different connection.
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::allocation::RowAllocationExt as _;
use crate::{Database, Error, ReadDatabase, ReadSnapshot, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadDependency {
    pub incarnation: [u8; 32],
    pub revision: u64,
}

/// A private, verified contract cached at connection admission. Getters support
/// the private hosted activation binding; this value does not itself authorize
/// a caller or supply cross-host ownership.
#[derive(Debug)]
pub struct ReadDependencySchema {
    cookie: i64,
    hash: [u8; 32],
}
impl ReadDependencySchema {
    pub fn schema_cookie(&self) -> i64 {
        self.cookie
    }
    pub fn contract_hash(&self) -> &[u8; 32] {
        &self.hash
    }

    fn check(&self, connection: &Connection) -> Result<()> {
        if connection.is_autocommit() {
            return Err(Error::ReadDependencySchema(
                "coordinate requires a transaction",
            ));
        }
        let cookie: i64 =
            connection.pragma_query_value(None, "schema_version", |r| r.budgeted_get(0))?;
        if cookie != self.cookie {
            return Err(Error::ReadDependencySchema(
                "schema changed since verification",
            ));
        }
        Ok(())
    }
}

fn read(connection: &Connection) -> Result<ReadDependency> {
    let row: Option<(Vec<u8>, i64)> = connection
        .query_row(
            "SELECT incarnation,revision FROM metadata_read_dependency WHERE singleton=1",
            [],
            |r| Ok((r.budgeted_get(0)?, r.budgeted_get(1)?)),
        )
        .optional()?;
    let Some((incarnation, revision)) = row else {
        return Err(Error::ReadDependencySchema("coordinate missing"));
    };
    let incarnation: [u8; 32] = incarnation
        .try_into()
        .map_err(|_| Error::ReadDependencySchema("invalid incarnation"))?;
    if incarnation == [0; 32] || revision < 1 {
        return Err(Error::ReadDependencySchema("invalid coordinate"));
    }
    Ok(ReadDependency {
        incarnation,
        revision: revision as u64,
    })
}

impl Database {
    pub fn read_dependency_schema(&self) -> &ReadDependencySchema {
        &self.dependency_schema
    }
    pub fn read_dependency(&self) -> Result<ReadDependency> {
        let transaction = self.connection.unchecked_transaction()?;
        self.dependency_schema.check(&transaction)?;
        read(&transaction)
    }

    /// Rotate only a private backup/restore copy under lifecycle exclusion,
    /// before checkpoint/publication. Never call on a serving database. Like
    /// clear_private_chunk_storage_binding, this is not a normal request API.
    /// Entropy is generated here, never selected by a public caller.
    pub fn rotate_private_read_dependency(
        &mut self,
        expected: &ReadDependency,
    ) -> Result<ReadDependency> {
        let incarnation =
            crate::schema::read_dependency::fresh_incarnation(Some(&expected.incarnation))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.dependency_schema.check(&transaction)?;
        if read(&transaction)? != *expected {
            return Err(Error::ReadDependencyChanged);
        }
        let revision = i64::try_from(expected.revision).map_err(|_| Error::IntegerRange)?;
        if transaction.execute(
            "UPDATE metadata_read_dependency SET incarnation=?1,revision=1 WHERE singleton=1 AND incarnation=?2 AND revision=?3",
            params![incarnation.as_slice(), expected.incarnation.as_slice(), revision],
        )? != 1 {
            return Err(Error::ReadDependencyChanged);
        }
        let next = read(&transaction)?;
        transaction.commit()?;
        Ok(next)
    }
}
impl ReadDatabase {
    pub fn read_dependency_schema(&self) -> &ReadDependencySchema {
        &self.dependency_schema
    }
    pub fn read_dependency(&self) -> Result<ReadDependency> {
        self.snapshot()?.read_dependency()
    }
}
impl ReadSnapshot<'_> {
    pub fn read_dependency(&self) -> Result<ReadDependency> {
        self.dependency_schema.check(self.connection())?;
        read(self.connection())
    }
    pub fn require_read_dependency(&self, expected: &ReadDependency) -> Result<()> {
        if self.read_dependency()? == *expected {
            Ok(())
        } else {
            Err(Error::ReadDependencyChanged)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ObjectKey {
    kind: String,
    name: String,
    table: String,
}
type Schema = BTreeMap<ObjectKey, Vec<Vec<u8>>>;

/// Tokenize SQL rather than deleting whitespace: literal bytes remain distinct,
/// and explicitly qualified identifier quotes from historical ALTER are normalized.
/// Unknown/truncated syntax fails closed. Actual schema definitions were already
/// parsed by SQLite; this lexer is only an equality canonicalizer.
fn sql_tokens(sql: &str) -> Result<Vec<Vec<u8>>> {
    let bytes = sql.as_bytes();
    let mut at = 0;
    let mut tokens = Vec::new();
    while at < bytes.len() {
        let first = bytes[at];
        if first.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"--") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at += 2;
            while at < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            if at == bytes.len() {
                return Err(Error::ReadDependencySchema("unterminated schema comment"));
            }
            at += 2;
            continue;
        }
        if matches!(first, b'\'' | b'"' | b'`' | b'[') {
            let start = at;
            let end = if first == b'[' { b']' } else { first };
            at += 1;
            let mut text = Vec::new();
            loop {
                let byte = *bytes
                    .get(at)
                    .ok_or(Error::ReadDependencySchema("unterminated schema quote"))?;
                at += 1;
                if byte == end {
                    if end != b']' && bytes.get(at) == Some(&end) {
                        text.push(end);
                        at += 1;
                        continue;
                    }
                    break;
                }
                text.push(byte);
            }
            // SQLite ALTER TABLE historically quotes precisely these renamed
            // table/column identifiers. Qualify those spellings explicitly;
            // do not treat arbitrary double-quoted tokens as identifiers since
            // SQLite can also interpret them as string literals (DQS).
            text.make_ascii_lowercase();
            let migrated_identifier = matches!(
                text.as_slice(),
                b"chunk_objects"
                    | b"chunk_object_attempts"
                    | b"kv_file_chunks"
                    | b"blocked_reason"
                    | b"source_hash"
            );
            if first != b'\'' && migrated_identifier {
                tokens.push(text);
            } else {
                tokens.push(bytes[start..at].to_vec());
            }
        } else if first.is_ascii_alphanumeric() || first == b'_' {
            let start = at;
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            tokens.push(bytes[start..at].to_ascii_lowercase());
        } else {
            tokens.push(vec![first]);
            at += 1;
        }
    }
    // sqlite_schema normally omits the last semicolon; do not alter inner ones.
    if tokens.last().is_some_and(|t| t == b";") {
        tokens.pop();
    }
    Ok(tokens)
}

fn schema_objects(connection: &Connection) -> Result<Schema> {
    let mut statement = connection.prepare(
        "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE substr(name,1,7) != 'sqlite_' COLLATE NOCASE AND sql IS NOT NULL ORDER BY type,name",
    )?;
    let mut rows = statement.query([])?;
    let mut objects = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let kind = row.budgeted_get(0)?;
        let name = row.budgeted_get(1)?;
        let table = row.budgeted_get(2)?;
        let sql: String = row.budgeted_get(3)?;
        objects.insert(ObjectKey { kind, name, table }, sql_tokens(&sql)?);
    }
    Ok(objects)
}

fn expected_schema() -> Result<&'static Schema> {
    static EXPECTED: OnceLock<Schema> = OnceLock::new();
    static BUILD: Mutex<()> = Mutex::new(());
    if let Some(schema) = EXPECTED.get() {
        return Ok(schema);
    }
    let _building = BUILD
        .lock()
        .map_err(|_| Error::ReadDependencySchema("schema build poisoned"))?;
    if let Some(schema) = EXPECTED.get() {
        return Ok(schema);
    }
    // Exact shared DDL, with a non-serving reference coordinate. No Database
    // opener, entropy dependency or recursive verification. Failed construction
    // (including caller allocation refusal) does not poison future admissions.
    let connection = crate::schema::reference_connection()?;
    let schema = schema_objects(&connection)?;
    let _ = EXPECTED.set(schema);
    EXPECTED
        .get()
        .ok_or(Error::ReadDependencySchema("canonical schema unavailable"))
}

pub(crate) fn verify(connection: &Connection) -> Result<ReadDependencySchema> {
    let transaction = connection.unchecked_transaction()?;
    verify_in_transaction(&transaction)
}

pub(crate) fn verify_in_transaction(transaction: &Connection) -> Result<ReadDependencySchema> {
    if transaction.is_autocommit() {
        return Err(Error::ReadDependencySchema(
            "schema proof requires a transaction",
        ));
    }
    let cookie: i64 =
        transaction.pragma_query_value(None, "schema_version", |r| r.budgeted_get(0))?;
    let actual = schema_objects(transaction)?;
    let expected = expected_schema()?;
    // Verify the complete existing schema, including projections, accounting,
    // checks, FKs and cascades. New counter hooks alone cannot certify any of it.
    for (key, value) in expected {
        if actual.get(key) != Some(value) {
            return Err(Error::ReadDependencySchema(
                "schema object differs from qualified contract",
            ));
        }
    }
    for key in actual.keys() {
        if !expected.contains_key(key)
            && (key
                .name
                .get(.."metadata_read_".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("metadata_read_"))
                || key.kind == "trigger"
                    && expected
                        .keys()
                        .any(|k| k.kind == "table" && k.name.eq_ignore_ascii_case(&key.table)))
        {
            return Err(Error::ReadDependencySchema(
                "unexpected hook on authoritative table",
            ));
        }
    }
    read(transaction)?;
    let mut hash = Sha256::new();
    hash.update(b"foks-read-dependency-schema-v1\0");
    for (key, tokens) in expected {
        for text in [&key.kind, &key.name, &key.table] {
            hash.update((text.len() as u64).to_le_bytes());
            hash.update(text.as_bytes());
        }
        hash.update((tokens.len() as u64).to_le_bytes());
        for token in tokens {
            hash.update((token.len() as u64).to_le_bytes());
            hash.update(token);
        }
    }
    Ok(ReadDependencySchema {
        cookie,
        hash: hash.finalize().into(),
    })
}

#[cfg(test)]
mod tests;
