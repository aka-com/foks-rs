//! Pure content comparison, not a database, storage, ownership or authorization
//! proof. Expected contracts must come from the caller's trusted artifact; no
//! object here can replace a connection-bound admission/transaction guard.
mod lex;
pub use lex::sql_equal;
use sha2::{Digest, Sha256};
use std::{borrow::Cow, cmp::Ordering, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractError {
    UnterminatedQuote,
    UnterminatedComment,
    LengthOverflow,
    ObjectOrder,
    DuplicateObject,
    InternalObject,
    MissingObject,
    ObjectChanged,
    ExtraObject,
    Poisoned,
}
impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "schema contract: {self:?}")
    }
}
impl std::error::Error for ContractError {}

#[derive(Clone, Copy, Debug)]
pub struct SchemaObject<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub table: &'a str,
    pub sql: &'a str,
}
impl SchemaObject<'_> {
    fn key(&self) -> (&str, &str, &str) {
        (self.kind, self.name, self.table)
    }
}
#[derive(Debug)]
pub struct ContractObject<'a> {
    kind: Cow<'a, str>,
    name: Cow<'a, str>,
    table: Cow<'a, str>,
    sql: Cow<'a, str>,
}
impl<'a> ContractObject<'a> {
    pub fn borrowed(o: SchemaObject<'a>) -> Self {
        Self {
            kind: Cow::Borrowed(o.kind),
            name: Cow::Borrowed(o.name),
            table: Cow::Borrowed(o.table),
            sql: Cow::Borrowed(o.sql),
        }
    }
    pub fn owned(kind: String, name: String, table: String, sql: String) -> Self {
        Self {
            kind: Cow::Owned(kind),
            name: Cow::Owned(name),
            table: Cow::Owned(table),
            sql: Cow::Owned(sql),
        }
    }
    pub fn as_object(&self) -> SchemaObject<'_> {
        SchemaObject {
            kind: &self.kind,
            name: &self.name,
            table: &self.table,
            sql: &self.sql,
        }
    }
}

/// Exactly SQLite's reserved ASCII prefix, not LIKE's underscore wildcard.
pub fn is_sqlite_internal_name(name: &str) -> bool {
    name.as_bytes()
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"sqlite_"))
}
fn reserved_dependency_name(name: &str) -> bool {
    name.as_bytes()
        .get(..14)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"metadata_read_"))
}
fn order(previous: SchemaObject<'_>, next: SchemaObject<'_>) -> Result<(), ContractError> {
    if (previous.kind, previous.name) == (next.kind, next.name) {
        return Err(ContractError::DuplicateObject);
    }
    if previous.key() >= next.key() {
        return Err(ContractError::ObjectOrder);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchPolicy {
    NativeRequired,
    ExactUnion,
}

#[derive(Debug)]
pub struct Contract<'a> {
    objects: Vec<ContractObject<'a>>,
    hash: [u8; 32],
}
impl<'a> Contract<'a> {
    pub fn new(objects: Vec<ContractObject<'a>>) -> Result<Self, ContractError> {
        let mut hash = Sha256::new();
        hash.update(b"foks-read-dependency-schema-v1\0");
        let mut previous = None;
        for stored in &objects {
            let object = stored.as_object();
            if is_sqlite_internal_name(object.name) {
                return Err(ContractError::InternalObject);
            }
            if let Some(prev) = previous {
                order(prev, object)?;
            }
            previous = Some(object);
            for text in [object.kind, object.name, object.table] {
                hash.update(
                    u64::try_from(text.len())
                        .map_err(|_| ContractError::LengthOverflow)?
                        .to_le_bytes(),
                );
                hash.update(text.as_bytes());
            }
            hash.update(lex::validate(object.sql)?.to_le_bytes());
            for token in lex::Tokens::new(object.sql) {
                token?.hash(&mut hash);
            }
        }
        Ok(Self {
            objects,
            hash: hash.finalize().into(),
        })
    }
    pub fn hash(&self) -> &[u8; 32] {
        &self.hash
    }
    pub fn verifier(&self, policy: MatchPolicy) -> SchemaVerifier<'_, 'a> {
        SchemaVerifier {
            contract: self,
            policy,
            position: 0,
            previous: None,
            failed: false,
        }
    }
}

#[derive(Default)]
struct PreviousKey {
    kind: String,
    name: String,
    table: String,
}
impl PreviousKey {
    fn object(&self) -> SchemaObject<'_> {
        SchemaObject {
            kind: &self.kind,
            name: &self.name,
            table: &self.table,
            sql: "",
        }
    }
    fn set(&mut self, object: SchemaObject<'_>) {
        for (target, source) in [
            (&mut self.kind, object.kind),
            (&mut self.name, object.name),
            (&mut self.table, object.table),
        ] {
            target.clear();
            target.push_str(source);
        }
    }
}

/// Streaming content equality only. Caller admission must bound observed fields
/// and charge this one retained key; no observed SQL survives observe().
pub struct SchemaVerifier<'contract, 'sql> {
    contract: &'contract Contract<'sql>,
    policy: MatchPolicy,
    position: usize,
    previous: Option<PreviousKey>,
    failed: bool,
}
impl SchemaVerifier<'_, '_> {
    pub fn observe(&mut self, object: SchemaObject<'_>) -> Result<(), ContractError> {
        if self.failed {
            return Err(ContractError::Poisoned);
        }
        let result = self.observe_inner(object);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn observe_inner(&mut self, object: SchemaObject<'_>) -> Result<(), ContractError> {
        if is_sqlite_internal_name(object.name) {
            return Err(ContractError::InternalObject);
        }
        if let Some(previous) = &self.previous {
            order(previous.object(), object)?;
        }
        self.previous
            .get_or_insert_with(PreviousKey::default)
            .set(object);
        if let Some(stored) = self.contract.objects.get(self.position) {
            let expected = stored.as_object();
            match expected.key().cmp(&object.key()) {
                Ordering::Less => return Err(ContractError::MissingObject),
                Ordering::Equal => {
                    if !sql_equal(expected.sql, object.sql)? {
                        return Err(ContractError::ObjectChanged);
                    }
                    self.position += 1;
                    return Ok(());
                }
                Ordering::Greater => {}
            }
        }
        if self.policy == MatchPolicy::ExactUnion
            || reserved_dependency_name(object.name)
            || object.kind == "trigger"
                && self.contract.objects.iter().any(|stored| {
                    let expected = stored.as_object();
                    expected.kind == "table" && expected.name.eq_ignore_ascii_case(object.table)
                })
        {
            return Err(ContractError::ExtraObject);
        }
        // Permitted extension SQL must still consume the entire lexer, including
        // malformed trailing comments/quotes. No ignored-extra early success.
        lex::validate(object.sql)?;
        Ok(())
    }
    pub fn finish(self) -> Result<(), ContractError> {
        if self.failed {
            Err(ContractError::Poisoned)
        } else if self.position != self.contract.objects.len() {
            Err(ContractError::MissingObject)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;

pub mod storage;
