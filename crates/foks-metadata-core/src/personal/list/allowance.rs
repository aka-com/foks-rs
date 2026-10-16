//! Page-only requested-allocation ownership; not a transaction or serving guard.
// Valid-schema cumulative hook credits + non-spendable requested-live auxiliaries.
use crate::{page_catalog::PageStatement as P, Error, Result};

#[derive(Clone, Copy, Debug)]
struct Layout {
    value: usize,
    sql_value: Option<usize>,
    cell: usize,
    decoded: usize,
    snowpack: usize,
    path: usize,
    option_generation: usize,
}
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct PageAllowanceTerms {
    parameter_copies: usize,
    list_transport: usize,
    decoded_container: usize,
    three_role_batches: usize,
    fifteen_row_decode: usize,
    fifteen_row_append: usize,
    // Never available to reserve_allocation: path/error cleanup can need this
    // after all dynamic credits have been consumed or refused.
    auxiliary_live: usize,
    dynamic_credits: usize,
    workspace_bytes: usize,
    // Separate invocation control allocation, reserved before constructing Arc.
    control_bytes: usize,
}
fn add(xs: &[usize]) -> Result<usize> {
    xs.iter()
        .try_fold(0usize, |a, b| a.checked_add(*b).ok_or(Error::Bounds))
}
fn mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or(Error::Bounds)
}
#[derive(Clone, Copy)]
struct Shape {
    decode: usize,
    encode: usize,
}
fn scalar() -> Shape {
    Shape {
        decode: 0,
        encode: 9,
    }
}
fn bytes(n: usize) -> Result<Shape> {
    Ok(Shape {
        decode: add(&[mul(4, n)?, 64])?,
        encode: add(&[9, n])?,
    })
}
fn array(v: usize, children: &[Shape]) -> Result<Shape> {
    let mut result = Shape {
        decode: add(&[mul(mul(4, children.len())?, v)?, 4096])?,
        encode: 9,
    };
    for child in children {
        result.decode = add(&[result.decode, child.decode])?;
        result.encode = add(&[result.encode, child.encode])?;
    }
    Ok(result)
}
fn variant(v: usize, child: Shape) -> Result<Shape> {
    // The longest valid selected schemas use tag b"0"; malformed path tags
    // are handled separately by auxiliary_live, not this successful-shape term.
    Ok(Shape {
        decode: add(&[mul(4, v)?, 128, 68, child.decode])?,
        encode: add(&[10, child.encode])?,
    })
}
fn schemas(v: usize) -> Result<(Shape, Shape)> {
    let role = array(v, &[scalar(), variant(v, scalar())?])?;
    let secret = array(
        v,
        &[
            scalar(),
            variant(v, array(v, &[bytes(16)?, bytes(4096)?])?)?,
        ],
    )?;
    let entry = array(
        v,
        &[
            bytes(16)?,
            bytes(16)?,
            bytes(17)?,
            scalar(),
            scalar(),
            role,
            bytes(32)?,
            secret,
            scalar(),
            bytes(32)?,
            scalar(),
        ],
    )?;
    let body = array(v, &[array(v, &[role, scalar()])?, bytes(65536)?])?;
    Ok((entry, body))
}
fn transport(statement: P, rows: usize, count: usize, payload: usize, l: Layout) -> Result<usize> {
    let spec = statement.spec();
    let row = spec.maximum_row_bytes()?;
    let reply = add(&[24, mul(rows, row)?])?;
    if let Some(q) = l.sql_value {
        // Exact caller copies and all Cell Vec requests are covered by the
        // existing unchanged admission. SQLite/rusqlite engine memory excluded.
        add(&[
            payload,
            mul(count, q)?,
            mul(
                spec.columns.len(),
                add(&[512, mul(add(&[rows, 1])?, l.cell)?])?,
            )?,
            row,
            reply,
        ])
    } else {
        // Parameters: old 24-byte header + full replacement + Cell vector.
        // P1 reply is moved into PageBuffer without a second payload copy.
        add(&[
            24,
            add(&[24, payload, mul(count, 5)?])?,
            mul(count, l.cell)?,
            reply,
        ])
    }
}

use super::{PageFault, PageProgress};
use crate::list_encoder::owned::ListRetainedOwner;
use foks_snowpack::AllocationBudget;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc,
};

