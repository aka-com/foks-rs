//! Feature-only cumulative charge qualification. Not a reusable budget or guard.
pub mod formula;
mod input;
#[cfg(test)]
mod tests;
use crate::{personal, Error, Sql};
use foks_snowpack::AllocationBudget;
pub use input::Invocation;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, Mutex},
};

pub const MAX_INPUT: usize = 20_480;
pub const MAX_OUTPUT: usize = 8192;
const PHASES: usize = 12;
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum Phase {
    Authentication,
    Entry,
    Parent,
    ParentAuthorization,
    Page,
    Decode,
    Roles,
    Append,
    Growth,
    Finish,
    Frame,
    Other,
}
const NAMES: [&str; PHASES] = [
    "authentication",
    "entry",
    "parent",
    "parent_authorization",
    "page",
    "decode",
    "roles",
    "append",
    "growth",
    "finish",
    "frame",
    "other",
];
thread_local! {
 static PHASE: Cell<Phase> = const { Cell::new(Phase::Other) };
 static ACTIVE: RefCell<Option<Arc<Recorder>>> = const { RefCell::new(None) };
}
pub(crate) struct PhaseGuard {
    previous: Phase,
    current: Phase,
    start: u64,
    growth_start: u64,
}
impl Drop for PhaseGuard {
    fn drop(&mut self) {
        if matches!(self.current, Phase::Page) {
            ACTIVE.with(|a| {
                if let Some(r) = a.borrow().as_ref() {
                    let mut s = r.state.lock().unwrap();
                    s.page_scopes += 1;
                    let whole = s.total.admitted_bytes - self.start;
                    let growth =
                        s.phases[Phase::Growth as usize].admitted_bytes - self.growth_start;
                    s.maximum_page_charge = s.maximum_page_charge.max(whole);
                    s.maximum_page_scratch = s.maximum_page_scratch.max(whole - growth);
                }
            });
        }
        PHASE.with(|p| p.set(self.previous));
    }
}
pub(crate) fn phase(p: Phase) -> PhaseGuard {
    PhaseGuard {
        previous: PHASE.with(|old| old.replace(p)),
        current: p,
        start: ACTIVE.with(|a| {
            a.borrow()
                .as_ref()
                .map_or(0, |r| r.state.lock().unwrap().total.admitted_bytes)
        }),
        growth_start: ACTIVE.with(|a| {
            a.borrow().as_ref().map_or(0, |r| {
                r.state.lock().unwrap().phases[Phase::Growth as usize].admitted_bytes
            })
        }),
    }
}
pub(crate) fn allocator_failed() {
    ACTIVE.with(|a| {
        if let Some(r) = a.borrow().as_ref() {
            r.state.lock().unwrap().allocator_failed = true;
        }
    });
}

fn decimal<S: serde::Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(v)
}
fn optional_decimal<S: serde::Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => s.serialize_some(&Decimal(*v)),
        None => s.serialize_none(),
    }
}
struct Decimal(u64);
impl Serialize for Decimal {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}
#[derive(Clone, Copy, Default, Serialize)]
pub struct Totals {
    #[serde(serialize_with = "decimal")]
    pub attempts: u64,
    #[serde(serialize_with = "decimal")]
    pub admitted_bytes: u64,
    #[serde(serialize_with = "decimal")]
    pub refused_calls: u64,
    #[serde(serialize_with = "decimal")]
    pub first_refusal: u64,
}
const PREFIX: usize = 16;
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub enum Origin {
    Snowpack,
    RpcResponse,
}
#[derive(Clone, Copy, Serialize)]
pub struct Event {
    #[serde(serialize_with = "decimal")]
    pub ordinal: u64,
    pub phase: &'static str,
    pub origin: Origin,
    pub control: bool,
    #[serde(serialize_with = "decimal")]
    pub bytes: u64,
    pub accepted: bool,
}
#[derive(Clone, Copy)]
struct State {
    phases: [Totals; PHASES],
    total: Totals,
    origins: [Totals; 2],
    prefix: [Option<Event>; PREFIX],
    page_scopes: u64,
    maximum_page_charge: u64,
    maximum_page_scratch: u64,
    first_refusal: u64,
    allocator_failed: bool,
    overflow: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            phases: [Totals::default(); PHASES],
            total: Totals::default(),
            origins: [Totals::default(); 2],
            prefix: [None; PREFIX],
            page_scopes: 0,
            maximum_page_charge: 0,
            maximum_page_scratch: 0,
            first_refusal: 0,
            allocator_failed: false,
            overflow: false,
        }
    }
}
struct Recorder {
    state: Mutex<State>,
    fail_at: u64,
}
impl Recorder {
    fn new(fail_at: u64) -> Self {
        Self {
            state: Mutex::new(State::default()),
            fail_at,
        }
    }
    fn record(&self, bytes: usize, origin: Origin, control: bool) -> bool {
        let index = PHASE.with(|p| p.get() as usize);
        let mut s = self.state.lock().unwrap();
        let Some(next) = s.total.attempts.checked_add(1) else {
            s.overflow = true;
            return false;
        };
        let bytes = bytes as u64;
        let accepted = !s.overflow
            && next != self.fail_at
            && s.total.admitted_bytes.checked_add(bytes).is_some();
        s.total.attempts = next;
        s.phases[index].attempts += 1;
        s.origins[origin as usize].attempts += 1;
        if next <= PREFIX as u64 {
            s.prefix[next as usize - 1] = Some(Event {
                ordinal: next,
                phase: NAMES[index],
                origin,
                control,
                bytes,
                accepted,
            });
        }
        if accepted {
            s.total.admitted_bytes += bytes;
            s.phases[index].admitted_bytes += bytes;
            s.origins[origin as usize].admitted_bytes += bytes;
        } else {
            s.total.refused_calls += 1;
            s.phases[index].refused_calls += 1;
            s.origins[origin as usize].refused_calls += 1;
            if s.first_refusal == 0 {
                s.first_refusal = next;
                s.total.first_refusal = next;
            }
            if s.phases[index].first_refusal == 0 {
                s.phases[index].first_refusal = next;
            }
            if s.origins[origin as usize].first_refusal == 0 {
                s.origins[origin as usize].first_refusal = next;
            }
        }
        accepted
    }
}
impl AllocationBudget for Recorder {
    fn reserve(&self, bytes: usize) -> bool {
        self.record(bytes, Origin::Snowpack, false)
    }
}
impl foks_rpc::ResponseAllocationBudget for Recorder {
    fn reserve(&self, bytes: usize, control: bool) -> bool {
        self.record(bytes, Origin::RpcResponse, control)
    }
}
struct ActiveGuard(Option<Arc<Recorder>>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.with(|a| *a.borrow_mut() = self.0.take());
    }
}

