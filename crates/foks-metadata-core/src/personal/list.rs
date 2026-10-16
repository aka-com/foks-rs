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
pub(super) struct Decoded {
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
trait Encoder {
    type Output;
    fn append(&mut self, entry: &KvDirent, small: Option<&KvSmallFileBox>) -> crate::Result<()>;
    fn finish(self, final_page: bool) -> crate::Result<Self::Output>;
}
impl Encoder for ListEncoder {
    type Output = Vec<u8>;
    fn append(&mut self, entry: &KvDirent, small: Option<&KvSmallFileBox>) -> crate::Result<()> {
        ListEncoder::append(self, entry, small)
    }
    fn finish(self, final_page: bool) -> crate::Result<Self::Output> {
        ListEncoder::finish(self, final_page)
    }
}
#[cfg(any(test, feature = "list-allocation-qualification"))]
impl Encoder for crate::list_encoder::owned::OwnedListEncoder {
    type Output = crate::list_encoder::owned::OwnedListBytes;
    fn append(&mut self, entry: &KvDirent, small: Option<&KvSmallFileBox>) -> crate::Result<()> {
        Self::append(self, entry, small)
    }
    fn finish(self, final_page: bool) -> crate::Result<Self::Output> {
        Self::finish(self, final_page)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageFault {
    Unauthenticated,
    Permission,
    Missing,
    UnsupportedProfile,
    Corrupt,
    BadArguments,
    StalePreparation,
    CachedObjectMissing,
    Invalid(&'static str),
    IntegerRange,
    Conflict,
    KernelPermission,
    Quota,
    Bounds,
    Storage,
    Expired,
}
impl From<PersonalError> for PageFault {
    fn from(error: PersonalError) -> Self {
        use crate::Error as K;
        match error {
            PersonalError::Unauthenticated => Self::Unauthenticated,
            PersonalError::Permission => Self::Permission,
            PersonalError::Missing => Self::Missing,
            PersonalError::UnsupportedProfile => Self::UnsupportedProfile,
            PersonalError::Corrupt => Self::Corrupt,
            PersonalError::BadArguments => Self::BadArguments,
            PersonalError::StalePreparation => Self::StalePreparation,
            PersonalError::CachedObjectMissing => Self::CachedObjectMissing,
            // No current page callee can produce a stale vector. Consume any
            // future heap-bearing error while its page allowance is still held.
            PersonalError::StaleCache(vector) => {
                drop(vector);
                Self::Storage
            }
            PersonalError::Kernel(error) => match error {
                K::Invalid(message) => Self::Invalid(message),
                K::IntegerRange => Self::IntegerRange,
                K::Conflict => Self::Conflict,
                K::Permission => Self::KernelPermission,
                K::Quota => Self::Quota,
                K::Bounds => Self::Bounds,
                K::Storage => Self::Storage,
                K::Expired => Self::Expired,
            },
        }
    }
}
impl From<PageFault> for PersonalError {
    fn from(error: PageFault) -> Self {
        use crate::Error as K;
        match error {
            PageFault::Unauthenticated => Self::Unauthenticated,
            PageFault::Permission => Self::Permission,
            PageFault::Missing => Self::Missing,
            PageFault::UnsupportedProfile => Self::UnsupportedProfile,
            PageFault::Corrupt => Self::Corrupt,
            PageFault::BadArguments => Self::BadArguments,
            PageFault::StalePreparation => Self::StalePreparation,
            PageFault::CachedObjectMissing => Self::CachedObjectMissing,
            PageFault::Invalid(message) => K::Invalid(message).into(),
            PageFault::IntegerRange => K::IntegerRange.into(),
            PageFault::Conflict => K::Conflict.into(),
            PageFault::KernelPermission => K::Permission.into(),
            PageFault::Quota => K::Quota.into(),
            PageFault::Bounds => K::Bounds.into(),
            PageFault::Storage => K::Storage.into(),
            PageFault::Expired => K::Expired.into(),
        }
    }
}
#[derive(Default)]
struct PageWork {
    active: std::cell::Cell<bool>,
}
struct PageScope<'a>(&'a std::cell::Cell<bool>);
impl PageWork {
    fn enter(&self) -> std::result::Result<PageScope<'_>, PageFault> {
        if self.active.replace(true) {
            return Err(PageFault::Storage);
        }
        Ok(PageScope(&self.active))
    }
}
impl Drop for PageScope<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
#[derive(Clone, Copy)]
struct PageProgress {
    added: usize,
    last: Option<Position>,
    exhausted: bool,
}

fn process_page(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    request: &ListRequest,
    mut after: Option<Position>,
    remaining: usize,
    encoder: &mut impl Encoder,
    work: &mut PageWork,
) -> std::result::Result<PageProgress, PageFault> {
    // W1 only proves the lifetime boundary. No allowance or SQL/deadline counter
    // is reset. All scratch still uses the original cumulative ambient hook.
    let _scope = work.enter()?;
    let result = (|| -> Result<PageProgress> {
        let mut added = 0;
        let limit = remaining.min(usize::from(P::ListInitial.spec().effective_rows()?).min(15));
        #[cfg(feature = "list-allocation-qualification")]
        let _page_phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Page);
        let (statement, p) = parameters(credential.uid(), request, after, limit)?;
        let page = sql.query_page(statement, &p)?;
        charge(limit * std::mem::size_of::<Decoded>())?;
        let mut decoded = Vec::with_capacity(limit);
        let mut failure = None;
        let mut rows = 0;
        for row in page.rows() {
            #[cfg(feature = "list-allocation-qualification")]
            let _phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Decode);
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
            #[cfg(feature = "list-allocation-qualification")]
            let _phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Roles);
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
            #[cfg(feature = "list-allocation-qualification")]
            let _phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Append);
            encoder.append(&row.entry, small)?;
            added += 1;
        }
        if let Some(error) = failure {
            return Err(error);
        }
        // Drop the single page and decoded schemas before the next SQL page or
        // final encoder allocation; output buffers retain their own admission.
        drop(decoded);
        drop(page);
        #[cfg(feature = "list-allocation-qualification")]
        drop(_page_phase);
        Ok(PageProgress {
            added,
            last: after,
            exhausted: rows < limit,
        })
    })();
    // Conversion consumes/drops any error payload before _scope can end.
    result.map_err(PageFault::from)
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
    read_with_encoder(
        sql,
        credential,
        request,
        now_micros,
        maximum_directory_bytes,
        ListEncoder::new,
    )
}

