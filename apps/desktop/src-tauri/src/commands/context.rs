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
use std::collections::{HashMap, HashSet};
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
    chat_generation: Arc<AtomicU64>,
    chat_identities: Mutex<HashMap<String, foks_agent_proto::chat::ChatScope>>,
    load_generation: Arc<AtomicU64>,
    load: Arc<Mutex<Option<CatalogLoadToken>>>,
    /// The highest mutation epoch a fresh listing of this scope's own profile
    /// has covered. Held per scope so a profile's own read settles that
    /// profile, which the root epoch cannot say on its behalf.
    fresh_epoch: Arc<AtomicU64>,
}

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<AgentHandle>,
    pub(super) chat_views: Arc<Mutex<HashMap<String, Weak<AtomicBool>>>>,
    catalog_load: Arc<Mutex<Option<CatalogLoadToken>>>,
    catalog_coordination: Arc<Mutex<()>>,
    next_catalog_generation: Arc<AtomicU64>,
    pub(super) catalog_generation: Arc<AtomicU64>,
    pub(super) chat_generation: Arc<AtomicU64>,
    catalog_load_generation: Arc<AtomicU64>,
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
    mutation_in_flight: Arc<AtomicBool>,
    pub(super) mutation_requires_refresh: Arc<AtomicBool>,
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
            catalog_load: Arc::clone(&root.load),
            catalog_coordination: Arc::default(),
            next_catalog_generation: Arc::default(),
            catalog_generation: Arc::clone(&root.generation),
            chat_generation: Arc::clone(&root.chat_generation),
            catalog_load_generation: Arc::clone(&root.load_generation),
            catalog_mutation_epoch: Arc::default(),
            catalog_fresh_epoch: Arc::default(),
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
            pending_upload_paths: Arc::default(),
            local_accounts: Arc::default(),
            retired_profiles: Arc::default(),
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
                state
                    .chat_generation
                    .store(self.next_generation(), Ordering::Release);
                state.load_generation.store(generation, Ordering::Release);
                Arc::new(state)
            })
            .clone();
        let mut view = self.clone();
        view.scope = scope;
        view.catalog_load = Arc::clone(&state.load);
        view.catalog_generation = Arc::clone(&state.generation);
        view.chat_generation = Arc::clone(&state.chat_generation);
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
        let retirement = self.next_generation();
        if self.scope == MutationScope::Root {
            for state in self
                .scopes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
            {
                state.load_generation.store(retirement, Ordering::Release);
                if let Some(token) = state
                    .load
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    token.cancel();
                }
            }
        } else if self.mutation_profile().is_some() {
            self.root
                .load_generation
                .store(retirement, Ordering::Release);
            if let Some(token) = self
                .root
                .load
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                token.cancel();
            }
        }
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
        self.accept_catalog_locked(generation, generation, catalog, false)
    }

    #[cfg(test)]
    pub(super) fn publish_catalog(
        &self,
        load_generation: u64,
        catalog: CatalogSnapshot,
        publish: impl FnOnce(u64),
    ) -> bool {
        self.publish_catalog_snapshot(load_generation, catalog, |generation, _| {
            publish(generation)
        })
    }

    pub(super) fn publish_catalog_snapshot(
        &self,
        load_generation: u64,
        catalog: CatalogSnapshot,
        publish: impl FnOnce(u64, &CatalogSnapshot),
    ) -> bool {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self
            .catalog_load
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(CatalogLoadToken::is_cancelled)
        {
            return false;
        }
        let observed_reads = catalog.store_reads.clone();
        let completed_profiles = catalog.full_item_reads.clone();
        let generation = self.next_generation();
        if !self.accept_catalog_locked(load_generation, generation, catalog, true) {
            return false;
        }
        let retained = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut published = retained.as_ref().expect("accepted catalog").clone();
        if let Some(profile) = self.mutation_profile() {
            let others = published
                .profiles
                .iter()
                .filter(|candidate| *candidate != profile)
                .cloned()
                .collect::<Vec<_>>();
            for other in others {
                remove_profile_catalog(&mut published, &other);
            }
            published.profiles.retain(|candidate| candidate == profile);
        }
        published.full_item_reads = completed_profiles;
        for read in &mut published.store_reads {
            read.state = observed_reads
                .iter()
                .find(|observed| observed.store == read.store)
                .map_or(foks_desktop::CatalogStoreReadState::NotLoaded, |observed| {
                    observed.state
                });
        }
        publish(generation, &published);
        true
    }

    fn accept_catalog_locked(
        &self,
        load_generation: u64,
        generation: u64,
        mut catalog: CatalogSnapshot,
        preserve_unchanged: bool,
    ) -> bool {
        if self.catalog_load_generation.load(Ordering::Acquire) != load_generation {
            return false;
        }
        let completed = catalog
            .profiles
            .iter()
            .filter(|profile| profile_catalog_complete(&catalog, profile))
            .cloned()
            .collect::<Vec<_>>();
        // Progressive snapshots name all configured profiles, including ones
        // not read yet. Only incoming inventory can answer a retirement;
        // retained inventory from an earlier publication cannot do so.
        let observed_profiles = catalog
            .inventory
            .iter()
            .map(|inventory| inventory.profile.clone())
            .collect::<HashSet<_>>();
        let complete = catalog.full_item_reads.is_some()
            && catalog.failures.is_empty()
            && catalog.blocked_profiles.is_empty()
            && catalog
                .profiles
                .iter()
                .all(|profile| completed.contains(profile));
        let previous = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(previous) = &previous {
            retain_pending_profiles(previous, &mut catalog);
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
                    .store_reads
                    .iter()
                    .any(|read| read.store.profile() != profile)
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
            let complete = completed.iter().any(|candidate| candidate == profile);
            if previous
                .as_ref()
                .is_none_or(|previous| !profile_chat_matches(previous, &catalog, profile))
            {
                self.chat_generation.store(generation, Ordering::Release);
            }
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
            retained.store_reads.extend(catalog.store_reads);
            if let Some(full_item_reads) = catalog.full_item_reads {
                retained
                    .full_item_reads
                    .get_or_insert_with(Vec::new)
                    .extend(full_item_reads);
            }
            retained.inventory.extend(catalog.inventory);
            retained.profile_overviews.extend(catalog.profile_overviews);
            retained.failures.extend(catalog.failures);
            retained.blocked_profiles.extend(catalog.blocked_profiles);
            if observed_profiles.contains(profile) {
                self.retired_profiles
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(profile);
            }
            self.clear_profile_facts(profile);
            self.catalog_generation.store(generation, Ordering::Release);
            self.advance_root_generation(if preserve_unchanged {
                generation
            } else {
                self.next_generation()
            });
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
                if completed.contains(profile) {
                    state.requires_refresh.store(false, Ordering::Release);
                }
                if previous
                    .as_ref()
                    .is_none_or(|previous| !profile_chat_matches(previous, &catalog, profile))
                {
                    state.chat_generation.store(generation, Ordering::Release);
                }
            }
        }
        self.retired_profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|profile| !observed_profiles.contains(profile));
        let changed_profiles = preserve_unchanged.then(|| {
            let previous = self
                .catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut profiles = catalog.profiles.clone();
            if let Some(previous) = previous.as_ref() {
                profiles.extend(previous.profiles.clone());
                profiles.retain(|profile| !profile_catalog_matches(previous, &catalog, profile));
            }
            profiles.extend(scopes.keys().filter_map(|scope| match scope {
                MutationScope::Profile(profile) if !catalog.profiles.contains(profile) => {
                    Some(profile.clone())
                }
                _ => None,
            }));
            profiles.sort();
            profiles.dedup();
            profiles
        });
        self.reconcile_local_accounts(&catalog);
        // Facts describe the previously accepted catalog. Retire them only
        // when its replacement is published, under the same coordination
        // lock used by retain_* so an old read cannot restore them afterward.
        if let Some(profiles) = &changed_profiles {
            for profile in profiles {
                self.clear_profile_facts(profile);
            }
        } else {
            self.clear_group_facts();
        }
        *self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(catalog);
        // Some reads capture the generation before selecting their target.
        // Publish it last: an old generation with a new target is rejected by
        // retain_*, but a new generation must never identify the old catalog.
        for (scope, state) in scopes.iter() {
            if let (MutationScope::Profile(profile), Some(changed)) = (scope, &changed_profiles) {
                if !changed.contains(profile) {
                    continue;
                }
            }
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

    fn advance_root_generation(&self, generation: u64) {
        self.root
            .load_generation
            .store(generation, Ordering::Release);
        self.root.generation.store(generation, Ordering::Release);
    }

    pub(crate) fn invalidate_catalog(&self) {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.scope == MutationScope::LocalAliases {
            return;
        }
        self.catalog_mutation_epoch.fetch_add(1, Ordering::AcqRel);
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
                for read in &mut catalog.store_reads {
                    if read.store.profile() == profile {
                        read.state = foks_desktop::CatalogStoreReadState::NotLoaded;
                    }
                }
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
            self.retired_profiles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(profile.to_owned());
            self.clear_profile_facts(profile);
            let generation = self.next_generation();
            self.catalog_load_generation
                .store(generation, Ordering::Release);
            self.catalog_generation.store(generation, Ordering::Release);
            self.chat_generation.store(generation, Ordering::Release);
            self.advance_root_generation(generation);
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
            state.chat_generation.store(generation, Ordering::Release);
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
        self.chat_generation.store(generation, Ordering::Release);
    }

    /// Discards the cached items of the one store a write lands in. Every
    /// other store's items are still exactly what its own last read
    /// published, so a write to one vault does not make the renderer read
    /// every other vault of the profile again.
    ///
    /// The profile's full-item claim goes with them: with one store's items
    /// discarded, the profile has no longer been read whole.
    pub(super) fn invalidate_catalog_items(&self, store: &CatalogStoreRef) {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.catalog_mutation_epoch.fetch_add(1, Ordering::AcqRel);
        self.retire_catalog_load();
        let mut catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(catalog) = catalog.as_mut() {
            catalog.items.retain(|item| &item.store != store);
            for read in &mut catalog.store_reads {
                if &read.store == store {
                    read.state = foks_desktop::CatalogStoreReadState::NotLoaded;
                }
            }
            if let Some(profiles) = &mut catalog.full_item_reads {
                profiles.retain(|profile| profile != store.profile());
            }
        }
        let generation = self.next_generation();
        self.catalog_generation.store(generation, Ordering::Release);
        self.advance_root_generation(generation);
    }

    /// Whether the catalog listing that is about to run must read every
    /// store's first page for itself, with the mutation epoch it will cover.
    ///
    /// A listing is read fresh when a mutation has retired catalog state
    /// since the last fresh listing finished, while an ambiguous mutation is
    /// still unreconciled, or when the caller asks for it (the user's own
    /// Refresh). The epoch is cleared only by a fresh listing that publishes,
    /// so a load that fails or is retired leaves the next one fresh as well.
    /// A mutation that lands while a fresh listing runs raises the epoch past
    /// the one that listing covers, so the listing after it is fresh again.
    ///
    /// Freshness is settled per profile. A listing of one profile answers for
    /// that profile alone, so it is judged against the highest epoch either a
    /// listing of every profile or a listing of this one has covered.
    pub(super) fn catalog_read_freshness(&self, requested: bool) -> (bool, u64) {
        let epoch = self.catalog_mutation_epoch.load(Ordering::Acquire);
        (
            requested
                || self.mutation_requires_refresh.load(Ordering::Acquire)
                || epoch != self.settled_fresh_epoch(),
            epoch,
        )
    }

    /// The highest mutation epoch a fresh listing that covered this scope has
    /// published.
    fn settled_fresh_epoch(&self) -> u64 {
        let every_profile = self.catalog_fresh_epoch.load(Ordering::Acquire);
        match self.mutation_profile() {
            Some(_) => every_profile.max(self.scope_state.fresh_epoch.load(Ordering::Acquire)),
            None => every_profile,
        }
    }

    /// Records that a fresh listing covering `epoch` published its snapshot.
    /// A listing made in a profile's scope settles that profile only; one
    /// made outside any profile's scope read every profile and settles them
    /// all.
    pub(super) fn note_fresh_catalog_read(&self, epoch: u64) {
        if self.mutation_profile().is_some() {
            self.scope_state
                .fresh_epoch
                .fetch_max(epoch, Ordering::AcqRel);
        } else {
            self.catalog_fresh_epoch.fetch_max(epoch, Ordering::AcqRel);
        }
    }

    pub(super) fn selected_chat(
        &self,
        id: &str,
    ) -> Result<(u64, foks_agent_proto::TeamStoreRef), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let team = self.selected_active_team_for_mutation(id)?;
        self.require_chat_access(&team.profile)?;
        Ok((self.chat_generation.load(Ordering::Acquire), team))
    }

    fn require_chat_access(&self, profile: &str) -> Result<(), AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let catalog = catalog.as_ref().ok_or_else(catalog_changed_during_read)?;
        if catalog.inventory.iter().any(|entry| {
            entry.profile == profile && (!entry.accounts_complete || !entry.teams_complete)
        }) {
            return Err(catalog_changed_during_read());
        }
        if let Some(overview) = catalog
            .profile_overviews
            .iter()
            .find(|overview| overview.profile == profile)
        {
            let foks_agent_proto::ResponseResult::Success { value } = &overview.server_status
            else {
                return Err(catalog_changed_during_read());
            };
            let status: foks_agent_proto::ServerStatusSnapshot =
                serde_json::from_value(value.clone())
                    .map_err(|_| invalid_response("Invalid chat server status."))?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if status.profile != profile
                || status.chat_supported != Some(true)
                || match status.compatibility {
                    foks_agent_proto::CompatibilityStatus::NotRequired => false,
                    foks_agent_proto::CompatibilityStatus::Validated {
                        expires_at,
                        capabilities,
                    } => expires_at <= now || !capabilities.contains("chat"),
                    _ => true,
                }
            {
                return Err(AgentError::new(
                    "capability-unavailable",
                    "Chat access must be revalidated for this profile.",
                    true,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn accept_chat_scope(
        &self,
        id: &str,
        generation: u64,
        scope: &foks_agent_proto::chat::ChatScope,
    ) -> Result<(), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.chat_generation.load(Ordering::Acquire) != generation {
            return Err(AgentError::new(
                "chat-restart",
                "Chat access changed during the request.",
                true,
            ));
        }
        if self.selected_active_team_for_mutation(id)? != scope.store {
            return Err(invalid_response("The selected chat identity changed."));
        }
        self.require_chat_access(&scope.store.profile)?;
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(overview) = catalog.as_ref().and_then(|catalog| {
            catalog
                .profile_overviews
                .iter()
                .find(|overview| overview.profile == scope.store.profile)
        }) {
            if let foks_agent_proto::ResponseResult::Success { value } = &overview.server_status {
                if value
                    .get("host")
                    .and_then(|host| host.get("host_id_hex"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|host| host != scope.host)
                {
                    self.chat_generation
                        .store(self.next_generation(), Ordering::Release);
                    return Err(invalid_response("The selected chat host changed."));
                }
            }
        }
        let mut identities = self
            .scope_state
            .chat_identities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if identities.get(id).is_some_and(|previous| previous != scope) {
            self.chat_generation
                .store(self.next_generation(), Ordering::Release);
            identities.insert(id.to_owned(), scope.clone());
            return Err(AgentError::new(
                "chat-restart",
                "The verified chat identity changed.",
                true,
            ));
        }
        if !identities.contains_key(id) && identities.len() >= 4096 {
            return Err(invalid_request("Too many verified chat identities."));
        }
        identities.insert(id.to_owned(), scope.clone());
        Ok(())
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
            Some(_) => Err(AgentError::new(
                "item-version-mismatch",
                "This item changed. Refresh the vault before reading it.",
                true,
            )),
            None => Err(AgentError::new(
                "item-not-found",
                "This item is no longer in the vault.",
                false,
            )),
        }
    }

    /// The error for a store the catalog does not list because a mutation
    /// retired its profile's catalog and no read has covered the profile
    /// since. The store is not known to be gone, so the answer names the
    /// unrefreshed vault and invites a retry, as an incomplete read's does.
    fn retired_store_error(&self, id: &str) -> Option<AgentError> {
        let value: serde_json::Value = serde_json::from_str(id).ok()?;
        let profile = value.get("profile")?.as_str()?;
        let retired = self
            .retired_profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(profile);
        retired.then(|| {
            AgentError::new(
                "catalog-required",
                format!("The vault for {profile} changed and has not been read again. Refresh and try again."),
                true,
            )
        })
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
                self.retired_store_error(store)
                    .or_else(|| unrefreshed_store_error(catalog, store))
                    .unwrap_or_else(|| {
                    AgentError::new(
                        "store-not-found",
                        "This group is currently inaccessible. There may have been a server issue or you may have been removed.",
                        false,
                    )
                })
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
            return Err(self
                .retired_store_error(id)
                .or_else(|| unrefreshed_store_error(catalog, id))
                .unwrap_or_else(|| {
                    AgentError::new(
                        "store-not-found",
                        "This account is no longer in the vault.",
                        false,
                    )
                }));
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
            return Err(self.retired_store_error(id)
                    .or_else(|| unrefreshed_store_error(catalog, id))
                    .unwrap_or_else(|| {
                AgentError::new(
                    "store-not-found",
                    "This group is currently inaccessible. There may have been a server issue or you may have been removed.",
                    false,
                )
            }));
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
            return Err(self
                .retired_store_error(id)
                .or_else(|| unrefreshed_store_error(catalog, id))
                .unwrap_or_else(|| {
                    AgentError::new(
                        "store-not-found",
                        "The group to admit is no longer in the vault.",
                        false,
                    )
                }));
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
                self.retired_store_error(id)
                    .or_else(|| unrefreshed_store_error(catalog, id))
                    .unwrap_or_else(|| {
                        AgentError::new(
                            "store-not-found",
                            "This vault is no longer available.",
                            false,
                        )
                    })
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
            Err(error)
                if error.code == "item-version-mismatch" || error.code == "item-not-found" =>
            {
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
            .pending_upload_paths
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
        self.pending_upload_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    pub(crate) fn record_picked_path(&self, path: PathBuf) -> Result<String, AgentError> {
        let wire_path = path.to_str().map(ToOwned::to_owned).ok_or_else(|| {
            AgentError::new(
                "upload-source",
                "The selected file path is not valid UTF-8.",
                false,
            )
        })?;
        let mut pending = self
            .pending_upload_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.clear();
        pending.insert(wire_path.clone(), path);
        Ok(wire_path)
    }

    pub(super) fn upload_path(&self, wire_path: &str) -> Result<PathBuf, AgentError> {
        self.pending_upload_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(wire_path)
            .cloned()
            .ok_or_else(|| {
                AgentError::new(
                    "drop-not-authorized",
                    "Choose or drop the file again to import it.",
                    false,
                )
            })
    }

    pub(super) fn release_upload_path(&self, wire_path: &str) {
        self.pending_upload_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(wire_path);
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
        // Local alias updates only affect local display names and do not modify
        // catalog data. Therefore, do not cancel ongoing catalog loads (neither
        // scoped nor root). Cancelling an in-flight root load would abort an
        // active boot or reconciliation request, forcing the frontend to restart it.
        if self.scope != MutationScope::LocalAliases {
            self.retire_catalog_load();
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

fn retain_pending_profiles(previous: &CatalogSnapshot, incoming: &mut CatalogSnapshot) {
    for profile in incoming.profiles.clone() {
        if incoming
            .inventory
            .iter()
            .any(|entry| entry.profile == profile)
        {
            if profile_chat_matches(previous, incoming, &profile)
                && !incoming.profile_blocked(&profile)
            {
                for read in &mut incoming.store_reads {
                    if read.store.profile() == profile
                        && read.state == foks_desktop::CatalogStoreReadState::NotLoaded
                        && previous.store_reads.iter().any(|old| {
                            old.store == read.store
                                && old.state == foks_desktop::CatalogStoreReadState::Complete
                        })
                        && !incoming
                            .failures
                            .iter()
                            .any(|failure| failure_profile(failure) == profile)
                    {
                        read.state = foks_desktop::CatalogStoreReadState::Complete;
                        incoming.items.extend(
                            previous
                                .items
                                .iter()
                                .filter(|item| item.store == read.store)
                                .cloned(),
                        );
                    }
                }
            }
            continue;
        }
        if incoming.profile_blocked(&profile)
            || incoming
                .failures
                .iter()
                .any(|failure| failure_profile(failure) == profile)
            || !previous.inventory.iter().any(|entry| {
                entry.profile == profile && entry.accounts_complete && entry.teams_complete
            })
        {
            continue;
        }
        remove_profile_catalog(incoming, &profile);
        incoming.stores.extend(
            previous
                .stores
                .iter()
                .filter(|store| store.profile() == profile)
                .cloned(),
        );
        incoming.known_stores.extend(
            previous
                .known_stores
                .iter()
                .filter(|store| store.profile() == profile)
                .cloned(),
        );
        incoming.items.extend(
            previous
                .items
                .iter()
                .filter(|item| item.store.profile() == profile)
                .cloned(),
        );
        incoming.store_reads.extend(
            previous
                .store_reads
                .iter()
                .filter(|read| read.store.profile() == profile)
                .cloned(),
        );
        incoming.inventory.extend(
            previous
                .inventory
                .iter()
                .filter(|entry| entry.profile == profile)
                .cloned(),
        );
        incoming.profile_overviews.extend(
            previous
                .profile_overviews
                .iter()
                .filter(|entry| entry.profile == profile)
                .cloned(),
        );
        incoming.failures.extend(
            previous
                .failures
                .iter()
                .filter(|failure| failure_profile(failure) == profile)
                .cloned(),
        );
        incoming.blocked_profiles.extend(
            previous
                .blocked_profiles
                .iter()
                .filter(|entry| **entry == profile)
                .cloned(),
        );
        if previous
            .full_item_reads
            .iter()
            .flatten()
            .any(|entry| *entry == profile)
        {
            incoming
                .full_item_reads
                .get_or_insert_with(Vec::new)
                .push(profile);
        }
    }
}

fn profile_chat_matches(left: &CatalogSnapshot, right: &CatalogSnapshot, profile: &str) -> bool {
    fn facts(catalog: &CatalogSnapshot, profile: &str) -> serde_json::Value {
        let mut stores = catalog
            .stores
            .iter()
            .filter(|store| store.profile() == profile)
            .map(|store| {
                match store {
                    CatalogStoreSummary::Account { store } => serde_json::json!([store]),
                    CatalogStoreSummary::Team {
                        store,
                        kind,
                        active,
                        creation_phase,
                        ..
                    } => serde_json::json!([store, kind, active, creation_phase]),
                }
                .to_string()
            })
            .collect::<Vec<_>>();
        stores.sort();
        let overview = catalog
            .profile_overviews
            .iter()
            .find(|overview| overview.profile == profile);
        let accounts = overview.map(|overview| match &overview.accounts {
            foks_agent_proto::ResponseResult::Success { value } => {
                let mut value = value.clone();
                if let Some(accounts) = value.as_array_mut() {
                    for account in accounts.iter_mut() {
                        if let Some(account) = account.as_object_mut() {
                            account.remove("local_alias");
                        }
                    }
                    accounts.sort_by_key(serde_json::Value::to_string);
                }
                value
            }
            result => serde_json::to_value(result).unwrap_or_default(),
        });
        let status = overview.map(|overview| match &overview.server_status {
            foks_agent_proto::ResponseResult::Success { value } => {
                let mut value = value.clone();
                if let Some(host) = value
                    .get_mut("host")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    host.remove("host_chain_sequence");
                    host.remove("merkle_epoch");
                }
                if let Some(compatibility) = value
                    .get_mut("compatibility")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    if let Some(expires_at) = compatibility
                        .remove("expires_at")
                        .and_then(|value| value.as_u64())
                    {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        compatibility
                            .insert("lease_live".into(), serde_json::json!(expires_at > now));
                    }
                    if let Some(capabilities) = compatibility
                        .get_mut("capabilities")
                        .and_then(serde_json::Value::as_array_mut)
                    {
                        capabilities.retain(|capability| capability.as_str() == Some("chat"));
                    }
                }
                value
            }
            result => serde_json::to_value(result).unwrap_or_default(),
        });
        let failures = catalog.failures.iter().filter(|failure| matches!(&failure.scope,
            foks_desktop::CatalogFailureScope::Profile { profile: candidate, source } if candidate == profile && source != "KV catalog" && source != "known store index"
        )).map(|failure| format!("{:?}", failure)).collect::<Vec<_>>();
        serde_json::json!([
            catalog
                .profiles
                .iter()
                .any(|candidate| candidate == profile),
            stores,
            accounts,
            status,
            catalog.profile_blocked(profile),
            failures
        ])
    }
    facts(left, profile) == facts(right, profile)
}

fn profile_catalog_matches(left: &CatalogSnapshot, right: &CatalogSnapshot, profile: &str) -> bool {
    let contains = |catalog: &CatalogSnapshot| {
        catalog
            .profiles
            .iter()
            .any(|candidate| candidate == profile)
    };
    contains(left) == contains(right)
        && left
            .stores
            .iter()
            .filter(|entry| entry.profile() == profile)
            .eq(right
                .stores
                .iter()
                .filter(|entry| entry.profile() == profile))
        && left
            .known_stores
            .iter()
            .filter(|entry| entry.profile() == profile)
            .eq(right
                .known_stores
                .iter()
                .filter(|entry| entry.profile() == profile))
        && left
            .items
            .iter()
            .filter(|entry| entry.store.profile() == profile)
            .eq(right
                .items
                .iter()
                .filter(|entry| entry.store.profile() == profile))
        && left
            .store_reads
            .iter()
            .filter(|entry| entry.store.profile() == profile)
            .eq(right
                .store_reads
                .iter()
                .filter(|entry| entry.store.profile() == profile))
        && left
            .inventory
            .iter()
            .filter(|entry| entry.profile == profile)
            .eq(right
                .inventory
                .iter()
                .filter(|entry| entry.profile == profile))
        && left
            .profile_overviews
            .iter()
            .filter(|entry| entry.profile == profile)
            .eq(right
                .profile_overviews
                .iter()
                .filter(|entry| entry.profile == profile))
        && left
            .failures
            .iter()
            .filter(|entry| failure_profile(entry) == profile)
            .eq(right
                .failures
                .iter()
                .filter(|entry| failure_profile(entry) == profile))
        && left.profile_blocked(profile) == right.profile_blocked(profile)
        && left
            .full_item_reads
            .iter()
            .flatten()
            .any(|entry| entry == profile)
            == right
                .full_item_reads
                .iter()
                .flatten()
                .any(|entry| entry == profile)
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
        && catalog
            .profile_overviews
            .iter()
            .filter(|overview| overview.profile == profile)
            .all(|overview| {
                let foks_agent_proto::ResponseResult::Success { value } = &overview.server_status
                else {
                    return false;
                };
                let Ok(status) =
                    serde_json::from_value::<foks_agent_proto::ServerStatusSnapshot>(value.clone())
                else {
                    return false;
                };
                status.profile == profile
                    && match status.compatibility {
                        foks_agent_proto::CompatibilityStatus::NotRequired => true,
                        foks_agent_proto::CompatibilityStatus::Validated { expires_at, .. } => {
                            expires_at
                                > std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                        }
                        _ => false,
                    }
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
        .store_reads
        .retain(|read| read.store.profile() != profile);
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

/// The answer for a store that is absent from `catalog.stores` when its
/// profile's last read recorded its stores as incomplete, rather than
/// because the vault no longer holds it. The renderer keeps showing a profile's previous stores while
/// its read is incomplete, so a request for one of them has to say that the
/// vault has not been refreshed, and be retryable, instead of claiming the
/// store is gone. The store still authorizes nothing: only a complete read
/// of the profile does that.
fn unrefreshed_store_error(catalog: &CatalogSnapshot, id: &str) -> Option<AgentError> {
    let value: serde_json::Value = serde_json::from_str(id).ok()?;
    let profile = value.get("profile")?.as_str()?;
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("team");
    if !catalog
        .profiles
        .iter()
        .any(|candidate| candidate == profile)
    {
        return None;
    }
    // Every read of a profile records its inventory, complete or not. A
    // catalog with no record for the profile says nothing about a read, so
    // it is not the unrefreshed case.
    let inventory = catalog
        .inventory
        .iter()
        .find(|entry| entry.profile == profile)?;
    let complete = if kind == "account" {
        inventory.accounts_complete
    } else {
        inventory.teams_complete
    };
    if complete {
        return None;
    }
    Some(AgentError::new(
        "catalog-required",
        format!("The vault for {profile} has not been refreshed. Refresh and try again."),
        true,
    ))
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
