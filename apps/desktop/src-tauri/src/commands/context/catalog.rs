//! Catalog loads and publication: generations, mutation epochs, freshness
//! and retired profiles.

use super::{AppState, LockExt, MutationGuard, MutationScope};
use crate::agent::AgentError;
use crate::commands::store_key::store_id;
use foks_desktop::{CatalogLoadToken, CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

impl AppState {
    fn begin_catalog_load(&self) -> (u64, CatalogLoadToken) {
        let retirement = self.next_generation();
        if self.scope == MutationScope::Root {
            for state in self.scopes.lock_unpoisoned().values() {
                state.load_generation.store(retirement, Ordering::Release);
                if let Some(token) = state.load.lock_unpoisoned().take() {
                    token.cancel();
                }
            }
        } else if self.mutation_profile().is_some() {
            self.root
                .load_generation
                .store(retirement, Ordering::Release);
            if let Some(token) = self.root.load.lock_unpoisoned().take() {
                token.cancel();
            }
        }
        let token = CatalogLoadToken::default();
        let mut live = self.catalog_load().lock_unpoisoned();
        if let Some(previous) = live.replace(token.clone()) {
            previous.cancel();
        }
        // Loading is not publication. Keep the accepted snapshot and its
        // dependent facts usable until a replacement has been validated.
        let generation = self.next_generation();
        self.catalog_load_generation()
            .store(generation, Ordering::Release);
        (generation, token)
    }

    /// Stores a loaded catalog. Returns false, storing nothing, when a later
    /// load or mutation replaced the generation this load began with.
    #[must_use]
    pub(in crate::commands) fn accept_catalog(
        &self,
        generation: u64,
        catalog: CatalogSnapshot,
    ) -> bool {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        self.accept_catalog_locked(generation, generation, catalog, false)
    }

    #[cfg(test)]
    pub(in crate::commands) fn publish_catalog(
        &self,
        load_generation: u64,
        catalog: CatalogSnapshot,
        publish: impl FnOnce(u64),
    ) -> bool {
        self.publish_catalog_snapshot(load_generation, catalog, |generation, _| {
            publish(generation)
        })
    }

    pub(in crate::commands) fn publish_catalog_snapshot(
        &self,
        load_generation: u64,
        catalog: CatalogSnapshot,
        publish: impl FnOnce(u64, &CatalogSnapshot),
    ) -> bool {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self
            .catalog_load()
            .lock_unpoisoned()
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
        let retained = self.catalog.lock_unpoisoned();
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
        if self.catalog_load_generation().load(Ordering::Acquire) != load_generation {
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
        let previous = self.catalog.lock_unpoisoned().clone();
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
                self.chat_generation().store(generation, Ordering::Release);
            }
            self.reconcile_local_accounts(&catalog);
            let mut retained = self.catalog.lock_unpoisoned();
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
                self.retired_profiles.lock_unpoisoned().remove(profile);
            }
            self.clear_profile_facts(profile);
            self.catalog_generation()
                .store(generation, Ordering::Release);
            self.advance_root_generation(if preserve_unchanged {
                generation
            } else {
                self.next_generation()
            });
            if complete {
                self.mutation_requires_refresh()
                    .store(false, Ordering::Release);
            }
            return true;
        }
        if self.scope != MutationScope::Root {
            return false;
        }
        let scopes = self.scopes.lock_unpoisoned();
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
            .lock_unpoisoned()
            .retain(|profile| !observed_profiles.contains(profile));
        let changed_profiles = preserve_unchanged.then(|| {
            let previous = self.catalog.lock_unpoisoned();
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
        *self.catalog.lock_unpoisoned() = Some(catalog);
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
        self.catalog_generation()
            .store(generation, Ordering::Release);
        if complete {
            self.mutation_requires_refresh()
                .store(false, Ordering::Release);
        }
        true
    }

    /// The current catalog snapshot and its generation. When `expected` names
    /// the generation an earlier `list_catalog` returned and a later load or
    /// mutation has replaced it, the read fails instead of answering from a
    /// snapshot the caller never saw.
    pub(in crate::commands) fn catalog_at(
        &self,
        expected: Option<u64>,
    ) -> Result<(u64, Option<CatalogSnapshot>), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        let generation = self.catalog_generation().load(Ordering::Acquire);
        if expected.is_some_and(|expected| expected != generation) {
            return Err(catalog_changed_during_read());
        }
        Ok((generation, self.catalog.lock_unpoisoned().clone()))
    }

    fn reconcile_local_accounts(&self, catalog: &CatalogSnapshot) {
        self.local_accounts.lock_unpoisoned().retain(|_, account| {
            (self.scope != MutationScope::Root || catalog.profiles.contains(&account.profile))
                && !catalog.inventory.iter().any(|inventory| {
                    inventory.profile == account.profile && inventory.accounts_complete
                })
        });
    }

    pub(in crate::commands) fn invalidate_alias_metadata(&self, profile: &str) {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if let Some(catalog) = self.catalog.lock_unpoisoned().as_mut() {
            catalog
                .profile_overviews
                .retain(|overview| overview.profile != profile);
        }
    }

    pub(super) fn next_generation(&self) -> u64 {
        self.next_catalog_generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn advance_root_generation(&self, generation: u64) {
        self.root
            .load_generation
            .store(generation, Ordering::Release);
        self.root.generation.store(generation, Ordering::Release);
    }

    pub(crate) fn invalidate_catalog(&self) {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.scope == MutationScope::LocalAliases {
            return;
        }
        self.catalog_mutation_epoch.fetch_add(1, Ordering::AcqRel);
        if let Some(profile) = self.mutation_profile() {
            self.retire_catalog_load();
            if let Some(catalog) = self.catalog.lock_unpoisoned().as_mut() {
                let mut local_accounts = self.local_accounts.lock_unpoisoned();
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
                .lock_unpoisoned()
                .insert(profile.to_owned());
            self.clear_profile_facts(profile);
            let generation = self.next_generation();
            self.catalog_load_generation()
                .store(generation, Ordering::Release);
            self.catalog_generation()
                .store(generation, Ordering::Release);
            self.chat_generation().store(generation, Ordering::Release);
            self.advance_root_generation(generation);
            return;
        }
        if let Some(token) = self.catalog_load().lock_unpoisoned().take() {
            token.cancel();
        }
        let generation = self.next_generation();
        self.catalog_load_generation()
            .store(generation, Ordering::Release);
        *self.catalog.lock_unpoisoned() = None;
        self.clear_group_facts();
        self.local_accounts.lock_unpoisoned().clear();
        for state in self.scopes.lock_unpoisoned().values() {
            state.generation.store(generation, Ordering::Release);
            state.chat_generation.store(generation, Ordering::Release);
            state.load_generation.store(generation, Ordering::Release);
            if let Some(token) = state.load.lock_unpoisoned().take() {
                token.cancel();
            }
        }
        // As with publication, a read must not attach the new generation to
        // a target or fact selected from the retired catalog.
        self.catalog_generation()
            .store(generation, Ordering::Release);
        self.chat_generation().store(generation, Ordering::Release);
    }

    /// Discards the cached items of the one store a write lands in. Every
    /// other store's items are still exactly what its own last read
    /// published, so a write to one vault does not make the renderer read
    /// every other vault of the profile again.
    ///
    /// The profile's full-item claim goes with them: with one store's items
    /// discarded, the profile has no longer been read whole.
    pub(in crate::commands) fn invalidate_catalog_items(&self, store: &CatalogStoreRef) {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        self.catalog_mutation_epoch.fetch_add(1, Ordering::AcqRel);
        self.retire_catalog_load();
        let mut catalog = self.catalog.lock_unpoisoned();
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
        self.catalog_generation()
            .store(generation, Ordering::Release);
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
    pub(in crate::commands) fn catalog_read_freshness(&self, requested: bool) -> (bool, u64) {
        let epoch = self.catalog_mutation_epoch.load(Ordering::Acquire);
        (
            requested
                || self.mutation_requires_refresh().load(Ordering::Acquire)
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
    pub(in crate::commands) fn note_fresh_catalog_read(&self, epoch: u64) {
        if self.mutation_profile().is_some() {
            self.scope_state
                .fresh_epoch
                .fetch_max(epoch, Ordering::AcqRel);
        } else {
            self.catalog_fresh_epoch.fetch_max(epoch, Ordering::AcqRel);
        }
    }

    pub(in crate::commands) fn begin_catalog_load_checked(
        &self,
    ) -> Result<(u64, CatalogLoadToken), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.root.in_flight.load(Ordering::Acquire)
            || self.scopes.lock_unpoisoned().iter().any(|(scope, state)| {
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
    pub(in crate::commands) fn begin_missing_catalog_for_mutation(
        &self,
        permit: &MutationGuard,
    ) -> Result<Option<(u64, CatalogLoadToken)>, AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        self.require_reservation(permit)?;
        if self
            .catalog
            .lock_unpoisoned()
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
    pub(super) fn retire_catalog_load(&self) {
        if let Some(token) = self.catalog_load().lock_unpoisoned().take() {
            token.cancel();
        }
        self.catalog_load_generation()
            .store(self.next_generation(), Ordering::Release);
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

pub(in crate::commands) fn catalog_changed_during_read() -> AgentError {
    AgentError::new(
        "catalog-required",
        "The vault changed while it was loading. Refresh and try again.",
        true,
    )
}
