//! Mutation scopes, per-scope views of [`AppState`] and mutation
//! reservations.

use super::{AppState, LockExt, MutationScope, MutationScopeState};
use crate::agent::AgentError;
use crate::commands::store_key::StoreKey;
use crate::commands::validation::invalid_request;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

impl AppState {
    /// A view of this state for `scope`, whose scope state is created on
    /// first use. The view shares every field except `scope` and
    /// `scope_state` with `self`.
    fn scoped(&self, scope: MutationScope) -> Result<Self, AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        let mut scopes = self.scopes.lock_unpoisoned();
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
        view.scope_state = state;
        Ok(view)
    }

    pub(in crate::commands) fn for_profile(&self, profile: &str) -> Result<Self, AgentError> {
        if !crate::commands::validation::valid_response_text(profile, 256) {
            return Err(invalid_request("Provide a valid server profile name."));
        }
        self.scoped(MutationScope::Profile(profile.to_owned()))
    }

    pub(in crate::commands) fn for_store(&self, store: &str) -> Result<Self, AgentError> {
        if store.len() > 2048 {
            return Err(invalid_request("Select a valid vault store."));
        }
        let store =
            StoreKey::parse(store).ok_or_else(|| invalid_request("Select a valid vault store."))?;
        self.for_profile(store.profile())
    }

    pub(in crate::commands) fn for_local_aliases(&self) -> Result<Self, AgentError> {
        self.scoped(MutationScope::LocalAliases)
    }

    pub(in crate::commands) fn mutation_profile(&self) -> Option<&str> {
        match &self.scope {
            MutationScope::Profile(profile) => Some(profile),
            _ => None,
        }
    }

    /// Inspection commands do not acquire the desktop mutation gate or retire
    /// the catalog. Their agent operations still perform checked profile access.
    pub(in crate::commands) fn begin_catalog_action(
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

    /// Whether any scope holds a mutation reservation.
    pub fn mutation_in_progress(&self) -> bool {
        self.root.in_flight.load(Ordering::Acquire)
            || self
                .scopes
                .lock_unpoisoned()
                .values()
                .any(|state| state.in_flight.load(Ordering::Acquire))
    }

    /// Acquire this guard before any mutation. Refusal is immediate:
    /// a second write cannot proceed until the prior operation reconciles.
    pub fn begin_mutation(&self) -> Result<MutationGuard, AgentError> {
        self.reserve_mutation(false)
    }

    fn reserve_mutation(&self, initialize: bool) -> Result<MutationGuard, AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        let scopes = self.scopes.lock_unpoisoned();
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
        self.mutation_in_flight().store(true, Ordering::Release);
        if self.scope == MutationScope::Root {
            for state in scopes.values() {
                if let Some(token) = state.load.lock_unpoisoned().take() {
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
                if let Some(token) = self.root.load.lock_unpoisoned().take() {
                    token.cancel();
                }
                self.root
                    .load_generation
                    .store(self.next_generation(), Ordering::Release);
            }
        }
        Ok(MutationGuard(Arc::clone(self.mutation_in_flight())))
    }

    /// Refuses `permit` unless it is the reservation this view's scope holds
    /// now.
    pub(super) fn require_reservation(&self, permit: &MutationGuard) -> Result<(), AgentError> {
        if !Arc::ptr_eq(self.mutation_in_flight(), &permit.0)
            || !self.mutation_in_flight().load(Ordering::Acquire)
        {
            return Err(invalid_request(
                "The mutation reservation belongs to another session.",
            ));
        }
        Ok(())
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

#[derive(Debug)]
pub struct MutationGuard(Arc<AtomicBool>);

impl Drop for MutationGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
