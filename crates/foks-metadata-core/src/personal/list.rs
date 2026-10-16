//! Shared List domain logic. The caller owns one authenticated SQL transaction.
//! No fence, schema capability, output lifetime or serving route is created here.
use super::{
    current_key_generations, recheck_current_credential, require_key, require_personal_profile,
    CurrentCredential, KeyAccess, KeyRequirement, PersonalError, Result,
};
use crate::{
    directory::read_list_parent_key,
    list_encoder::ListEncoder,
    page::{charge, Cell},
    page_catalog::PageStatement as P,
    Sql, Value,
};
use foks_proto::{KvDirent, KvSmallFileBox, Role};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListCursor {
    Initial,
    Mac([u8; 32]),
    Time(u64),
}

/// Normalized public arguments, not an authenticated request or private token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRequest {
    parent: [u8; 16],
    cursor: ListCursor,
    count: usize,
    load_small: bool,
}
impl ListRequest {
    pub fn new(
        parent: [u8; 16],
        cursor: ListCursor,
        requested_count: u64,
        load_small: bool,
    ) -> Self {
        let count = if requested_count == 0 || requested_count > 4096 {
            4096
        } else {
            requested_count as usize
        };
        Self {
            parent,
            cursor,
            count,
            load_small,
        }
    }
}
#[derive(Clone, Copy)]
struct Position {
    id: [u8; 16],
    mac: [u8; 32],
    time: u64,
}
struct Decoded {
    entry: KvDirent,
    small: Option<KvSmallFileBox>,
}

