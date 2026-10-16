//! Transaction-local bounded generation lookup. This does not authorize a read.
use super::{PersonalError, Result};
use crate::{
    page::Cell,
    page_catalog::{role_generation_parameters, PageStatement},
    Sql,
};
use foks_proto::Role;

/// Preserve input order and duplicates; the caller must keep the same transaction
/// for generation lookup and use. No cache or proof escapes this function.
pub fn current_key_generations(
    sql: &mut impl Sql,
    uid: &[u8],
    roles: &[Role],
) -> Result<Vec<Option<u64>>> {
    let parameters = role_generation_parameters(uid, roles).map_err(|error| match error {
        crate::Error::Invalid(_) => PersonalError::BadArguments,
        error => PersonalError::Kernel(error),
    })?;
    crate::page::charge(roles.len() * std::mem::size_of::<Option<u64>>())?;
    let mut result = Vec::with_capacity(roles.len());
    let page = sql.query_page(PageStatement::RoleGenerations, &parameters)?;
    for (index, row) in page.rows().enumerate() {
        if index >= roles.len() {
            return Err(PersonalError::Corrupt);
        }
        let mut cells = row.cells();
        let ordinal = cells.next();
        let present = cells.next();
        let kind = cells.next();
        let generation = cells.next();
        if ordinal != Some(Cell::Integer(index as i64)) || cells.next().is_some() {
            return Err(PersonalError::Corrupt);
        }
        let value = match (present, kind, generation) {
            (Some(Cell::Integer(0)), Some(Cell::Text("null")), Some(Cell::Integer(0))) => None,
            (Some(Cell::Integer(1)), Some(Cell::Text("integer")), Some(Cell::Integer(value))) => {
                Some(u64::try_from(value).map_err(|_| PersonalError::Corrupt)?)
            }
            _ => return Err(PersonalError::Corrupt),
        };
        result.push(value);
    }
    if result.len() != roles.len() {
        return Err(PersonalError::Corrupt);
    }
    Ok(result)
}
