//! Shared desktop state, cached authorization facts, and mutation coordination.

use crate::agent::{AgentError, AgentHandle};
use crate::commands::accounts::{AccountDto, DeviceDto};
use crate::commands::groups::{
    admission_not_resumable, member_not_actionable, member_role_from_dto, FederationEntryDto,
    MemberRole, PartyDto,
};
use crate::commands::validation::{
    invalid_request, invalid_response, require_profile_available, required_field,
    valid_device_member_id_hex, valid_operation_id_hex, valid_typed_entity_id_hex,
    AD_HOC_TEAM_ID_PREFIX, DEVICE_ID_PREFIX, HOST_ID_PREFIX, NAMED_TEAM_ID_PREFIX,
};
use crate::commands::vault::store_id;
use foks_desktop::{
    CatalogItem, CatalogLoadToken, CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

pub const MAIN: &str = "main";

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
    generation: Arc<AtomicU64>,
    load_generation: Arc<AtomicU64>,
    load: Arc<Mutex<Option<CatalogLoadToken>>>,
}

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<AgentHandle>,
    pub(super) chat_views: Arc<Mutex<HashMap<String, Weak<AtomicBool>>>>,
    catalog_load: Arc<Mutex<Option<CatalogLoadToken>>>,
    catalog_coordination: Arc<Mutex<()>>,
    next_catalog_generation: Arc<AtomicU64>,
    pub(super) catalog_generation: Arc<AtomicU64>,
    catalog_load_generation: Arc<AtomicU64>,
    pub(super) catalog: Arc<Mutex<Option<CatalogSnapshot>>>,
    mutation_in_flight: Arc<AtomicBool>,
    pub(super) mutation_requires_refresh: Arc<AtomicBool>,
    scope: MutationScope,
    scope_state: Arc<MutationScopeState>,
    root: Arc<MutationScopeState>,
    scopes: Arc<Mutex<HashMap<MutationScope, Arc<MutationScopeState>>>>,
    pending_drop_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
    local_accounts: Arc<Mutex<HashMap<String, foks_agent_proto::AccountStoreRef>>>,
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
            catalog_load: Arc::clone(&root.load),
            catalog_coordination: Arc::default(),
            next_catalog_generation: Arc::default(),
            catalog_generation: Arc::clone(&root.generation),
            catalog_load_generation: Arc::clone(&root.load_generation),
            catalog: Arc::default(),
            mutation_in_flight: Arc::clone(&root.in_flight),
            mutation_requires_refresh: Arc::clone(&root.requires_refresh),
            scope: MutationScope::Root,
            scope_state: Arc::clone(&root),
            root,
            scopes: Arc::new(Mutex::new(HashMap::from([(
                MutationScope::LocalAliases,
                Arc::new(MutationScopeState::default()),
            )]))),
            pending_drop_paths: Arc::default(),
            local_accounts: Arc::default(),
            accounts: Arc::default(),
            devices: Arc::default(),
            rosters: Arc::default(),
            federations: Arc::default(),
        }
    }

    fn scoped(&self, scope: MutationScope) -> Result<Self, AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut scopes = self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !scopes.contains_key(&scope) && scopes.len() >= 257 {
            scopes.retain(|scope, state| {
                *scope == MutationScope::LocalAliases
                    || Arc::strong_count(state) > 1
                    || state.in_flight.load(Ordering::Acquire)
                    || state.requires_refresh.load(Ordering::Acquire)
            });
            if scopes.len() >= 257 {
                return Err(AgentError::new(
                    "busy",
                    "Too many unreconciled mutation scopes. Refresh the vault before continuing.",
                    false,
                ));
            }
        }
        let state = scopes
            .entry(scope.clone())
            .or_insert_with(|| {
                let state = MutationScopeState::default();
                let generation = self.root.generation.load(Ordering::Acquire);
                state.generation.store(generation, Ordering::Release);
                state.load_generation.store(generation, Ordering::Release);
                Arc::new(state)
            })
            .clone();
        let mut view = self.clone();
        view.scope = scope;
        view.catalog_load = Arc::clone(&state.load);
        view.catalog_generation = Arc::clone(&state.generation);
        view.catalog_load_generation = Arc::clone(&state.load_generation);
        view.mutation_in_flight = Arc::clone(&state.in_flight);
        view.mutation_requires_refresh = Arc::clone(&state.requires_refresh);
        view.scope_state = state;
        Ok(view)
    }

    pub(super) fn for_profile(&self, profile: &str) -> Result<Self, AgentError> {
        if !crate::commands::validation::valid_response_text(profile, 256) {
            return Err(invalid_request("Provide a valid server profile name."));
        }
        self.scoped(MutationScope::Profile(profile.to_owned()))
    }

    pub(super) fn for_store(&self, store: &str) -> Result<Self, AgentError> {
        if store.len() > 2048 {
            return Err(invalid_request("Select a valid vault store."));
        }
        let value: serde_json::Value = serde_json::from_str(store)
            .map_err(|_| invalid_request("Select a valid vault store."))?;
        let profile = value
            .get("profile")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid_request("Select a valid vault store."))?;
        self.for_profile(profile)
    }

    pub(super) fn for_local_aliases(&self) -> Result<Self, AgentError> {
        self.scoped(MutationScope::LocalAliases)
    }

    pub(super) fn mutation_profile(&self) -> Option<&str> {
        match &self.scope {
            MutationScope::Profile(profile) => Some(profile),
            _ => None,
        }
    }

    fn clear_profile_facts(&self, profile: &str) {
        self.accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|_, account| account.profile != profile);
        let belongs = |store: &String| {
            serde_json::from_str::<serde_json::Value>(store)
                .ok()
                .and_then(|value| {
                    value
                        .get("profile")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .is_some_and(|value| value == profile)
        };
        self.devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|store, _| !belongs(store));
        self.rosters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|store, _| !belongs(store));
        self.federations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|store, _| !belongs(store));
    }

    fn clear_group_facts(&self) {
        self.accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.rosters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.federations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    pub(super) fn retain_accounts(
        &self,
        generation: u64,
        accounts: &[AccountDto],
    ) -> Result<(), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.catalog_generation.load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        *self
            .accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = accounts
            .iter()
            .cloned()
            .map(|account| (account.store.clone(), account))
            .collect();
        Ok(())
    }

    /// Install freshly read authorization facts and bind the operation target
    /// under one coordination lock. Earlier metadata reads cannot replace the
    /// prepared facts between their installation and synchronous validation.
    pub(super) fn with_prepared_group_facts<T>(
        &self,
        permit: &MutationGuard,
        prepared: crate::commands::preparation::PreparedGroupFacts,
        select: impl FnOnce() -> Result<T, AgentError>,
    ) -> Result<T, AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Arc::ptr_eq(&self.mutation_in_flight, &permit.0)
            || !self.mutation_in_flight.load(Ordering::Acquire)
        {
            return Err(invalid_request(
                "The mutation reservation belongs to another session.",
            ));
        }
        if self.catalog_generation.load(Ordering::Acquire) != prepared.generation {
            return Err(catalog_changed_during_group_read());
        }
        if let Some((profile, accounts)) = prepared.accounts {
            if accounts.iter().any(|account| account.profile != profile) {
                return Err(invalid_response(
                    "The account identities belong to another profile.",
                ));
            }
            let mut retained = self
                .accounts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            retained.retain(|_, account| account.profile != profile);
            retained.extend(
                accounts
                    .into_iter()
                    .map(|account| (account.store.clone(), account)),
            );
        }
        if let Some(parties) = prepared.parties {
            self.rosters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(prepared.store.clone(), parties);
        }
        if let Some(federation) = prepared.federation {
            self.federations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(prepared.store, federation);
        }
        select()
    }

    pub(super) fn retain_roster(
        &self,
        generation: u64,
        store: String,
        parties: &[PartyDto],
    ) -> Result<(), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.catalog_generation.load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        self.rosters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(store, parties.to_vec());
        Ok(())
    }

    pub(super) fn retain_devices(
        &self,
        generation: u64,
        store: String,
        devices: &[DeviceDto],
    ) -> Result<(), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.catalog_generation.load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        self.devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(store, devices.to_vec());
        Ok(())
    }

    pub(super) fn retain_federation(
        &self,
        generation: u64,
        store: String,
        entries: &[FederationEntryDto],
    ) -> Result<(), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.catalog_generation.load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        self.federations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(store, entries.to_vec());
        Ok(())
    }

    pub(super) fn retain_group_details(
        &self,
        generation: u64,
        store: String,
        parties: Option<&[PartyDto]>,
        federation: Option<&[FederationEntryDto]>,
    ) -> Result<(), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.catalog_generation.load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        if let Some(parties) = parties {
            self.rosters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(store.clone(), parties.to_vec());
        }
        if let Some(federation) = federation {
            self.federations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(store, federation.to_vec());
        }
        Ok(())
    }

    fn begin_catalog_load(&self) -> (u64, CatalogLoadToken) {
        let token = CatalogLoadToken::default();
        let mut live = self
            .catalog_load
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = live.replace(token.clone()) {
            previous.cancel();
        }
        // Loading is not publication. Keep the accepted snapshot and its
        // dependent facts usable until a replacement has been validated.
        let generation = self.next_generation();
        self.catalog_load_generation
            .store(generation, Ordering::Release);
        (generation, token)
    }

    /// Stores a loaded catalog. Returns false, storing nothing, when a later
    /// load or mutation replaced the generation this load began with.
    #[must_use]
    pub(super) fn accept_catalog(&self, generation: u64, catalog: CatalogSnapshot) -> bool {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.catalog_load_generation.load(Ordering::Acquire) != generation {
            return false;
        }
        if let Some(profile) = self.mutation_profile() {
            if catalog.profiles != [profile]
                || catalog
                    .full_item_reads
                    .iter()
                    .flatten()
                    .any(|candidate| candidate != profile)
                || catalog
                    .stores
                    .iter()
                    .any(|store| store.profile() != profile)
                || catalog
                    .known_stores
                    .iter()
                    .any(|store| store.profile() != profile)
                || catalog
                    .items
                    .iter()
                    .any(|item| item.store.profile() != profile)
                || catalog
                    .inventory
                    .iter()
                    .any(|inventory| inventory.profile != profile)
                || catalog
                    .profile_overviews
                    .iter()
                    .any(|overview| overview.profile != profile)
                || catalog
                    .blocked_profiles
                    .iter()
                    .any(|blocked| blocked != profile)
                || catalog
                    .failures
                    .iter()
                    .any(|failure| failure_profile(failure) != profile)
            {
                return false;
            }
            let complete = profile_catalog_complete(&catalog, profile);
            self.reconcile_local_accounts(&catalog);
            let mut retained = self
                .catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let retained = retained.get_or_insert_with(CatalogSnapshot::default);
            remove_profile_catalog(retained, profile);
            if !retained.profiles.iter().any(|existing| existing == profile) {
                retained.profiles.push(profile.to_owned());
            }
            retained.stores.extend(catalog.stores);
            retained.known_stores.extend(catalog.known_stores);
            retained.items.extend(catalog.items);
            if let Some(full_item_reads) = &mut retained.full_item_reads {
                full_item_reads.extend(catalog.full_item_reads.into_iter().flatten());
            }
            retained.inventory.extend(catalog.inventory);
            retained.profile_overviews.extend(catalog.profile_overviews);
            retained.failures.extend(catalog.failures);
            retained.blocked_profiles.extend(catalog.blocked_profiles);
            self.clear_profile_facts(profile);
            self.catalog_generation.store(generation, Ordering::Release);
            self.advance_root_generation();
            if complete {
                self.mutation_requires_refresh
                    .store(false, Ordering::Release);
            }
            return true;
        }
        if self.scope != MutationScope::Root {
            return false;
        }
        let scopes = self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (scope, state) in scopes.iter() {
            if let MutationScope::Profile(profile) = scope {
                if profile_catalog_complete(&catalog, profile) {
                    state.requires_refresh.store(false, Ordering::Release);
                }
            }
        }
        let complete = catalog.full_item_reads.is_some()
            && catalog.failures.is_empty()
            && catalog.blocked_profiles.is_empty()
            && catalog
                .profiles
                .iter()
                .all(|profile| profile_catalog_complete(&catalog, profile));
        self.reconcile_local_accounts(&catalog);
        // Facts describe the previously accepted catalog. Retire them only
        // when its replacement is published, under the same coordination
        // lock used by retain_* so an old read cannot restore them afterward.
        self.clear_group_facts();
        *self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(catalog);
        // Some reads capture the generation before selecting their target.
        // Publish it last: an old generation with a new target is rejected by
        // retain_*, but a new generation must never identify the old catalog.
        for state in scopes.values() {
            state.generation.store(generation, Ordering::Release);
            state.load_generation.store(generation, Ordering::Release);
        }
        self.catalog_generation.store(generation, Ordering::Release);
        if complete {
            self.mutation_requires_refresh
                .store(false, Ordering::Release);
        }
        true
    }

    /// The current catalog snapshot and its generation. When `expected` names
    /// the generation an earlier `list_catalog` returned and a later load or
    /// mutation has replaced it, the read fails instead of answering from a
    /// snapshot the caller never saw.
    pub(super) fn catalog_at(
        &self,
        expected: Option<u64>,
    ) -> Result<(u64, Option<CatalogSnapshot>), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.catalog_generation.load(Ordering::Acquire);
        if expected.is_some_and(|expected| expected != generation) {
            return Err(catalog_changed_during_read());
        }
        Ok((
            generation,
            self.catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        ))
    }

    fn reconcile_local_accounts(&self, catalog: &CatalogSnapshot) {
        self.local_accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|_, account| {
                (self.scope != MutationScope::Root || catalog.profiles.contains(&account.profile))
                    && !catalog.inventory.iter().any(|inventory| {
                        inventory.profile == account.profile && inventory.accounts_complete
                    })
            });
    }

    pub(super) fn invalidate_alias_metadata(&self, profile: &str) {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(catalog) = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            catalog
                .profile_overviews
                .retain(|overview| overview.profile != profile);
        }
    }

    fn next_generation(&self) -> u64 {
        self.next_catalog_generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn advance_root_generation(&self) {
        let generation = self.next_generation();
        self.root
            .load_generation
            .store(generation, Ordering::Release);
        self.root.generation.store(generation, Ordering::Release);
    }

    pub(super) fn invalidate_catalog(&self) {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.scope == MutationScope::LocalAliases {
            return;
        }
        if let Some(profile) = self.mutation_profile() {
            self.retire_catalog_load();
            if let Some(catalog) = self
                .catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut()
            {
                let mut local_accounts = self
                    .local_accounts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for store in &catalog.stores {
                    if let CatalogStoreSummary::Account { store } = store {
                        if store.profile == profile {
                            local_accounts.insert(
                                store_id(&CatalogStoreRef::Account(store.clone())),
                                store.clone(),
                            );
                        }
                    }
                }
                catalog.stores.retain(|store| store.profile() != profile);
                catalog.items.retain(|item| item.store.profile() != profile);
                if let Some(full_item_reads) = &mut catalog.full_item_reads {
                    full_item_reads.retain(|candidate| candidate != profile);
                }
                catalog
                    .inventory
                    .retain(|inventory| inventory.profile != profile);
                catalog
                    .profile_overviews
                    .retain(|overview| overview.profile != profile);
            }
            self.clear_profile_facts(profile);
            let generation = self.next_generation();
            self.catalog_load_generation
                .store(generation, Ordering::Release);
            self.catalog_generation.store(generation, Ordering::Release);
            self.advance_root_generation();
            return;
        }
        if let Some(token) = self
            .catalog_load
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            token.cancel();
        }
        let generation = self.next_generation();
        self.catalog_load_generation
            .store(generation, Ordering::Release);
        *self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.clear_group_facts();
        self.local_accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        for state in self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            state.generation.store(generation, Ordering::Release);
            state.load_generation.store(generation, Ordering::Release);
            if let Some(token) = state
                .load
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                token.cancel();
            }
        }
        // As with publication, a read must not attach the new generation to
        // a target or fact selected from the retired catalog.
        self.catalog_generation.store(generation, Ordering::Release);
    }

    pub(super) fn begin_catalog_load_checked(&self) -> Result<(u64, CatalogLoadToken), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.root.in_flight.load(Ordering::Acquire)
            || self
                .scopes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .any(|(scope, state)| {
                    (self.scope == MutationScope::Root || *scope == self.scope)
                        && state.in_flight.load(Ordering::Acquire)
                })
        {
            return Err(AgentError::new(
                "mutation-in-flight",
                "Wait for the current operation to finish before refreshing.",
                false,
            ));
        }
        Ok(self.begin_catalog_load())
    }

    /// Only the current mutation reservation can repair a missing catalog.
    /// Ordinary refreshes remain excluded until the caller dispatches or exits.
    pub(super) fn begin_missing_catalog_for_mutation(
        &self,
        permit: &MutationGuard,
    ) -> Result<Option<(u64, CatalogLoadToken)>, AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Arc::ptr_eq(&self.mutation_in_flight, &permit.0)
            || !self.mutation_in_flight.load(Ordering::Acquire)
        {
            return Err(invalid_request(
                "The mutation reservation belongs to another session.",
            ));
        }
        if self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|catalog| {
                self.mutation_profile().is_none_or(|profile| {
                    catalog
                        .inventory
                        .iter()
                        .any(|inventory| inventory.profile == profile)
                })
            })
        {
            return Ok(None);
        }
        Ok(Some(self.begin_catalog_load()))
    }

    // Called under catalog_coordination after acquiring a mutation reservation.
    // In-flight refreshes must not replace authorization facts during preflight.
    fn retire_catalog_load(&self) {
        if let Some(token) = self
            .catalog_load
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            token.cancel();
        }
        self.catalog_load_generation
            .store(self.next_generation(), Ordering::Release);
    }

    pub(super) fn selected_item(
        &self,
        store: &str,
        path: &str,
        version: u64,
    ) -> Result<CatalogItem, AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before reading an item.",
                true,
            ));
        };
        let item = catalog
            .items
            .iter()
            .find(|item| store_id(&item.store) == store && item.metadata.path == path);
        match item {
            Some(item) if catalog.store_blocked(&item.store) => Err(AgentError::new(
                "capability-unavailable",
                "The server does not permit reading this item.",
                false,
            )),
            Some(item) if item.metadata.version == version => Ok(item.clone()),
            Some(_) => {
                let mut error = AgentError::new(
                    "version-mismatch",
                    "This item changed. Refresh the vault before reading it.",
                    false,
                );
                error.fatal = true;
                Err(error)
            }
            None => Err(AgentError::new(
                "item-not-found",
                "This item is no longer in the vault.",
                false,
            )),
        }
    }

    pub(super) fn selected_team(&self, store: &str) -> Result<(String, String), AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault to view group details.",
                true,
            ));
        };
        let selected = catalog
            .stores
            .iter()
            .find_map(|candidate| match candidate {
                CatalogStoreSummary::Team { store: team, .. }
                    if store_id(&CatalogStoreRef::Team(team.clone())) == store =>
                {
                    Some((team.profile.clone(), team.team_alias.clone()))
                }
                _ => None,
            })
            .ok_or_else(|| {
                AgentError::new(
                    "store-not-found",
                    "This group is no longer in the vault.",
                    false,
                )
            })?;
        require_profile_available(catalog, &selected.0)?;
        Ok(selected)
    }

    pub(super) fn selected_account(
        &self,
        id: &str,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        self.selected_account_inner(id, true)
    }

    pub(super) fn local_account(
        &self,
        id: &str,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        self.selected_account_inner(id, false).or_else(|error| {
            self.local_accounts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(id)
                .cloned()
                .ok_or(error)
        })
    }

    fn selected_account_inner(
        &self,
        id: &str,
        require_available: bool,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault to access this account.",
                true,
            ));
        };
        let account = catalog.stores.iter().find_map(|candidate| match candidate {
            CatalogStoreSummary::Account { store }
                if store_id(&CatalogStoreRef::Account(store.clone())) == id =>
            {
                Some(store.clone())
            }
            _ => None,
        });
        let Some(account) = account else {
            return Err(AgentError::new(
                "store-not-found",
                "This account is no longer in the vault.",
                false,
            ));
        };
        if require_available {
            require_profile_available(catalog, &account.profile)?;
        }
        Ok(account)
    }

    pub(super) fn selected_account_dto(&self, id: &str) -> Result<AccountDto, AgentError> {
        let account = self.selected_account(id)?;
        let accounts = self
            .accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let selected = accounts.get(id).ok_or_else(|| {
            AgentError::new(
                "accounts-required",
                "Refresh accounts before using this account.",
                true,
            )
        })?;
        if selected.profile != account.profile || selected.alias != account.account_alias {
            return Err(invalid_response(
                "The cached account identity does not match the current vault.",
            ));
        }
        Ok(selected.clone())
    }

    pub(super) fn selected_device_target(
        &self,
        account_store_id: &str,
        device_id: &str,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        let account = self.selected_account(account_store_id)?;
        if !valid_device_member_id_hex(device_id) {
            return Err(invalid_request("Select a valid device ID."));
        }
        let devices = self
            .devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(retained) = devices.get(account_store_id) else {
            return Err(AgentError::new(
                "devices-required",
                "Refresh the device list before removing a device.",
                true,
            ));
        };
        let mut matches = retained.iter().filter(|device| device.id == device_id);
        let Some(device) = matches.next() else {
            return Err(AgentError::new(
                "device-not-found",
                "This device was not found in the current device list.",
                false,
            ));
        };
        if matches.next().is_some() {
            return Err(invalid_response(
                "The device list contains a duplicate device id.",
            ));
        }
        if device.current {
            return Err(AgentError::new(
                "current-device",
                "You cannot remove the current device.",
                false,
            ));
        }
        if !valid_typed_entity_id_hex(device_id, DEVICE_ID_PREFIX) {
            return Err(AgentError::new(
                "device-not-removable",
                "This device is not a removable software device.",
                false,
            ));
        }
        Ok(account)
    }

    pub(super) fn ensure_profile_available(&self, profile: &str) -> Result<(), AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        require_profile_available(
            catalog.as_ref().ok_or_else(|| {
                AgentError::new(
                    "catalog-required",
                    "Refresh the vault before making changes.",
                    true,
                )
            })?,
            profile,
        )
    }

    pub(super) fn selected_active_team_for_mutation(
        &self,
        id: &str,
    ) -> Result<foks_agent_proto::TeamStoreRef, AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before modifying this group.",
                true,
            ));
        };
        let team = catalog.stores.iter().find_map(|candidate| match candidate {
            CatalogStoreSummary::Team {
                store,
                kind,
                active,
                ..
            } if store_id(&CatalogStoreRef::Team(store.clone())) == id => {
                Some((store.clone(), kind.as_str(), *active))
            }
            _ => None,
        });
        let Some((team, kind, active)) = team else {
            return Err(AgentError::new(
                "store-not-found",
                "This group is no longer in the vault.",
                false,
            ));
        };
        require_profile_available(catalog, &team.profile)?;
        if !active {
            return Err(AgentError::new(
                "inactive-group",
                "This group is inactive. Complete its setup before modifying it.",
                false,
            ));
        }
        if kind != "named" {
            return Err(AgentError::new(
                "group-management-unavailable",
                "Member and federation settings are only available for named groups.",
                false,
            ));
        }
        Ok(team)
    }

    pub(super) fn selected_remote_named_team(
        &self,
        local_profile: &str,
        id: &str,
    ) -> Result<foks_agent_proto::TeamStoreRef, AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before federating this group.",
                true,
            ));
        };
        let remote = catalog.stores.iter().find_map(|candidate| match candidate {
            CatalogStoreSummary::Team {
                store,
                kind,
                active,
                ..
            } if store_id(&CatalogStoreRef::Team(store.clone())) == id => {
                Some((store.clone(), kind.as_str(), *active))
            }
            _ => None,
        });
        let Some((remote, kind, active)) = remote else {
            return Err(AgentError::new(
                "store-not-found",
                "The group to admit is no longer in the vault.",
                false,
            ));
        };
        require_profile_available(catalog, &remote.profile)?;
        if remote.profile == local_profile || kind != "named" || !active {
            return Err(invalid_request(
                "Select an active group from an external server.",
            ));
        }
        Ok(remote)
    }

    pub(super) fn selected_member_target(
        &self,
        selected_store_id: &str,
        username: &str,
    ) -> Result<(foks_agent_proto::TeamStoreRef, String, MemberRole), AgentError> {
        let team = self.selected_active_team_for_mutation(selected_store_id)?;
        let username = required_field(username, "Username is required.")?;
        let rosters = self
            .rosters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(roster) = rosters.get(selected_store_id) else {
            return Err(AgentError::new(
                "roster-required",
                "Refresh this group's member list before changing a member.",
                true,
            ));
        };
        let mut matches = roster
            .iter()
            .filter(|party| party.username.as_deref() == Some(username.as_str()));
        let Some(member) = matches.next() else {
            return Err(member_not_actionable());
        };
        if matches.next().is_some() || member.party_kind != "user" || !member.locally_manageable {
            return Err(member_not_actionable());
        }
        let current = member_role_from_dto(&member.destination_role)?;
        let party_id_hex = member.party_id_hex.clone();
        drop(rosters);
        let account_id = store_id(&CatalogStoreRef::Account(
            foks_agent_proto::AccountStoreRef {
                profile: team.profile.clone(),
                account_alias: team.account_alias.clone(),
            },
        ));
        let accounts = self
            .accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(account) = accounts.get(&account_id) else {
            return Err(AgentError::new(
                "accounts-required",
                "Refresh accounts before changing a group member.",
                true,
            ));
        };
        if account.username == username {
            return Err(member_not_actionable());
        }
        Ok((team, party_id_hex, current))
    }

    pub(super) fn selected_inactive_admission(
        &self,
        store_id: &str,
        operation_id: &str,
    ) -> Result<(foks_agent_proto::TeamStoreRef, FederationEntryDto), AgentError> {
        let team = self.selected_active_team_for_mutation(store_id)?;
        if !valid_operation_id_hex(operation_id) {
            return Err(invalid_request(
                "Choose a valid pending admission to re-run.",
            ));
        }
        let federations = self
            .federations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entries) = federations.get(store_id) else {
            return Err(AgentError::new(
                "federation-required",
                "Refresh this group's federation list before re-running an admission.",
                true,
            ));
        };
        let mut matches = entries.iter().filter(|entry| {
            !entry.active && entry.operation_id_hex.as_deref() == Some(operation_id)
        });
        let Some(entry) = matches.next() else {
            return Err(admission_not_resumable());
        };
        if matches.next().is_some() {
            return Err(admission_not_resumable());
        }
        let entry = entry.clone();
        drop(federations);
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entry.remote_profile == team.profile
            || !catalog
                .as_ref()
                .is_some_and(|catalog| catalog.profiles.contains(&entry.remote_profile))
        {
            return Err(admission_not_resumable());
        }
        require_profile_available(
            catalog
                .as_ref()
                .ok_or_else(catalog_changed_during_group_read)?,
            &entry.remote_profile,
        )?;
        Ok((team, entry))
    }

    pub(super) fn selected_active_federation_target(
        &self,
        store_id: &str,
        remote_host_id_hex: &str,
        remote_team_id_hex: &str,
    ) -> Result<(foks_agent_proto::TeamStoreRef, FederationEntryDto), AgentError> {
        let team = self.selected_active_team_for_mutation(store_id)?;
        if !valid_typed_entity_id_hex(remote_host_id_hex, HOST_ID_PREFIX)
            || !(valid_typed_entity_id_hex(remote_team_id_hex, NAMED_TEAM_ID_PREFIX)
                || valid_typed_entity_id_hex(remote_team_id_hex, AD_HOC_TEAM_ID_PREFIX))
        {
            return Err(invalid_request(
                "Choose an active federated group from the refreshed list.",
            ));
        }
        let federations = self
            .federations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries = federations.get(store_id).ok_or_else(|| {
            AgentError::new(
                "federation-required",
                "Refresh this group's federation list before removing a federated group.",
                true,
            )
        })?;
        let matching = entries
            .iter()
            .filter(|entry| {
                entry.active
                    && entry.operation_id_hex.is_some()
                    && entry.remote_host_id_hex == remote_host_id_hex
                    && entry.remote_team_id_hex == remote_team_id_hex
            })
            .collect::<Vec<_>>();
        let [entry] = matching.as_slice() else {
            return Err(invalid_request(
                "The federated group target is missing or ambiguous.",
            ));
        };
        let entry = (*entry).clone();
        drop(federations);
        let rosters = self
            .rosters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let roster = rosters.get(store_id).ok_or_else(|| {
            AgentError::new(
                "roster-required",
                "Refresh this group's member list before removing a federated group.",
                true,
            )
        })?;
        let matching = roster
            .iter()
            .filter(|party| {
                party.party_id_hex == remote_team_id_hex
                    && party.scoped_host_id_hex.as_deref() == Some(remote_host_id_hex)
                    && !party.locally_manageable
                    && matches!(party.party_kind.as_str(), "named-team" | "ad-hoc-team")
                    && party.destination_role == entry.destination
            })
            .count();
        if matching != 1 {
            return Err(invalid_request(
                "The federated group does not match the refreshed member list.",
            ));
        }
        Ok((team, entry))
    }

    pub(super) fn selected_store(
        &self,
        id: &str,
    ) -> Result<(CatalogStoreRef, Option<bool>), AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "The vault catalog is not loaded. Refresh to continue.",
                true,
            ));
        };
        let selected = catalog
            .stores
            .iter()
            .find_map(|candidate| match candidate {
                CatalogStoreSummary::Account { store }
                    if store_id(&CatalogStoreRef::Account(store.clone())) == id =>
                {
                    Some((CatalogStoreRef::Account(store.clone()), None))
                }
                CatalogStoreSummary::Team { store, active, .. }
                    if store_id(&CatalogStoreRef::Team(store.clone())) == id =>
                {
                    Some((CatalogStoreRef::Team(store.clone()), Some(*active)))
                }
                _ => None,
            })
            .ok_or_else(|| {
                AgentError::new(
                    "store-not-found",
                    "This vault is no longer available.",
                    false,
                )
            })?;
        let profile = match &selected.0 {
            CatalogStoreRef::Account(store) => &store.profile,
            CatalogStoreRef::Team(store) => &store.profile,
        };
        require_profile_available(catalog, profile)?;
        Ok(selected)
    }

    pub(super) fn selected_create_store(&self, id: &str) -> Result<CatalogStoreRef, AgentError> {
        let (store, active) = self.selected_store(id)?;
        match active {
            Some(false) => Err(AgentError::new(
                "inactive-group",
                "This group is inactive. Complete its setup before modifying it.",
                false,
            )),
            Some(true) | None => Ok(store),
        }
    }

    pub(super) fn selected_mutation_item(
        &self,
        store: &str,
        path: &str,
        version: u64,
    ) -> Result<CatalogItem, AgentError> {
        match self.selected_item(store, path, version) {
            Ok(item) => match self.selected_store(store)?.1 {
                Some(false) => Err(AgentError::new(
                    "inactive-group",
                    "This group is inactive. Complete its setup before modifying it.",
                    false,
                )),
                Some(true) | None => Ok(item),
            },
            Err(error) if error.code == "version-mismatch" || error.code == "item-not-found" => {
                Err(AgentError::new(
                    "conflict",
                    "This item changed or was removed. Refresh the vault before modifying it.",
                    false,
                ))
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn record_drop_paths(&self, paths: &[PathBuf]) -> Vec<String> {
        let mut pending = self
            .pending_drop_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.clear();
        let wire_paths: Vec<String> = paths
            .iter()
            .filter_map(|path| path.to_str().map(ToOwned::to_owned))
            .collect();
        // Accept only a single UTF-8 file path for drops. Record multi-drop
        // paths for UI reporting without authorizing them.
        if paths.len() == 1 && wire_paths.len() == 1 {
            pending.insert(wire_paths[0].clone(), paths[0].clone());
        }
        wire_paths
    }

    pub(crate) fn clear_drop_paths(&self) {
        self.pending_drop_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    pub(super) fn take_drop_path(&self, wire_path: &str) -> Result<PathBuf, AgentError> {
        self.pending_drop_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(wire_path)
            .ok_or_else(|| {
                AgentError::new(
                    "drop-not-authorized",
                    "Drop the file again to import it.",
                    false,
                )
            })
    }

    /// Inspection commands do not acquire the desktop mutation gate or retire
    /// the catalog. Their agent operations still perform checked profile access.
    pub(super) fn begin_catalog_action(
        &self,
        changes_catalog: bool,
    ) -> Result<Option<MutationGuard>, AgentError> {
        if !changes_catalog {
            return Ok(None);
        }
        let guard = self.begin_mutation()?;
        self.invalidate_catalog();
        Ok(Some(guard))
    }

    /// Acquire this guard before any mutation. Refusal is immediate:
    /// a second write cannot proceed until the prior operation reconciles.
    pub fn begin_mutation(&self) -> Result<MutationGuard, AgentError> {
        self.reserve_mutation(false)
    }

    fn reserve_mutation(&self, initialize: bool) -> Result<MutationGuard, AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let scopes = self
            .scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let blocked = (!initialize && self.root.requires_refresh.load(Ordering::Acquire))
            || scopes.iter().any(|(scope, state)| {
                (self.scope == MutationScope::Root || *scope == self.scope)
                    && state.requires_refresh.load(Ordering::Acquire)
            });
        if blocked {
            let mut error = AgentError::new(
                "ambiguous",
                "Refresh the vault to verify the previous change before making another one.",
                false,
            );
            error.ambiguous = true;
            return Err(error);
        }
        if self.root.in_flight.load(Ordering::Acquire)
            || scopes.iter().any(|(scope, state)| {
                (self.scope == MutationScope::Root || *scope == self.scope)
                    && state.in_flight.load(Ordering::Acquire)
            })
        {
            return Err(AgentError::new(
                "mutation-in-flight",
                "Another change is currently in progress. Wait for it to complete before trying again.",
                false,
            ));
        }
        self.mutation_in_flight.store(true, Ordering::Release);
        if self.scope == MutationScope::Root {
            for state in scopes.values() {
                if let Some(token) = state
                    .load
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    token.cancel();
                }
                state
                    .load_generation
                    .store(self.next_generation(), Ordering::Release);
            }
        }
        if self.scope != MutationScope::LocalAliases {
            self.retire_catalog_load();
        }
        if self.scope != MutationScope::Root {
            if let Some(token) = self
                .root
                .load
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                token.cancel();
            }
            self.root
                .load_generation
                .store(self.next_generation(), Ordering::Release);
        }
        Ok(MutationGuard(Arc::clone(&self.mutation_in_flight)))
    }

    /// Serializes credential initialization with other mutations while permitting
    /// bootstrap repair after an uncertain response. Initialization does not clear
    /// `mutation_requires_refresh`; a successful catalog reconciliation clears it.
    pub fn begin_initialization(&self) -> Result<MutationGuard, AgentError> {
        if self.scope != MutationScope::Root {
            return Err(invalid_request(
                "Initialization requires a root reservation.",
            ));
        }
        self.reserve_mutation(true)
    }
}