#[derive(Serialize)]
pub struct Layout {
    pub pointer_bits: u32,
    pub value_bytes: usize,
    pub sql_value_bytes: Option<usize>,
    pub cell_bytes: usize,
    pub decoded_bytes: usize,
    pub snowpack_value_bytes: usize,
}
pub fn layout() -> Layout {
    Layout {
        pointer_bits: usize::BITS,
        value_bytes: std::mem::size_of::<crate::Value>(),
        sql_value_bytes: {
            #[cfg(feature = "native")]
            {
                Some(std::mem::size_of::<rusqlite::types::Value>())
            }
            #[cfg(not(feature = "native"))]
            {
                None
            }
        },
        cell_bytes: std::mem::size_of::<crate::page::Cell<'static>>(),
        decoded_bytes: personal::list_decoded_size(),
        snowpack_value_bytes: std::mem::size_of::<foks_snowpack::Value>(),
    }
}
#[derive(Serialize)]
pub struct LayoutReport {
    pub version: u8,
    pub target: Layout,
    pub bounds: formula::Bounds,
}
pub fn layout_report() -> crate::Result<LayoutReport> {
    Ok(LayoutReport {
        version: 1,
        target: layout(),
        bounds: formula::bounds()?,
    })
}
#[derive(Serialize)]
pub struct PhaseTotal {
    pub name: &'static str,
    #[serde(flatten)]
    pub totals: Totals,
}
#[derive(Serialize)]
pub struct OriginTotal {
    pub origin: Origin,
    #[serde(flatten)]
    pub totals: Totals,
}
#[derive(Serialize)]
pub struct Report {
    pub version: u8,
    pub outcome: &'static str,
    pub target: Layout,
    pub phases: [PhaseTotal; PHASES],
    pub total: Totals,
    pub origins: [OriginTotal; 2],
    pub prefix: [Option<Event>; PREFIX],
    #[serde(serialize_with = "decimal")]
    pub page_scopes: u64,
    #[serde(serialize_with = "decimal")]
    pub maximum_page_charge: u64,
    #[serde(serialize_with = "decimal")]
    pub maximum_page_scratch: u64,
    #[serde(serialize_with = "decimal")]
    pub response_bytes: u64,
    pub list_sha256: Option<String>,
    #[serde(serialize_with = "decimal")]
    pub frame_bytes: u64,
    pub frame_sha256: Option<String>,
    #[serde(serialize_with = "decimal")]
    pub sql_calls: u64,
    #[serde(serialize_with = "decimal")]
    pub first_refusal: u64,
    pub allocator_failed: bool,
    #[serde(serialize_with = "optional_decimal")]
    pub memory_pages: Option<u64>,
}
fn hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    use std::fmt::Write;
    for b in digest {
        write!(out, "{b:02x}").unwrap();
    }
    out
}
struct Counted<'a, S> {
    sql: &'a mut S,
    calls: u64,
}
impl<S: Sql> Sql for Counted<'_, S> {
    fn query(
        &mut self,
        s: crate::Statement,
        p: &[crate::Value],
    ) -> crate::Result<Option<Vec<crate::Value>>> {
        self.calls += 1;
        self.sql.query(s, p)
    }
    fn execute(&mut self, s: crate::Statement, p: &[crate::Value]) -> crate::Result<u64> {
        self.calls += 1;
        self.sql.execute(s, p)
    }
    fn query_page(
        &mut self,
        s: crate::page_catalog::PageStatement,
        p: &[crate::Value],
    ) -> crate::Result<crate::page::PageBuffer> {
        self.calls += 1;
        self.sql.query_page(s, p)
    }
}
fn outcome(e: &personal::PersonalError) -> &'static str {
    use personal::PersonalError as P;
    match e {
        P::Unauthenticated => "unauthenticated",
        P::Permission => "permission",
        P::Missing => "missing",
        P::Corrupt => "corrupt",
        P::BadArguments => "invalid",
        P::UnsupportedProfile => "unsupported",
        P::Kernel(e) => e.code(),
        _ => "unexpected_personal_error",
    }
}
/// Parsing is bounded fixture overhead, deliberately outside charge observation.
pub fn run(sql: &mut impl Sql, input: &[u8]) -> crate::Result<Report> {
    let invocation = Invocation::decode(input)?;
    run_parsed(sql, invocation)
}
pub fn run_parsed(sql: &mut impl Sql, i: Invocation) -> crate::Result<Report> {
    if ACTIVE.with(|a| a.borrow().is_some()) {
        return Err(Error::Invalid("trace reentry"));
    }
    let recorder = Arc::new(Recorder::new(i.fail_at));
    let _active = ActiveGuard(ACTIVE.with(|a| a.replace(Some(recorder.clone()))));
    let mut sql = Counted { sql, calls: 0 };
    let result = foks_snowpack::with_allocation_budget(recorder.clone(), || {
        foks_rpc::with_response_budget(recorder.clone(), || {
            let credential = {
                let _phase = phase(Phase::Authentication);
                personal::authenticate_peer(&mut sql, &i.peer, i.now_micros)
                    .map_err(|e| outcome(&e))?
            };
            let response = personal::read_personal_list(
                &mut sql,
                &credential,
                &i.request,
                i.now_micros,
                crate::directory::MAXIMUM_DIRECTORY_BYTES,
            )
            .map_err(|e| outcome(&e))?;
            let frame = if i.frame {
                let _phase = phase(Phase::Frame);
                Some(
                    foks_rpc::encode_success_response_at(&response, i.rpc_sequence).map_err(
                        |e| match e {
                            foks_rpc::Error::FrameTooLarge { .. } => "frame_too_large",
                            _ => "frame_error",
                        },
                    )?,
                )
            } else {
                None
            };
            Ok::<_, &'static str>((response, frame))
        })
    });
    let s = *recorder.state.lock().unwrap();
    let mut report = Report {
        version: 1,
        outcome: match &result {
            Ok(_) => "ok",
            Err(e) => e,
        },
        target: layout(),
        phases: std::array::from_fn(|n| PhaseTotal {
            name: NAMES[n],
            totals: s.phases[n],
        }),
        total: s.total,
        origins: [
            OriginTotal {
                origin: Origin::Snowpack,
                totals: s.origins[0],
            },
            OriginTotal {
                origin: Origin::RpcResponse,
                totals: s.origins[1],
            },
        ],
        prefix: s.prefix,
        page_scopes: s.page_scopes,
        maximum_page_charge: s.maximum_page_charge,
        maximum_page_scratch: s.maximum_page_scratch,
        response_bytes: 0,
        list_sha256: None,
        frame_bytes: 0,
        frame_sha256: None,
        sql_calls: sql.calls,
        first_refusal: s.first_refusal,
        allocator_failed: s.allocator_failed,
        memory_pages: None,
    };
    if s.first_refusal != 0 || s.overflow {
        report.outcome = "trace_refused";
    } else if s.allocator_failed {
        report.outcome = "allocator_failed";
    }
    if report.outcome == "ok" {
        if let Ok((response, frame)) = result {
            report.response_bytes = response.len() as u64;
            report.list_sha256 = Some(hash(&response));
            if let Some(frame) = frame {
                report.frame_bytes = frame.len() as u64;
                report.frame_sha256 = Some(hash(&frame));
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        report.memory_pages = Some(core::arch::wasm32::memory_size(0) as u64);
    }
    Ok(report)
}
/// Fixed-capacity writer bounds diagnostics before any output Vec allocation.
pub fn encode_report(value: &impl Serialize) -> crate::Result<Vec<u8>> {
    struct Buffer {
        bytes: [u8; MAX_OUTPUT],
        len: usize,
    }
    impl std::io::Write for Buffer {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            let end = self
                .len
                .checked_add(b.len())
                .filter(|e| *e <= MAX_OUTPUT)
                .ok_or_else(|| std::io::Error::other("trace output bound"))?;
            self.bytes[self.len..end].copy_from_slice(b);
            self.len = end;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut b = Buffer {
        bytes: [0; MAX_OUTPUT],
        len: 0,
    };
    serde_json::to_writer(&mut b, value).map_err(|_| Error::Bounds)?;
    let mut out = Vec::new();
    out.try_reserve_exact(b.len).map_err(|_| Error::Bounds)?;
    out.extend_from_slice(&b.bytes[..b.len]);
    Ok(out)
}
