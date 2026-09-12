//! Exhaustive, keyset-paged durable owners for the shared protected directory.
use crate::*;

#[derive(Clone, Debug)]
pub enum ProtectedRecordOwner {
    Mutation(MutationOperation),
    Chat(ChatOperation),
    Sso(SsoFlow),
    Team(TeamMutationOperation),
}

/// Private cursor; a caller must discard it when hard-state metadata changes.
#[derive(Default)]
pub struct ProtectedOwnerCursor {
    table: usize,
    after: Vec<u8>,
}

impl HardStateStore {
    /// One indexed owner lookup. None means all owner tables were exhausted.
    /// This deliberately includes terminal owners whose cleanup may have failed.
    pub fn next_protected_owner(
        &self,
        cursor: &mut ProtectedOwnerCursor,
    ) -> Result<Option<ProtectedRecordOwner>> {
        const TABLES: [&str; 4] = [
            "mutation_operations",
            "chat_operations",
            "sso_flows",
            "team_mutation_operations",
        ];
        while let Some(table) = TABLES.get(cursor.table) {
            let id: Option<[u8;16]> = self.connection.query_row(
                &format!("SELECT operation_id FROM {table} WHERE operation_id>?1 ORDER BY operation_id LIMIT 1"),
                [&cursor.after], |row| row.get(0),
            ).optional()?;
            let Some(id) = id else {
                cursor.table += 1;
                cursor.after.clear();
                continue;
            };
            let owner = match cursor.table {
                0 => self.mutation(&id)?.map(ProtectedRecordOwner::Mutation),
                1 => self.chat_operation(&id)?.map(ProtectedRecordOwner::Chat),
                2 => self.sso_flow(&id)?.map(ProtectedRecordOwner::Sso),
                3 => self.team_mutation(&id)?.map(ProtectedRecordOwner::Team),
                _ => unreachable!("table selected above"),
            }
            .ok_or(Error::InvalidMutationOperation(
                "protected owner disappeared",
            ))?;
            cursor.after = id.to_vec();
            return Ok(Some(owner));
        }
        Ok(None)
    }
}
