//! Schema-owned read invalidation, separate from hosted authority intent receipts.
//! A schema proof belongs to the Database/ReadDatabase that created it. Snapshots
//! borrow that proof; callers cannot attach a proof from a different connection.
use std::sync::{Mutex, OnceLock};

use foks_schema_contract::{Contract, ContractObject, MatchPolicy};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

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

const SCHEMA_OBJECTS_SQL: &str = "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE typeof(name) != 'text' OR substr(name,1,7) != 'sqlite_' COLLATE NOCASE ORDER BY type,name";
fn contract_error(_: foks_schema_contract::ContractError) -> Error {
    Error::ReadDependencySchema("schema object differs from qualified contract")
}
fn object(row: &rusqlite::Row<'_>) -> Result<ContractObject<'static>> {
    Ok(ContractObject::owned(
        row.budgeted_get(0)?,
        row.budgeted_get(1)?,
        row.budgeted_get(2)?,
        row.budgeted_get(3)?,
    ))
}
fn reference_contract(connection: &Connection) -> Result<Contract<'static>> {
    let mut statement = connection.prepare(SCHEMA_OBJECTS_SQL)?;
    let mut rows = statement.query([])?;
    let mut objects = Vec::new();
    while let Some(row) = rows.next()? {
        objects.push(object(row)?);
    }
    Contract::new(objects).map_err(contract_error)
}

fn expected_schema() -> Result<&'static Contract<'static>> {
    static EXPECTED: OnceLock<Contract<'static>> = OnceLock::new();
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
    let schema = reference_contract(&connection)?;
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
    let expected = expected_schema()?;
    let mut verifier = expected.verifier(MatchPolicy::NativeRequired);
    let mut statement = transaction.prepare(SCHEMA_OBJECTS_SQL)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let object = object(row)?;
        verifier
            .observe(object.as_object())
            .map_err(contract_error)?;
    }
    verifier.finish().map_err(contract_error)?;
    read(transaction)?;
    Ok(ReadDependencySchema {
        cookie,
        hash: *expected.hash(),
    })
}

#[cfg(test)]
mod tests;

// The allocation-heavy historical algorithm is a test oracle only. It cannot
// be invoked by serving admission or any transaction guard.
#[cfg(test)]
#[path = "../../foks-schema-contract/src/test_oracle.rs"]
mod test_oracle;
#[cfg(test)]
type TestSchema = std::collections::BTreeMap<(String, String, String), Vec<Vec<u8>>>;
#[cfg(test)]
fn schema_objects(connection: &Connection) -> Result<TestSchema> {
    let mut statement = connection.prepare(SCHEMA_OBJECTS_SQL)?;
    let mut rows = statement.query([])?;
    let mut result = std::collections::BTreeMap::new();
    while let Some(row) = rows.next()? {
        let object = object(row)?;
        let view = object.as_object();
        // Every historical fixture that inspects this schema also compares the
        // independent old digest with the new borrowed-token implementation.
        let old_hash = test_oracle::hash([(view.kind, view.name, view.table, view.sql)])
            .map_err(Error::ReadDependencySchema)?;
        let new_hash =
            Contract::new(vec![ContractObject::borrowed(view)]).map_err(contract_error)?;
        assert_eq!(*new_hash.hash(), old_hash);
        result.insert(
            (view.kind.into(), view.name.into(), view.table.into()),
            test_oracle::tokens(view.sql).map_err(Error::ReadDependencySchema)?,
        );
    }
    Ok(result)
}
