//! Shared desktop state, cached authorization facts, and mutation coordination.
//!
//! [`AppState`] is declared here; its methods are grouped by concern:
//!
//! - `scopes`: mutation scopes, per-scope views and mutation reservations.
//! - `catalog`: catalog loads, publication, generations, epochs and retired
//!   profiles.
//! - `lookup`: resolution of renderer-selected stores, items, members and
//!   federation targets against the accepted catalog.
//! - `chat`: chat-scope access and verified chat identities.
//! - `facts`: cached accounts, devices, rosters and federations.
//! - `grants`: file paths authorized by a drop or the native picker.

mod catalog;
mod chat;
mod facts;
mod grants;
mod lookup;
mod scopes;

use crate::agent::AgentHandle;
use crate::commands::accounts::{AccountDto, DeviceDto};
use crate::commands::groups::{FederationEntryDto, PartyDto};
use foks_desktop::{CatalogLoadToken, CatalogSnapshot};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

pub(super) use catalog::catalog_changed_during_read;
pub use scopes::MutationGuard;

pub const MAIN: &str = "main";

/// Locks a mutex, recovering the guard when an earlier holder panicked.
/// Every lock in this module uses this policy.
trait LockExt<T: ?Sized> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> LockExt<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum MutationScope {
    Root,
    Profile(String),
    LocalAliases,
}

#[derive(Default)]
struct MutationScopeState {
    in_flight: Arc<AtomicBool>,
    requires_refresh: Arc<AtomicBool>,
    generation: AtomicU64,
    chat_generation: AtomicU64,
    chat_identities: Mutex<HashMap<String, foks_agent_proto::chat::ChatScope>>,
    load_generation: AtomicU64,
    load: Mutex<Option<CatalogLoadToken>>,
    /// The highest mutation epoch a fresh listing of this scope's own profile
    /// has covered. Held per scope so a profile's own read settles that
    /// profile, which the root epoch cannot say on its behalf.
    fresh_epoch: AtomicU64,
}

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<AgentHandle>,
    pub(super) chat_views: Arc<Mutex<HashMap<String, Weak<AtomicBool>>>>,
    catalog_coordination: Arc<Mutex<()>>,
    next_catalog_generation: Arc<AtomicU64>,
    /// Counts the mutations that have retired catalog state. Compared against
    /// `catalog_fresh_epoch` to decide whether the next catalog listing must
    /// be read for this pass instead of being served from the agent's
    /// retained snapshot, which is only as fresh as its own lifetime.
    catalog_mutation_epoch: Arc<AtomicU64>,
    /// The highest mutation epoch a completed fresh listing of every profile
    /// has covered. A listing of one profile settles that profile's own
    /// scope instead; see [`MutationScopeState::fresh_epoch`].
    catalog_fresh_epoch: Arc<AtomicU64>,
    pub(super) catalog: Arc<Mutex<Option<CatalogSnapshot>>>,
    /// The scope this view was made for, and that scope's state. A view made
    /// by `scoped` shares every other field with the view it came from.
    scope: MutationScope,
    scope_state: Arc<MutationScopeState>,
    root: Arc<MutationScopeState>,
    scopes: Arc<Mutex<HashMap<MutationScope, Arc<MutationScopeState>>>>,
    pending_upload_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
    local_accounts: Arc<Mutex<HashMap<String, foks_agent_proto::AccountStoreRef>>>,
    /// Profiles whose catalog a mutation retired and no read has covered
    /// since. Their stores are absent from the catalog without being known
    /// to be gone, so a lookup answers with the unrefreshed vault rather
    /// than a missing store. Locked innermost and only briefly.
    retired_profiles: Arc<Mutex<HashSet<String>>>,
    pub(super) accounts: Arc<Mutex<HashMap<String, AccountDto>>>,
    pub(super) devices: Arc<Mutex<HashMap<String, Vec<DeviceDto>>>>,
    pub(super) rosters: Arc<Mutex<HashMap<String, Vec<PartyDto>>>>,
    pub(super) federations: Arc<Mutex<HashMap<String, Vec<FederationEntryDto>>>>,
}

impl AppState {
    pub fn new(agent: Arc<AgentHandle>) -> Self {
        let root = Arc::new(MutationScopeState::default());
        Self {
            agent,
            chat_views: Arc::default(),
            catalog_coordination: Arc::default(),
            next_catalog_generation: Arc::default(),
            catalog_mutation_epoch: Arc::default(),
            catalog_fresh_epoch: Arc::default(),
            catalog: Arc::default(),
            scope: MutationScope::Root,
            scope_state: Arc::clone(&root),
            root,
            scopes: Arc::new(Mutex::new(HashMap::from([(
                MutationScope::LocalAliases,
                Arc::new(MutationScopeState::default()),
            )]))),
            pending_upload_paths: Arc::default(),
            local_accounts: Arc::default(),
            retired_profiles: Arc::default(),
            accounts: Arc::default(),
            devices: Arc::default(),
            rosters: Arc::default(),
            federations: Arc::default(),
        }
    }

    // The accessors below read the state of this view's scope: the root
    // state for the view `new` returns, and the profile's or local aliases'
    // state for a view `scoped` returns.

    /// The catalog load running in this view's scope, if any.
    fn catalog_load(&self) -> &Mutex<Option<CatalogLoadToken>> {
        &self.scope_state.load
    }

    /// The generation of the catalog this view's scope last accepted.
    pub(super) fn catalog_generation(&self) -> &AtomicU64 {
        &self.scope_state.generation
    }

    /// The generation of the chat facts this view's scope last accepted.
    pub(super) fn chat_generation(&self) -> &AtomicU64 {
        &self.scope_state.chat_generation
    }

    /// The generation a catalog load in this view's scope must still match
    /// for its snapshot to be accepted.
    fn catalog_load_generation(&self) -> &AtomicU64 {
        &self.scope_state.load_generation
    }

    /// Set while this view's scope holds a mutation reservation.
    fn mutation_in_flight(&self) -> &Arc<AtomicBool> {
        &self.scope_state.in_flight
    }

    /// Set while a mutation in this view's scope has an unreconciled outcome.
    pub(super) fn mutation_requires_refresh(&self) -> &Arc<AtomicBool> {
        &self.scope_state.requires_refresh
    }
}