fn failure_profile(failure: &foks_desktop::CatalogFailure) -> &str {
    match &failure.scope {
        foks_desktop::CatalogFailureScope::Profile { profile, .. } => profile,
        foks_desktop::CatalogFailureScope::Store(store) => store.profile(),
    }
}

fn profile_catalog_complete(catalog: &CatalogSnapshot, profile: &str) -> bool {
    catalog
        .full_item_reads
        .as_ref()
        .is_some_and(|profiles| profiles.iter().any(|candidate| candidate == profile))
        && catalog
            .profiles
            .iter()
            .any(|candidate| candidate == profile)
        && catalog.inventory.iter().any(|inventory| {
            inventory.profile == profile && inventory.accounts_complete && inventory.teams_complete
        })
        && !catalog.profile_blocked(profile)
        && !catalog
            .failures
            .iter()
            .any(|failure| failure_profile(failure) == profile)
        && !catalog.profile_overviews.iter().any(|overview| {
            overview.profile == profile
                && matches!(
                    overview.server_status,
                    foks_agent_proto::ResponseResult::Error { .. }
                )
        })
}

fn remove_profile_catalog(catalog: &mut CatalogSnapshot, profile: &str) {
    if let Some(full_item_reads) = &mut catalog.full_item_reads {
        full_item_reads.retain(|candidate| candidate != profile);
    }
    catalog.stores.retain(|store| store.profile() != profile);
    catalog
        .known_stores
        .retain(|store| store.profile() != profile);
    catalog.items.retain(|item| item.store.profile() != profile);
    catalog
        .inventory
        .retain(|inventory| inventory.profile != profile);
    catalog
        .profile_overviews
        .retain(|overview| overview.profile != profile);
    catalog
        .failures
        .retain(|failure| failure_profile(failure) != profile);
    catalog
        .blocked_profiles
        .retain(|blocked| blocked != profile);
}

#[derive(Debug)]
pub struct MutationGuard(Arc<AtomicBool>);

impl Drop for MutationGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) fn catalog_changed_during_read() -> AgentError {
    AgentError::new(
        "catalog-required",
        "The vault changed while it was loading. Refresh and try again.",
        true,
    )
}

fn catalog_changed_during_group_read() -> AgentError {
    AgentError::new(
        "catalog-required",
        "The vault changed while group details were loading. Refresh and try again.",
        true,
    )
}