#[cfg(any(test, feature = "list-allocation-qualification"))]
pub(crate) fn read_personal_list_owned(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    request: &ListRequest,
    now_micros: u64,
    maximum_directory_bytes: usize,
    owner: std::sync::Arc<dyn crate::list_encoder::owned::ListRetainedOwner>,
) -> Result<crate::list_encoder::owned::OwnedListBytes> {
    if owner.failed() {
        return Err(crate::Error::Bounds.into());
    }
    let output = read_with_encoder(
        sql,
        credential,
        request,
        now_micros,
        maximum_directory_bytes,
        |count| crate::list_encoder::owned::OwnedListEncoder::new(count, owner.clone()),
    )?;
    if owner.failed() {
        drop(output);
        return Err(crate::Error::Bounds.into());
    }
    Ok(output)
}

fn read_with_encoder<E: Encoder>(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    request: &ListRequest,
    now_micros: u64,
    maximum_directory_bytes: usize,
    make_encoder: impl FnOnce(usize) -> crate::Result<E>,
) -> Result<E::Output> {
    #[cfg(feature = "list-allocation-qualification")]
    let _entry_phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Entry);
    recheck_current_credential(sql, credential, now_micros)?;
    require_personal_profile(sql, credential)?;
    let key = {
        #[cfg(feature = "list-allocation-qualification")]
        let _phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Parent);
        read_list_parent_key(
            sql,
            credential.uid(),
            &request.parent,
            maximum_directory_bytes,
        )?
    };
    {
        #[cfg(feature = "list-allocation-qualification")]
        let _phase =
            crate::allocation_trace::phase(crate::allocation_trace::Phase::ParentAuthorization);
        require_key(
            sql,
            credential,
            KeyRequirement {
                key,
                access: KeyAccess::Read,
            },
        )?;
    }
    let mut encoder = make_encoder(request.count)?;
    let mut count = 0;
    let mut after = None;
    let mut work = PageWork::default();
    loop {
        let progress = process_page(
            sql,
            credential,
            request,
            after,
            request.count - count,
            &mut encoder,
            &mut work,
        )?;
        count += progress.added;
        after = progress.last;
        if count == request.count || progress.exhausted {
            #[cfg(feature = "list-allocation-qualification")]
            let _phase = crate::allocation_trace::phase(crate::allocation_trace::Phase::Finish);
            return encoder.finish(count < request.count).map_err(Into::into);
        }
    }
}

#[cfg(test)]
mod workspace_tests;