struct State {
    occupied: std::sync::atomic::AtomicBool,
    failed: std::sync::atomic::AtomicBool,
    generation: std::sync::atomic::AtomicU64,
    charged: std::sync::atomic::AtomicUsize,
    attempts: std::sync::atomic::AtomicU64,
    traffic: std::sync::atomic::AtomicU64,
    refusals: std::sync::atomic::AtomicU64,
    first_refusal: std::sync::atomic::AtomicU64,
}
pub(super) struct Budget {
    parent: Arc<dyn ListRetainedOwner>,
    state: State,
    terms: PageAllowanceTerms,
}
fn control_bytes() -> Result<usize> {
    // Pinned alloc::sync ArcInner repr(C): strong,weak AtomicUsize then data.
    // pad_to_align includes the tail; no estimate of allocator metadata/RSS.
    let counts = std::alloc::Layout::array::<std::sync::atomic::AtomicUsize>(2)
        .map_err(|_| Error::Bounds)?;
    let (layout, _) = counts
        .extend(std::alloc::Layout::new::<Budget>())
        .map_err(|_| Error::Bounds)?;
    Ok(layout.pad_to_align().size())
}
fn terms(l: Layout) -> Result<PageAllowanceTerms> {
    if !(2..=1024).contains(&l.path)
        || P::ListMacContinue.spec().maximum_row_bytes()? != 69782
        || P::RoleGenerations.spec().maximum_row_bytes()? != 39
        || P::ListMacContinue.spec().effective_rows()? != 15
        || P::RoleGenerations.spec().effective_rows()? != 7
    {
        return Err(Error::Bounds);
    }
    let (entry, body) = schemas(l.snowpack)?;
    let parameter_copies = add(&[mul(6, l.value)?, 97])?;
    let list_transport = transport(P::ListMacContinue, 15, 6, 137, l)?;
    let decoded_container = mul(15, l.decoded)?;
    let three_role_batches = mul(
        3,
        add(&[
            33,
            mul(7, l.option_generation)?,
            transport(P::RoleGenerations, 7, 16, 333, l)?,
        ])?,
    )?;
    let row_decode = add(&[
        mul(4, 4096 + 65536)?,
        8192,
        entry.decode,
        entry.encode,
        4096,
        body.decode,
    ])?;
    let row_append = add(&[
        4096,
        65536,
        512,
        mul(40, l.snowpack)?,
        entry.encode,
        4096,
        body.encode,
        4096,
        9,
        4096,
    ])?;
    let fifteen_row_decode = mul(15, row_decode)?;
    let fifteen_row_append = mul(15, row_append)?;
    let dynamic_credits = add(&[
        parameter_copies,
        list_transport,
        decoded_container,
        three_role_batches,
        fifteen_row_decode,
        fifteen_row_append,
    ])?;
    // Path reaches65 at the checked recursion boundary. RawVec max128 slots,
    // old64+new128 growth overlaps at192; original128+errorclone65 is193.
    // Two sets of65 cloned variant tags<=31 plus one typed wrong-tag diagnostic
    // expected b"0" / found<=31. Stack Error/Value/Decimal objects need no heap.
    let auxiliary_live = add(&[mul(128 + 65, l.path)?, mul(2 * 65, 31)?, 1, 31])?;
    Ok(PageAllowanceTerms {
        parameter_copies,
        list_transport,
        decoded_container,
        three_role_batches,
        fifteen_row_decode,
        fifteen_row_append,
        auxiliary_live,
        dynamic_credits,
        workspace_bytes: add(&[dynamic_credits, auxiliary_live])?,
        control_bytes: control_bytes()?,
    })
}

