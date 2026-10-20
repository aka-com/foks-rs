//! One-record parent decoding qualification, not an authenticated List route.
//!
//! The caller owns one SQL transaction across the probe, slices and decode.
//! Authentication, revision/fencing and response publication remain its job.
use crate::{
    page::Cell,
    page_catalog::PageStatement as P,
    personal::{PersonalError, Result},
    Sql, Value,
};
use foks_proto::{KvDirectory, RoleAndGeneration};

pub const MAXIMUM_DIRECTORY_BYTES: usize = 16 * 1024 * 1024;
pub const DIRECTORY_SLICE_BYTES: usize = 512 * 1024;

fn dimensions<'a>(cells: &mut impl Iterator<Item = Cell<'a>>) -> Result<(i64, usize)> {
    let (Some(Cell::Integer(version)), Some(Cell::Text("blob")), Some(Cell::Integer(length))) =
        (cells.next(), cells.next(), cells.next())
    else {
        return Err(PersonalError::Corrupt);
    };
    if version <= 0 || length <= 0 {
        return Err(PersonalError::Corrupt);
    }
    Ok((
        version,
        usize::try_from(length).map_err(|_| PersonalError::Corrupt)?,
    ))
}

/// Decode the exact current parent as native List does, including legacy
/// records whose decoded id/version differ from their SQL selection keys.
/// The denormalized SQL key columns are deliberately never trusted here.
pub fn read_list_parent_key(
    sql: &mut impl Sql,
    uid: &[u8; 33],
    parent: &[u8; 16],
    maximum_bytes: usize,
) -> Result<RoleAndGeneration> {
    if maximum_bytes == 0 || maximum_bytes > MAXIMUM_DIRECTORY_BYTES {
        return Err(PersonalError::UnsupportedProfile);
    }
    crate::page::charge(49 + 3 * std::mem::size_of::<Value>())?;
    let parameters = [
        Value::Blob(uid.to_vec()),
        Value::Blob(parent.to_vec()),
        Value::Integer(maximum_bytes as i64),
    ];
    let (version, length) = {
        let page = sql.query_page(P::DirectoryProbe, &parameters)?;
        let mut rows = page.rows();
        let row = rows.next().ok_or(PersonalError::Missing)?;
        if rows.next().is_some() {
            return Err(PersonalError::Corrupt);
        }
        let mut cells = row.cells();
        let dimensions = dimensions(&mut cells)?;
        if cells.next().is_some() || dimensions.1 > maximum_bytes {
            return Err(PersonalError::Corrupt);
        }
        dimensions
    };
    // Admit once, reserve exactly one complete input, then copy each bounded
    // slice directly. No collection of pages or growing input-buffer copies.
    crate::page::charge(length)?;
    let mut exact = Vec::new();
    exact
        .try_reserve_exact(length)
        .map_err(|_| crate::Error::Bounds)?;
    while exact.len() < length {
        let offset = exact.len();
        let requested = DIRECTORY_SLICE_BYTES.min(length - offset);
        crate::page::charge(49 + 6 * std::mem::size_of::<Value>())?;
        let parameters = [
            Value::Blob(uid.to_vec()),
            Value::Blob(parent.to_vec()),
            Value::Integer(version),
            Value::Integer(offset as i64),
            Value::Integer(requested as i64),
            Value::Integer(maximum_bytes as i64),
        ];
        {
            let page = sql.query_page(P::DirectorySlice, &parameters)?;
            let mut rows = page.rows();
            let row = rows.next().ok_or(PersonalError::Corrupt)?;
            if rows.next().is_some() {
                return Err(PersonalError::Corrupt);
            }
            let mut cells = row.cells();
            if dimensions(&mut cells)? != (version, length) {
                return Err(PersonalError::Corrupt);
            }
            let Some(Cell::Blob(bytes)) = cells.next() else {
                return Err(PersonalError::Corrupt);
            };
            if bytes.len() != requested || cells.next().is_some() {
                return Err(PersonalError::Corrupt);
            }
            exact.extend_from_slice(bytes);
        }
    }
    // The final page is dropped before the existing decoder's AST/schema copies.
    // No key-only shortcut, new decoder, or decoded id/version consistency rule.
    let directory = KvDirectory::decode(&exact).map_err(|error| match error {
        foks_proto::Error::Snowpack(error)
            if error.kind == foks_snowpack::ErrorKind::AllocationLimit =>
        {
            PersonalError::Kernel(crate::Error::Bounds)
        }
        _ => PersonalError::Corrupt,
    })?;
    Ok(directory.key)
}

/// Bounded private qualification request, deliberately not a service entry.
/// It accepts only the pinned probe statement's exact uid/parent/limit shape.
pub fn qualify(sql: &mut impl Sql, input: Vec<u8>) -> Result<Vec<u8>> {
    let (statement, parameters) = crate::page::decode_parameters(input)?;
    let [Value::Blob(uid), Value::Blob(parent), Value::Integer(maximum)] = parameters.as_slice()
    else {
        return Err(PersonalError::BadArguments);
    };
    if statement != P::DirectoryProbe {
        return Err(PersonalError::BadArguments);
    }
    let uid = uid
        .as_slice()
        .try_into()
        .map_err(|_| PersonalError::BadArguments)?;
    let parent = parent
        .as_slice()
        .try_into()
        .map_err(|_| PersonalError::BadArguments)?;
    let maximum = usize::try_from(*maximum).map_err(|_| PersonalError::BadArguments)?;
    let key = read_list_parent_key(sql, uid, parent, maximum)?;
    foks_snowpack::encode(&key.to_value()).map_err(|error| {
        if error.kind == foks_snowpack::ErrorKind::AllocationLimit {
            PersonalError::Kernel(crate::Error::Bounds)
        } else {
            PersonalError::Corrupt
        }
    })
}

pub fn qualification_error(error: &PersonalError) -> &'static str {
    match error {
        PersonalError::Missing => "missing",
        PersonalError::Corrupt => "corrupt",
        PersonalError::UnsupportedProfile => "unsupported_profile",
        PersonalError::BadArguments => "bad_arguments",
        PersonalError::Kernel(error) => error.code(),
        _ => "invalid",
    }
}
