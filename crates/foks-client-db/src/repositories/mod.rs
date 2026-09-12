//! Domain repositories sharing the store's single SQLite transaction layer.

pub(crate) mod chat;
mod federation;
mod host;
mod jobs;
mod journals;
mod metadata;
pub(crate) mod sso;
mod team;
mod user;

use rusqlite::{Transaction, TransactionBehavior};

use super::{HardStateStore, Result};

impl HardStateStore {
    /// Starts the sole write-transaction mode used by hard-state repositories.
    pub(super) fn write_transaction(&mut self) -> Result<Transaction<'_>> {
        Ok(self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn domain_repositories_use_the_shared_write_transaction_layer() {
        let repositories = [
            include_str!("host.rs"),
            include_str!("chat.rs"),
            include_str!("federation.rs"),
            include_str!("jobs.rs"),
            include_str!("journals.rs"),
            include_str!("metadata.rs"),
            include_str!("team.rs"),
            include_str!("user.rs"),
            include_str!("sso.rs"),
        ];
        for repository in repositories {
            assert!(!repository.contains("transaction_with_behavior"));
            assert!(!repository.contains(".transaction()"));
        }
    }
}
