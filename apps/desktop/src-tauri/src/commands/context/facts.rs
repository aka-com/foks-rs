//! Cached authorization facts: accounts, devices, rosters and federations.
//!
//! Each cache holds the facts of the accepted catalog and is keyed by store
//! identity. A read retains its facts only while the catalog generation it
//! began with is current.

use super::{AppState, LockExt, MutationGuard};
use crate::agent::AgentError;
use crate::commands::accounts::{AccountDto, DeviceDto};
use crate::commands::groups::{FederationEntryDto, PartyDto};
use crate::commands::store_key::StoreKey;
use crate::commands::validation::invalid_response;
use std::sync::atomic::Ordering;

impl AppState {
    pub(super) fn clear_profile_facts(&self, profile: &str) {
        self.accounts
            .lock_unpoisoned()
            .retain(|_, account| account.profile != profile);
        let belongs =
            |store: &String| StoreKey::parse(store).is_some_and(|key| key.profile() == profile);
        self.devices
            .lock_unpoisoned()
            .retain(|store, _| !belongs(store));
        self.rosters
            .lock_unpoisoned()
            .retain(|store, _| !belongs(store));
        self.federations
            .lock_unpoisoned()
            .retain(|store, _| !belongs(store));
    }

    pub(super) fn clear_group_facts(&self) {
        self.accounts.lock_unpoisoned().clear();
        self.devices.lock_unpoisoned().clear();
        self.rosters.lock_unpoisoned().clear();
        self.federations.lock_unpoisoned().clear();
    }

    pub(in crate::commands) fn retain_accounts(
        &self,
        generation: u64,
        accounts: &[AccountDto],
    ) -> Result<(), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.catalog_generation().load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        *self.accounts.lock_unpoisoned() = accounts
            .iter()
            .cloned()
            .map(|account| (account.store.clone(), account))
            .collect();
        Ok(())
    }

    /// Install freshly read authorization facts and bind the operation target
    /// under one coordination lock. Earlier metadata reads cannot replace the
    /// prepared facts between their installation and synchronous validation.
    pub(in crate::commands) fn with_prepared_group_facts<T>(
        &self,
        permit: &MutationGuard,
        prepared: crate::commands::preparation::PreparedGroupFacts,
        select: impl FnOnce() -> Result<T, AgentError>,
    ) -> Result<T, AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        self.require_reservation(permit)?;
        if self.catalog_generation().load(Ordering::Acquire) != prepared.generation {
            return Err(catalog_changed_during_group_read());
        }
        if let Some((profile, accounts)) = prepared.accounts {
            if accounts.iter().any(|account| account.profile != profile) {
                return Err(invalid_response(
                    "The account identities belong to another profile.",
                ));
            }
            let mut retained = self.accounts.lock_unpoisoned();
            retained.retain(|_, account| account.profile != profile);
            retained.extend(
                accounts
                    .into_iter()
                    .map(|account| (account.store.clone(), account)),
            );
        }
        if let Some(parties) = prepared.parties {
            self.rosters
                .lock_unpoisoned()
                .insert(prepared.store.clone(), parties);
        }
        if let Some(federation) = prepared.federation {
            self.federations
                .lock_unpoisoned()
                .insert(prepared.store, federation);
        }
        select()
    }

    pub(in crate::commands) fn retain_roster(
        &self,
        generation: u64,
        store: String,
        parties: &[PartyDto],
    ) -> Result<(), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.catalog_generation().load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        self.rosters
            .lock_unpoisoned()
            .insert(store, parties.to_vec());
        Ok(())
    }

    pub(in crate::commands) fn retain_devices(
        &self,
        generation: u64,
        store: String,
        devices: &[DeviceDto],
    ) -> Result<(), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.catalog_generation().load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        self.devices
            .lock_unpoisoned()
            .insert(store, devices.to_vec());
        Ok(())
    }

    pub(in crate::commands) fn retain_federation(
        &self,
        generation: u64,
        store: String,
        entries: &[FederationEntryDto],
    ) -> Result<(), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.catalog_generation().load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        self.federations
            .lock_unpoisoned()
            .insert(store, entries.to_vec());
        Ok(())
    }

    pub(in crate::commands) fn retain_group_details(
        &self,
        generation: u64,
        store: String,
        parties: Option<&[PartyDto]>,
        federation: Option<&[FederationEntryDto]>,
    ) -> Result<(), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.catalog_generation().load(Ordering::Acquire) != generation {
            return Err(catalog_changed_during_group_read());
        }
        if let Some(parties) = parties {
            self.rosters
                .lock_unpoisoned()
                .insert(store.clone(), parties.to_vec());
        }
        if let Some(federation) = federation {
            self.federations
                .lock_unpoisoned()
                .insert(store, federation.to_vec());
        }
        Ok(())
    }
}

pub(super) fn catalog_changed_during_group_read() -> AgentError {
    AgentError::new(
        "catalog-required",
        "The vault changed while group details were loading. Refresh and try again.",
        true,
    )
}