fn layout() -> Layout {
    Layout {
        value: std::mem::size_of::<crate::Value>(),
        sql_value: {
            #[cfg(feature = "native")]
            {
                Some(std::mem::size_of::<rusqlite::types::Value>())
            }
            #[cfg(not(feature = "native"))]
            {
                None
            }
        },
        cell: std::mem::size_of::<crate::page::Cell<'static>>(),
        decoded: std::mem::size_of::<super::Decoded>(),
        snowpack: std::mem::size_of::<foks_snowpack::Value>(),
        path: std::mem::size_of::<foks_snowpack::PathSegment>(),
        option_generation: std::mem::size_of::<Option<u64>>(),
    }
}
pub(crate) fn target_terms() -> Result<PageAllowanceTerms> {
    terms(layout())
}
impl State {
    fn new() -> Self {
        Self {
            occupied: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            charged: AtomicUsize::new(0),
            attempts: AtomicU64::new(0),
            traffic: AtomicU64::new(0),
            refusals: AtomicU64::new(0),
            first_refusal: AtomicU64::new(0),
        }
    }
}
impl Budget {
    pub(super) fn new(parent: Arc<dyn ListRetainedOwner>) -> Result<Arc<Self>> {
        let terms = match target_terms() {
            Ok(terms) => terms,
            Err(error) => {
                parent.fail();
                return Err(error);
            }
        };
        let total = match terms.workspace_bytes.checked_add(terms.control_bytes) {
            Some(total) => total,
            None => {
                parent.fail();
                return Err(Error::Bounds);
            }
        };
        if parent.failed() || !parent.reserve_page_workspace(total) || parent.failed() {
            parent.fail();
            return Err(Error::Bounds);
        }
        // Infallible Arc allocation follows process/instance OOM disposal. The
        // original parent already owns this reservation and is never refunded.
        Ok(Arc::new(Self {
            parent,
            state: State::new(),
            terms,
        }))
    }
    fn failed(&self) -> bool {
        self.state.failed.load(Ordering::SeqCst) || self.parent.failed()
    }
    pub(super) fn fail(&self) {
        self.state.failed.store(true, Ordering::SeqCst);
        self.parent.fail();
    }
    pub(super) fn run(
        self: &Arc<Self>,
        f: impl FnOnce() -> std::result::Result<PageProgress, PageFault>,
    ) -> std::result::Result<PageProgress, PageFault> {
        if self.failed() {
            self.fail();
            return Err(PageFault::Bounds);
        }
        if self
            .state
            .occupied
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.fail();
            return Err(PageFault::Storage);
        }
        struct Scope<'a> {
            budget: &'a Budget,
            complete: bool,
        }
        impl Drop for Scope<'_> {
            fn drop(&mut self) {
                if !self.complete {
                    self.budget.fail();
                }
                self.budget.state.occupied.store(false, Ordering::SeqCst);
            }
        }
        let mut scope = Scope {
            budget: self,
            complete: false,
        };
        if self
            .state
            .generation
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .is_err()
        {
            return Err(PageFault::Bounds);
        }
        self.state.charged.store(0, Ordering::SeqCst);
        // Restore owns the previous hook until f and every page-local owner has
        // dropped. It restores before Scope clears occupancy, also on unwind.
        let result = foks_snowpack::with_allocation_budget(self.clone(), f);
        if result.is_err() {
            self.fail();
        }
        let result = match result {
            Ok(_) if self.failed() => Err(PageFault::Bounds),
            other => other, // Preserve earlier domain/error provenance.
        };
        scope.complete = true;
        result
    }
}
impl AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        let ordinal = self
            .state
            .attempts
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1));
        let mut accepted =
            ordinal.is_ok() && !self.failed() && self.state.occupied.load(Ordering::SeqCst);
        let current = self.state.charged.load(Ordering::SeqCst);
        let next = current.checked_add(bytes);
        accepted &= next.is_some_and(|n| n <= self.terms.dynamic_credits);
        // Traffic is never reset between pages. Counter overflow is a failure.
        accepted &= self
            .state
            .traffic
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                n.checked_add(bytes as u64)
            })
            .is_ok();
        #[cfg(feature = "list-allocation-qualification")]
        {
            accepted = crate::allocation_trace::observe_page_attempt(bytes, accepted);
        }
        // Observation may invoke qualification refusal; no outer lock is held.
        accepted &= !self.failed();
        if accepted {
            // Only the synchronous occupied scope owns reserve calls; no Arc to
            // this hook escapes. Host callback reentry remains phase-guarded.
            self.state
                .charged
                .store(next.expect("accepted checked sum"), Ordering::SeqCst);
            true
        } else {
            let _ = self
                .state
                .refusals
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1));
            if let Ok(previous) = ordinal {
                let _ = self.state.first_refusal.compare_exchange(
                    0,
                    previous + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            }
            self.fail();
            false
        }
    }
}

#[cfg(test)]
mod tests;
