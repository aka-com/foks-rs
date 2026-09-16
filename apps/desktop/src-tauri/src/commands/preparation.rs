//! Read-only mutation preparation under the same reservation as the single write.

use crate::agent::AgentError;
use crate::commands::accounts::{load_accounts, AccountDto};
use crate::commands::context::{catalog_changed_during_read, AppState, MutationGuard};
use crate::commands::groups::{
    load_group_details, FederationEntryDto, GroupDetailResultDto, PartyDto,
};
use crate::commands::vault::CatalogDto;
use foks_desktop::AgentTransport;
use std::sync::Arc;

/// A missing catalog is repaired before target/version validation, never by
/// replaying a write that has already reached the agent. A prior ambiguous
/// write still requires an explicit authoritative reconciliation.
pub(super) async fn prepare_catalog_mutation(
    state: &AppState,
) -> Result<MutationGuard, AgentError> {
    let permit = state.begin_mutation()?;
    ensure_catalog_for_mutation(state, &permit, state.agent.transport()).await?;
    Ok(permit)
}

pub(super) async fn ensure_catalog_for_mutation(
    state: &AppState,
    permit: &MutationGuard,
    transport: Arc<dyn AgentTransport>,
) -> Result<(), AgentError> {
    let Some((generation, token)) = state.begin_missing_catalog_for_mutation(permit)? else {
        return Ok(());
    };
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        foks_desktop::load_catalog_cancellable(transport, token).map_err(AgentError::from_desktop)
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!("Failed to prepare the vault catalog: {error}"))
    })??;
    // Apply the same response validation as list_catalog before publication.
    CatalogDto::from_snapshot(&snapshot)?;
    if !state.accept_catalog(generation, snapshot) {
        return Err(catalog_changed_during_read());
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum GroupMutationFacts {
    Members,
    Federation,
}

#[derive(Debug)]
pub(super) struct PreparedGroupFacts {
    pub(super) generation: u64,
    pub(super) store: String,
    pub(super) parties: Option<Vec<PartyDto>>,
    pub(super) federation: Option<Vec<FederationEntryDto>>,
    pub(super) accounts: Option<(String, Vec<AccountDto>)>,
}

pub(super) async fn prepare_group_mutation<T>(
    state: &AppState,
    store_id: &str,
    facts: GroupMutationFacts,
    select: impl FnOnce() -> Result<T, AgentError>,
) -> Result<(MutationGuard, T), AgentError> {
    let permit = prepare_catalog_mutation(state).await?;
    let prepared = prepare_group_facts(state, store_id, facts, state.agent.transport()).await?;
    let target = state.with_prepared_group_facts(&permit, prepared, select)?;
    Ok((permit, target))
}

pub(super) async fn prepare_group_facts(
    state: &AppState,
    store_id: &str,
    facts: GroupMutationFacts,
    transport: Arc<dyn AgentTransport>,
) -> Result<PreparedGroupFacts, AgentError> {
    let team = state.selected_active_team_for_mutation(store_id)?;
    let generation = state
        .catalog_generation
        .load(std::sync::atomic::Ordering::Acquire);
    // Keep these results owned by this preparation until synchronous target
    // validation. Ordinary metadata reads cannot replace them while accounts load.
    let details =
        load_group_details(state, store_id.to_owned(), Arc::clone(&transport), false).await?;
    let mut prepared = PreparedGroupFacts {
        generation,
        store: store_id.to_owned(),
        parties: None,
        federation: None,
        accounts: None,
    };
    match facts {
        GroupMutationFacts::Members => {
            prepared.parties = Some(match details.parties {
                GroupDetailResultDto::Success { value } => value,
                GroupDetailResultDto::Error { error } => return Err(error),
            });
            let (_, catalog) = state.catalog_at(Some(generation))?;
            let mut catalog = catalog.ok_or_else(catalog_changed_during_read)?;
            // Only this profile participates in the member identity check.
            // Read the current native identities, not a cached profile overview.
            catalog.stores.retain(|store| match store {
                foks_desktop::CatalogStoreSummary::Account { store } => {
                    store.profile == team.profile
                }
                foks_desktop::CatalogStoreSummary::Team { .. } => false,
            });
            catalog.profile_overviews.clear();
            let accounts = tauri::async_runtime::spawn_blocking(move || {
                load_accounts(transport.as_ref(), &catalog)
            })
            .await
            .map_err(|error| {
                AgentError::unknown(format!("Failed to prepare account identities: {error}"))
            })??;
            prepared.accounts = Some((team.profile, accounts));
        }
        GroupMutationFacts::Federation => {
            prepared.federation = Some(match details.federation {
                GroupDetailResultDto::Success { value } => value,
                GroupDetailResultDto::Error { error } => return Err(error),
            });
        }
    }
    Ok(prepared)
}

/// The same unlocked interval must authorize preparation and dispatch, even
/// when the application was locked and unlocked before preparation completed.
pub(super) fn check_mutation_access(
    expected: u64,
    current: Result<u64, AgentError>,
) -> Result<(), AgentError> {
    if current? != expected {
        return Err(AgentError::new(
            "app-locked",
            "The application locked before the change was submitted.",
            false,
        ));
    }
    Ok(())
}