fn decode_error(error: foks_proto::Error) -> PersonalError {
    match error {
        foks_proto::Error::Snowpack(error)
            if error.kind == foks_snowpack::ErrorKind::AllocationLimit =>
        {
            crate::Error::Bounds.into()
        }
        _ => PersonalError::Corrupt,
    }
}
fn exact<'a>(
    kind: Option<Cell<'a>>,
    length: Option<Cell<'a>>,
    payload: Option<Cell<'a>>,
    cap: usize,
) -> Result<&'a [u8]> {
    match (kind, length, payload) {
        (Some(Cell::Text("blob")), Some(Cell::Integer(length)), Some(Cell::Blob(bytes)))
            if length >= 0 && length as u64 == bytes.len() as u64 && bytes.len() <= cap =>
        {
            Ok(bytes)
        }
        _ => Err(PersonalError::Corrupt),
    }
}
fn decode(row: crate::page::Row<'_>, load: bool) -> Result<(Position, Decoded)> {
    let mut cells = row.cells();
    let (
        Some(Cell::Blob(id)),
        Some(Cell::Blob(mac)),
        Some(Cell::Integer(time)),
        Some(Cell::Blob(node)),
    ) = (cells.next(), cells.next(), cells.next(), cells.next())
    else {
        return Err(PersonalError::Corrupt);
    };
    let position = Position {
        id: id.try_into().map_err(|_| PersonalError::Corrupt)?,
        mac: mac.try_into().map_err(|_| PersonalError::Corrupt)?,
        time: u64::try_from(time).map_err(|_| PersonalError::Corrupt)?,
    };
    let node: &[u8; 17] = node.try_into().map_err(|_| PersonalError::Corrupt)?;
    let entry = exact(cells.next(), cells.next(), cells.next(), 4096)?;
    // Admit retained schema/exact copies before the native decoders allocate.
    charge(
        entry
            .len()
            .checked_mul(4)
            .and_then(|n| n.checked_add(4096))
            .ok_or(crate::Error::Bounds)?,
    )?;
    let entry = KvDirent::decode(entry).map_err(decode_error)?;
    let (present, kind, length, payload) = (cells.next(), cells.next(), cells.next(), cells.next());
    if cells.next().is_some() {
        return Err(PersonalError::Corrupt);
    }
    let small = match present {
        Some(Cell::Integer(0))
            if kind == Some(Cell::Text("null"))
                && length == Some(Cell::Integer(-1))
                && payload == Some(Cell::Null) =>
        {
            None
        }
        Some(Cell::Integer(1)) if load && node[0] == 3 => {
            let bytes = exact(kind, length, payload, 65536)?;
            charge(
                bytes
                    .len()
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(4096))
                    .ok_or(crate::Error::Bounds)?,
            )?;
            Some(KvSmallFileBox::decode(bytes).map_err(decode_error)?)
        }
        _ => return Err(PersonalError::Corrupt),
    };
    Ok((position, Decoded { entry, small }))
}
fn parameters(
    uid: &[u8; 33],
    request: &ListRequest,
    after: Option<Position>,
    limit: usize,
) -> Result<(P, Vec<Value>)> {
    charge(6 * std::mem::size_of::<Value>() + 33 + 16 + 32 + 16)?;
    let mut p = Vec::with_capacity(6);
    p.extend([
        Value::Blob(uid.to_vec()),
        Value::Blob(request.parent.to_vec()),
    ]);
    let statement = match (&request.cursor, after) {
        (ListCursor::Time(_), Some(pos)) => {
            p.extend([
                Value::Integer(pos.time as i64),
                Value::Blob(pos.id.to_vec()),
            ]);
            P::ListTimeContinue
        }
        (_, Some(pos)) => {
            p.extend([Value::Blob(pos.mac.to_vec()), Value::Blob(pos.id.to_vec())]);
            P::ListMacContinue
        }
        (ListCursor::Initial, None) => P::ListInitial,
        (ListCursor::Mac(mac), None) => {
            p.push(Value::Blob(mac.to_vec()));
            P::ListMacStart
        }
        (ListCursor::Time(time), None) => {
            p.push(Value::Integer(
                i64::try_from(*time).map_err(|_| PersonalError::Corrupt)?,
            ));
            P::ListTimeStart
        }
    };
    p.extend([
        Value::Integer(i64::from(request.load_small)),
        Value::Integer(limit as i64),
    ]);
    Ok((statement, p))
}
fn ordered(request: &ListRequest, previous: Option<Position>, next: Position) -> bool {
    match (&request.cursor, previous) {
        (ListCursor::Time(_), Some(p)) => (next.time, next.id) > (p.time, p.id),
        (_, Some(p)) => (next.mac, next.id) > (p.mac, p.id),
        (ListCursor::Mac(mac), None) => next.mac > *mac,
        (ListCursor::Time(time), None) => next.time >= *time,
        (ListCursor::Initial, None) => true,
    }
}
/// Rechecks the opaque SQL-created credential, then assembles exact public bytes.
/// Keep authentication, this call, and publication proof in the same transaction.
/// This helper neither starts/commits a transaction nor makes a production guard.
pub fn read_personal_list(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    request: &ListRequest,
    now_micros: u64,
    maximum_directory_bytes: usize,
) -> Result<Vec<u8>> {
    recheck_current_credential(sql, credential, now_micros)?;
    require_personal_profile(sql, credential)?;
    let key = read_list_parent_key(
        sql,
        credential.uid(),
        &request.parent,
        maximum_directory_bytes,
    )?;
    require_key(
        sql,
        credential,
        KeyRequirement {
            key,
            access: KeyAccess::Read,
        },
    )?;
    let mut encoder = ListEncoder::new(request.count)?;
    let mut count = 0;
    let mut after = None;
    loop {
        let limit = (request.count - count)
            .min(usize::from(P::ListInitial.spec().effective_rows()?).min(15));
        let (statement, p) = parameters(credential.uid(), request, after, limit)?;
        let page = sql.query_page(statement, &p)?;
        charge(limit * std::mem::size_of::<Decoded>())?;
        let mut decoded = Vec::with_capacity(limit);
        let mut failure = None;
        let mut rows = 0;
        for row in page.rows() {
            rows += 1;
            if rows > limit {
                return Err(PersonalError::Corrupt);
            }
            match decode(row, request.load_small) {
                Ok((position, value)) if ordered(request, after, position) => {
                    after = Some(position);
                    decoded.push(value);
                }
                Ok(_) => {
                    failure = Some(PersonalError::Corrupt);
                    break;
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        // Fixed page-local storage: no unbounded role cache and no key decisions
        // carried beyond the owner-held SQL transaction.
        let mut roles = [Role::NONE; 15];
        let mut role_count = 0;
        for row in &decoded {
            if let Some(small) = &row.small {
                if small.key.role != Role::NONE && small.key.role <= credential.role() {
                    roles[role_count] = small.key.role;
                    role_count += 1;
                }
            }
        }
        let mut generations = [None; 15];
        for (index, batch) in roles[..role_count].chunks(7).enumerate() {
            let values = current_key_generations(sql, credential.uid(), batch)?;
            generations[index * 7..index * 7 + values.len()].copy_from_slice(&values);
        }
        let mut role_index = 0;
        for row in &decoded {
            let small = row.small.as_ref().filter(|small| {
                if small.key.role == Role::NONE || small.key.role > credential.role() {
                    return false;
                }
                let current = generations[role_index];
                role_index += 1;
                small.key.generation > 0 && current.is_some_and(|g| small.key.generation <= g)
            });
            encoder.append(&row.entry, small)?;
            count += 1;
        }
        if let Some(error) = failure {
            return Err(error);
        }
        // Drop the single page and decoded schemas before the next SQL page or
        // final encoder allocation; output buffers retain their own admission.
        drop(decoded);
        drop(page);
        if count == request.count || rows < limit {
            return encoder.finish(count < request.count).map_err(Into::into);
        }
    }
}
