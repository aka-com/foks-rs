//! Typed Tauri command boundary over `foks-desktop`'s free functions.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use foks_agent_proto::{
    CredentialBackend, FederationRole, KvChunkResult, KvReadResult, KvRole, KvStoreRef,
    KvUploadHeader, Operation, PendingOperationKind, PendingOperationSummary, ProfileProtocol,
    ProfileTrust, SecretString, TeamKind, TeamRole, YubiRetryConfiguration,
};
use foks_desktop::{
    CatalogFailureScope, CatalogInventoryState, CatalogItem, CatalogLoadToken, CatalogSnapshot,
    CatalogStoreRef, CatalogStoreSummary, KvAccountMutation, KvItemRead, KvItemValue,
};
use foks_protocol_metadata::PINNED_PROTOCOL_METADATA_SHA256;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_dialog::DialogExt as _;
use zeroize::Zeroizing;

use crate::agent::{success_value, AgentError, AgentHandle};

pub const MAIN: &str = "main";
const MAXIMUM_TEXT_ITEM_BYTES: usize = 128 * 1024;
const MAXIMUM_CLIPBOARD_TEXT_BYTES: usize = 128 * 1024;
const MAXIMUM_INVITE_BYTES: usize = 4 * 1024;
const MAXIMUM_PASSPHRASE_BYTES: usize = 1024;
const MAXIMUM_RECOVERY_PHRASE_BYTES: usize = 4 * 1024;
const MAXIMUM_FIRST_RUN_ROWS: usize = 4 * 1024;
const MAXIMUM_SYNC_FACTS: usize = 10_000_000;
const ENTITY_ID_HEX_BYTES: usize = 66;
const USER_ID_PREFIX: &str = "01";
const HOST_ID_PREFIX: &str = "02";
const NAMED_TEAM_ID_PREFIX: &str = "03";
const DEVICE_ID_PREFIX: &str = "04";
const YUBI_ID_PREFIX: &str = "08";
const YUBI_ID_HEX_BYTES: usize = 68;
const SUBKEY_ID_PREFIX: &str = "0d";
const BACKUP_ID_PREFIX: &str = "10";
const AD_HOC_TEAM_ID_PREFIX: &str = "14";
pub struct AppState {
    pub agent: Arc<AgentHandle>,
    catalog_load: Mutex<Option<CatalogLoadToken>>,
    catalog_coordination: Mutex<()>,
    catalog_generation: AtomicU64,
    catalog: Mutex<Option<CatalogSnapshot>>,
    mutation_in_flight: Arc<AtomicBool>,
    mutation_requires_refresh: AtomicBool,
    pending_drop_paths: Mutex<HashMap<String, PathBuf>>,
    accounts: Mutex<HashMap<String, AccountDto>>,
    devices: Mutex<HashMap<String, Vec<DeviceDto>>>,
    rosters: Mutex<HashMap<String, Vec<PartyDto>>>,
    federations: Mutex<HashMap<String, Vec<FederationEntryDto>>>,
}

impl AppState {
    pub fn new(agent: Arc<AgentHandle>) -> Self {
        Self {
            agent,
            catalog_load: Mutex::new(None),
            catalog_coordination: Mutex::new(()),
            catalog_generation: AtomicU64::new(0),
            catalog: Mutex::new(None),
            mutation_in_flight: Arc::new(AtomicBool::new(false)),
            mutation_requires_refresh: AtomicBool::new(false),
            pending_drop_paths: Mutex::new(HashMap::new()),
            accounts: Mutex::new(HashMap::new()),
            devices: Mutex::new(HashMap::new()),
            rosters: Mutex::new(HashMap::new()),
            federations: Mutex::new(HashMap::new()),
        }
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

    fn retain_accounts(&self, generation: u64, accounts: &[AccountDto]) -> Result<(), AgentError> {
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

    fn retain_roster(
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

    fn retain_devices(
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

    fn retain_federation(
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

    fn begin_catalog_load(&self) -> (u64, CatalogLoadToken) {
        let token = CatalogLoadToken::default();
        let mut live = self
            .catalog_load
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = live.replace(token.clone()) {
            previous.cancel();
        }
        let generation = self.catalog_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.clear_group_facts();
        (generation, token)
    }

    fn accept_catalog(&self, generation: u64, catalog: CatalogSnapshot) {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.catalog_generation.load(Ordering::Acquire) == generation {
            *self
                .catalog
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(catalog);
            self.mutation_requires_refresh
                .store(false, Ordering::Release);
        }
    }

    fn invalidate_catalog(&self) {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(token) = self
            .catalog_load
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            token.cancel();
        }
        self.catalog_generation.fetch_add(1, Ordering::AcqRel);
        *self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.clear_group_facts();
    }

    fn begin_catalog_load_checked(&self) -> Result<(u64, CatalogLoadToken), AgentError> {
        let _coordination = self
            .catalog_coordination
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.mutation_in_flight.load(Ordering::Acquire) {
            return Err(AgentError::new(
                "mutation-in-flight",
                "Wait for the current change to finish before refreshing the vault.",
                false,
            ));
        }
        Ok(self.begin_catalog_load())
    }

    fn selected_item(
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
                "This server is not currently granting the capabilities needed to read this item.",
                false,
            )),
            Some(item) if item.metadata.version == version => Ok(item.clone()),
            Some(_) => {
                let mut error = AgentError::new(
                    "version-mismatch",
                    "This item changed. Refresh and review it before reading.",
                    false,
                );
                error.fatal = true;
                Err(error)
            }
            None => Err(AgentError::new(
                "item-not-found",
                "This item is no longer in the current catalog.",
                false,
            )),
        }
    }

    fn selected_team(&self, store: &str) -> Result<(String, String), AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before reading group details.",
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
                    "This group is no longer in the current catalog.",
                    false,
                )
            })?;
        require_profile_available(catalog, &selected.0)?;
        Ok(selected)
    }

    fn selected_account(&self, id: &str) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before using this account.",
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
                "This account is no longer in the current catalog.",
                false,
            ));
        };
        require_profile_available(catalog, &account.profile)?;
        Ok(account)
    }

    fn selected_account_dto(&self, id: &str) -> Result<AccountDto, AgentError> {
        let account = self.selected_account(id)?;
        let accounts = self
            .accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let selected = accounts.get(id).ok_or_else(|| {
            AgentError::new(
                "accounts-required",
                "Refresh account identities before using this account.",
                true,
            )
        })?;
        if selected.profile != account.profile || selected.alias != account.account_alias {
            return Err(invalid_response(
                "The retained account identity does not match the current catalog.",
            ));
        }
        Ok(selected.clone())
    }

    fn selected_device_target(
        &self,
        account_store_id: &str,
        device_id: &str,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        let account = self.selected_account(account_store_id)?;
        if !valid_device_member_id_hex(device_id) {
            return Err(invalid_request("Choose a valid device id."));
        }
        let devices = self
            .devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(retained) = devices.get(account_store_id) else {
            return Err(AgentError::new(
                "devices-required",
                "Refresh this account's device list before removing a device.",
                true,
            ));
        };
        let mut matches = retained.iter().filter(|device| device.id == device_id);
        let Some(device) = matches.next() else {
            return Err(AgentError::new(
                "device-not-found",
                "This device is no longer in the current device list.",
                false,
            ));
        };
        if matches.next().is_some() {
            return Err(invalid_response(
                "The retained device list contains a duplicate device id.",
            ));
        }
        if device.current {
            return Err(AgentError::new(
                "current-device",
                "This Mac cannot remove its own current device credential.",
                false,
            ));
        }
        if !valid_typed_entity_id_hex(device_id, DEVICE_ID_PREFIX) {
            return Err(AgentError::new(
                "device-not-removable",
                "This retained device is not a removable software-device identity.",
                false,
            ));
        }
        Ok(account)
    }

    fn ensure_profile_available(&self, profile: &str) -> Result<(), AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        require_profile_available(
            catalog.as_ref().ok_or_else(|| {
                AgentError::new(
                    "catalog-required",
                    "Refresh the vault before making this change.",
                    true,
                )
            })?,
            profile,
        )
    }

    fn selected_active_team_for_mutation(
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
                "Refresh the vault before changing a group.",
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
                "This group is no longer in the current catalog.",
                false,
            ));
        };
        require_profile_available(catalog, &team.profile)?;
        if !active {
            return Err(AgentError::new(
                "inactive-group",
                "This group reports inactive. Resume its creation before changing it.",
                false,
            ));
        }
        if kind != "named" {
            return Err(AgentError::new(
                "group-management-unavailable",
                "Member and federation changes are available only for named groups.",
                false,
            ));
        }
        Ok(team)
    }

    fn selected_remote_named_team(
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
                "Refresh the vault before admitting a group.",
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
                "The group to admit is no longer in the current catalog.",
                false,
            ));
        };
        require_profile_available(catalog, &remote.profile)?;
        if remote.profile == local_profile || kind != "named" || !active {
            return Err(invalid_request(
                "Choose an active named group from a different server.",
            ));
        }
        Ok(remote)
    }

    fn selected_member_target(
        &self,
        selected_store_id: &str,
        username: &str,
    ) -> Result<(foks_agent_proto::TeamStoreRef, String, MemberRole), AgentError> {
        let team = self.selected_active_team_for_mutation(selected_store_id)?;
        let username = required_field(username, "Enter the member username.")?;
        let rosters = self
            .rosters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(roster) = rosters.get(selected_store_id) else {
            return Err(AgentError::new(
                "roster-required",
                "Re-read this group's roster before changing a member.",
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
                "Refresh account identities before changing a group member.",
                true,
            ));
        };
        if account.username == username {
            return Err(member_not_actionable());
        }
        Ok((team, party_id_hex, current))
    }

    fn selected_inactive_admission(
        &self,
        store_id: &str,
        operation_id: &str,
    ) -> Result<(foks_agent_proto::TeamStoreRef, FederationEntryDto), AgentError> {
        let team = self.selected_active_team_for_mutation(store_id)?;
        if !valid_operation_id_hex(operation_id) {
            return Err(invalid_request(
                "Choose a valid journaled admission to re-run.",
            ));
        }
        let federations = self
            .federations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entries) = federations.get(store_id) else {
            return Err(AgentError::new(
                "federation-required",
                "Re-read this group's federation before re-running an admission.",
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

    fn selected_store(&self, id: &str) -> Result<(CatalogStoreRef, Option<bool>), AgentError> {
        let catalog = self
            .catalog
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before changing it.",
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
                    "This vault is no longer in the current catalog.",
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

    fn selected_create_store(&self, id: &str) -> Result<CatalogStoreRef, AgentError> {
        let (store, active) = self.selected_store(id)?;
        match active {
            Some(false) => Err(AgentError::new(
                "inactive-group",
                "This group reports inactive. Resume its creation before changing it.",
                false,
            )),
            Some(true) | None => Ok(store),
        }
    }

    fn selected_mutation_item(
        &self,
        store: &str,
        path: &str,
        version: u64,
    ) -> Result<CatalogItem, AgentError> {
        match self.selected_item(store, path, version) {
            Ok(item) => match self.selected_store(store)?.1 {
                Some(false) => Err(AgentError::new(
                    "inactive-group",
                    "This group reports inactive. Resume its creation before changing it.",
                    false,
                )),
                Some(true) | None => Ok(item),
            },
            Err(error) if error.code == "version-mismatch" || error.code == "item-not-found" => {
                Err(AgentError::new(
                    "conflict",
                    "This item changed or was removed. Refresh and review it before changing it.",
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
        // The product accepts one file at a time. Emit every representable
        // path so it can explain a multi-drop, but authorize nothing unless
        // the native event itself contained exactly one UTF-8 path.
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

    fn take_drop_path(&self, wire_path: &str) -> Result<PathBuf, AgentError> {
        self.pending_drop_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(wire_path)
            .ok_or_else(|| {
                AgentError::new(
                    "drop-not-authorized",
                    "Drop this file onto FOKS again before importing it.",
                    false,
                )
            })
    }

    /// Phase 3 commands acquire this guard before any mutation. Refusal is
    /// immediate: a second write is never queued behind an outcome it has not
    /// reconciled.
    pub fn begin_mutation(&self) -> Result<MutationGuard, AgentError> {
        if self.mutation_requires_refresh.load(Ordering::Acquire) {
            let mut error = AgentError::new(
                "ambiguous",
                "Refresh the vault to reconcile the previous change before making another one.",
                false,
            );
            error.ambiguous = true;
            return Err(error);
        }
        self.mutation_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                AgentError::new(
                    "mutation-in-flight",
                    "Another change is still in progress. Wait for its result before trying again.",
                    false,
                )
            })?;
        Ok(MutationGuard(Arc::clone(&self.mutation_in_flight)))
    }
}

#[derive(Debug)]
pub struct MutationGuard(Arc<AtomicBool>);

impl Drop for MutationGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn require_profile_available(catalog: &CatalogSnapshot, profile: &str) -> Result<(), AgentError> {
    if catalog.profile_blocked(profile) {
        Err(AgentError::new(
            "capability-unavailable",
            "This server is blocked by the current catalog result. Refresh its status before continuing.",
            false,
        ))
    } else {
        Ok(())
    }
}

fn required_field(value: &str, message: &'static str) -> Result<String, AgentError> {
    let value = value.trim();
    if value.is_empty() {
        Err(invalid_request(message))
    } else {
        Ok(value.to_owned())
    }
}

fn valid_response_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value.trim() == value
        && !value.contains(['\0', '\r', '\n'])
}

fn valid_local_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn bounded_local_name(value: &str, message: &'static str) -> Result<String, AgentError> {
    if valid_local_name(value) {
        Ok(value.to_owned())
    } else {
        Err(invalid_request(message))
    }
}

fn valid_entity_id_hex(value: &str) -> bool {
    value.len() == ENTITY_ID_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_typed_entity_id_hex(value: &str, prefix: &str) -> bool {
    valid_entity_id_hex(value) && value.starts_with(prefix)
}

fn valid_device_member_id_hex(value: &str) -> bool {
    valid_typed_entity_id_hex(value, DEVICE_ID_PREFIX)
        || (value.len() == YUBI_ID_HEX_BYTES
            && value.starts_with(YUBI_ID_PREFIX)
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

fn valid_operation_id_hex(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn require_response_row_cap(value: &serde_json::Value, noun: &str) -> Result<(), AgentError> {
    match value.as_array() {
        Some(rows) if rows.len() <= MAXIMUM_FIRST_RUN_ROWS => Ok(()),
        Some(_) => Err(invalid_response(format!(
            "The agent returned too many {noun}."
        ))),
        None => Err(invalid_response(format!(
            "The agent returned a non-list {noun} response."
        ))),
    }
}

fn require_nested_response_row_cap(
    value: &serde_json::Value,
    field: &str,
    noun: &str,
) -> Result<(), AgentError> {
    match value.get(field).and_then(serde_json::Value::as_array) {
        Some(rows) if rows.len() <= MAXIMUM_FIRST_RUN_ROWS => Ok(()),
        Some(_) => Err(invalid_response(format!(
            "The agent returned too many {noun}."
        ))),
        None => Err(invalid_response(format!(
            "The agent returned an invalid {noun} response."
        ))),
    }
}

fn bounded_field(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<String, AgentError> {
    let value = required_field(value, message)?;
    if value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(invalid_request(message))
    } else {
        Ok(value)
    }
}

fn optional_bounded_field(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<String, AgentError> {
    if value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(invalid_request(message))
    } else {
        Ok(value.trim().to_owned())
    }
}

fn confirmed_passphrase(
    passphrase: String,
    confirmation: String,
) -> Result<SecretString, AgentError> {
    let passphrase = Zeroizing::new(passphrase);
    let confirmation = Zeroizing::new(confirmation);
    if passphrase.as_str() != confirmation.as_str() {
        return Err(invalid_request(
            "The passphrase confirmation does not match.",
        ));
    }
    if passphrase.is_empty()
        || passphrase.len() > MAXIMUM_PASSPHRASE_BYTES
        || passphrase.contains(['\0', '\r', '\n'])
    {
        return Err(invalid_request(
            "Use one nonempty passphrase of at most 1,024 bytes.",
        ));
    }
    Ok(SecretString::new(passphrase.as_str()))
}

fn optional_confirmed_passphrase(
    passphrase: Option<String>,
    confirmation: Option<String>,
) -> Result<Option<SecretString>, AgentError> {
    match (passphrase, confirmation) {
        (None, None) => Ok(None),
        (Some(passphrase), Some(confirmation)) => {
            confirmed_passphrase(passphrase, confirmation).map(Some)
        }
        _ => Err(invalid_request(
            "Enter both the passphrase and its confirmation, or leave both empty.",
        )),
    }
}

fn bounded_secret(
    value: String,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<SecretString, AgentError> {
    let value = Zeroizing::new(value);
    if value.is_empty() || value.len() > maximum_bytes || value.contains(['\0', '\r', '\n']) {
        Err(invalid_request(message))
    } else {
        Ok(SecretString::new(value.as_str()))
    }
}

fn yubi_slots(signing_slot: u8, pq_slot: u8) -> Result<(u8, u8), AgentError> {
    let retired = |slot: u8| (0x82..=0x95).contains(&slot);
    if signing_slot == pq_slot || !retired(signing_slot) || !retired(pq_slot) {
        Err(invalid_request(
            "Choose two different retired PIV slots from 0x82 through 0x95.",
        ))
    } else {
        Ok((signing_slot, pq_slot))
    }
}

fn yubi_retry_configuration(
    puk: String,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<YubiRetryConfiguration, AgentError> {
    if pin_attempts == 0 || puk_attempts == 0 {
        return Err(invalid_request(
            "PIN and unlock-code retry counts must be positive.",
        ));
    }
    Ok(YubiRetryConfiguration {
        puk: bounded_secret(
            puk,
            128,
            "Use one nonempty unlock code of at most 128 bytes.",
        )?,
        pin_attempts,
        puk_attempts,
    })
}

fn pairing_phrase(value: String) -> Result<SecretString, AgentError> {
    let value = Zeroizing::new(value);
    if value.len() > MAXIMUM_RECOVERY_PHRASE_BYTES
        || foks_crypto::KexSecret::from_phrase(value.as_str()).is_err()
    {
        return Err(invalid_request(
            "Enter the complete valid device-pairing phrase.",
        ));
    }
    Ok(SecretString::new(value.as_str()))
}

fn exact_profile_confirmation(
    profile: &str,
    confirmation: &str,
    action: &str,
) -> Result<(), AgentError> {
    if confirmation == profile {
        Ok(())
    } else {
        Err(invalid_request(format!(
            "Type the exact server profile name to {action} it."
        )))
    }
}

fn positive_recovery_serial_with(
    mut fill: impl FnMut(&mut [u8]) -> Result<(), ()>,
) -> Result<u64, AgentError> {
    for _ in 0..4 {
        let mut bytes = [0u8; 8];
        fill(&mut bytes).map_err(|()| {
            AgentError::new(
                "randomness-unavailable",
                "The operating system could not create a device serial.",
                true,
            )
        })?;
        let serial = u64::from_le_bytes(bytes);
        if serial != 0 {
            return Ok(serial);
        }
    }
    Err(AgentError::new(
        "randomness-unavailable",
        "The operating system did not produce a valid device serial.",
        true,
    ))
}

fn positive_recovery_serial() -> Result<u64, AgentError> {
    positive_recovery_serial_with(|bytes| getrandom::fill(bytes).map_err(|_| ()))
}

fn member_not_actionable() -> AgentError {
    AgentError::new(
        "member-not-actionable",
        "This roster entry cannot be changed from this account.",
        false,
    )
}

fn member_role_from_dto(role: &RoleDto) -> Result<MemberRole, AgentError> {
    match (role.role, role.visibility) {
        ("Member", Some(visibility)) => Ok(MemberRole::Member { visibility }),
        ("Admin", None) => Ok(MemberRole::Admin),
        ("Owner", None) => Ok(MemberRole::Owner),
        _ => Err(invalid_response("The retained group role is invalid.")),
    }
}

fn admission_not_resumable() -> AgentError {
    AgentError::new(
        "admission-not-resumable",
        "This inactive admission does not have a unique resumable operation.",
        false,
    )
}

fn catalog_changed_during_group_read() -> AgentError {
    AgentError::new(
        "catalog-required",
        "The vault changed while these group facts were loading. Refresh and try again.",
        true,
    )
}

pub fn require_main_window(webview: &tauri::Webview) -> Result<(), AgentError> {
    if main_window_allowed(webview.label()) {
        Ok(())
    } else {
        Err(AgentError::new(
            "window-not-allowed",
            "This request did not come from the FOKS window.",
            false,
        ))
    }
}

fn main_window_allowed(label: &str) -> bool {
    label == MAIN
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusDto {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

impl From<foks_agent_proto::AgentStatus> for AgentStatusDto {
    fn from(status: foks_agent_proto::AgentStatus) -> Self {
        match status {
            foks_agent_proto::AgentStatus::Ready => Self {
                state: "ready".to_owned(),
                step: None,
            },
            foks_agent_proto::AgentStatus::Bootstrap { step } => Self {
                state: "bootstrap".to_owned(),
                step: Some(step),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedProfileDto {
    pub profile: String,
    pub acceptance: String,
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id: String,
    pub chain: u64,
    pub epoch: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckedProfileResponse {
    profile: CheckedProfileIdentity,
    probe: CheckedProbeResponse,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckedProfileIdentity {
    name: String,
    probe: String,
    protocol: serde_json::Value,
    trust: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckedProbeResponse {
    acceptance: String,
    lookup_name: String,
    canonical_name: String,
    host_id_hex: String,
    host_chain_sequence: u64,
    merkle_epoch: u64,
}

impl CheckedProfileDto {
    fn from_response(
        expected_profile: &str,
        expected_probe: &str,
        response: CheckedProfileResponse,
    ) -> Result<Self, AgentError> {
        let probe = response.probe;
        if response.profile.name != expected_profile
            || response.profile.probe != expected_probe
            || response.profile.protocol != serde_json::json!({"generation": "v019"})
            || response.profile.trust != serde_json::json!({"kind": "web-pki"})
            || probe.acceptance != "inserted"
            || normalized_probe_hostname(expected_probe).as_deref()
                != Some(probe.lookup_name.as_str())
            || !valid_response_text(&probe.canonical_name, 256)
            || !valid_typed_entity_id_hex(&probe.host_id_hex, HOST_ID_PREFIX)
        {
            return Err(invalid_response(
                "The agent returned an invalid checked-server report.",
            ));
        }
        Ok(Self {
            profile: response.profile.name,
            acceptance: probe.acceptance,
            lookup_name: probe.lookup_name,
            canonical_name: probe.canonical_name,
            host_id: probe.host_id_hex,
            chain: probe.host_chain_sequence,
            epoch: probe.merkle_epoch,
        })
    }

    fn from_existing_probe(
        requested_probe: &str,
        profile: &ProfileSummary,
        value: serde_json::Value,
    ) -> Result<Self, AgentError> {
        let checked = checked_server_response(value, &profile.name)?;
        if normalized_probe_endpoint(&profile.probe) != normalized_probe_endpoint(requested_probe)
            || normalized_probe_hostname(requested_probe).as_deref()
                != Some(checked.lookup_name.as_str())
        {
            return Err(invalid_response(
                "The agent returned a checked-server report for a different address.",
            ));
        }
        Ok(Self {
            profile: checked.profile,
            acceptance: checked.acceptance,
            lookup_name: checked.lookup_name,
            canonical_name: checked.canonical_name,
            host_id: checked.host_id,
            chain: checked.chain,
            epoch: checked.epoch,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOperationDto {
    pub kind: &'static str,
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl TryFrom<PendingOperationSummary> for PendingOperationDto {
    type Error = AgentError;

    fn try_from(summary: PendingOperationSummary) -> Result<Self, Self::Error> {
        if !valid_local_name(&summary.alias)
            || summary
                .target
                .as_ref()
                .is_some_and(|target| !valid_response_text(target, 256))
        {
            return Err(invalid_response(
                "The agent returned an invalid pending-operation identity.",
            ));
        }
        let kind = match summary.kind {
            PendingOperationKind::AccountSignup => "account-signup",
            PendingOperationKind::DeviceProvision => "device-provision",
            PendingOperationKind::PairingOffer => "pairing-offer",
            PendingOperationKind::PairingAcceptance => "pairing-acceptance",
            PendingOperationKind::AccountRecovery => "account-recovery",
            PendingOperationKind::YubiEnrollment => "yubi-enrollment",
            PendingOperationKind::TeamCreation => "team-creation",
            PendingOperationKind::TeamMemberAddition => "team-member-addition",
            PendingOperationKind::TeamMemberEdit => "team-member-edit",
            PendingOperationKind::TeamRekey => "team-rekey",
        };
        Ok(Self {
            kind,
            alias: summary.alias,
            target: summary.target,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPhraseDto {
    pub backup_alias: String,
    #[serde(serialize_with = "serialize_secret")]
    pub phrase: Zeroizing<String>,
}

impl std::fmt::Debug for BackupPhraseDto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BackupPhraseDto")
            .field("backup_alias", &self.backup_alias)
            .field("phrase", &"[REDACTED]")
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupPhraseResponse {
    backup_alias: String,
    #[serde(deserialize_with = "deserialize_secret")]
    phrase: Zeroizing<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountSyncResponse {
    username: String,
    user_chain_sequence: u64,
    directories: usize,
    entries: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassphraseResponse {
    generation: u64,
    stretch_version: String,
    verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupCommitResponse {
    backup_alias: String,
    account_alias: String,
    backup_id_hex: String,
    user_chain_sequence: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryResponse {
    alias: String,
    device_id_hex: String,
    user_chain_sequence: u64,
}

fn backup_phrase_response(
    value: serde_json::Value,
    expected_backup: &str,
) -> Result<BackupPhraseDto, AgentError> {
    let response: BackupPhraseResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.backup_alias != expected_backup
        || response.phrase.is_empty()
        || response.phrase.len() > MAXIMUM_RECOVERY_PHRASE_BYTES
        || response.phrase.contains(['\0', '\r', '\n'])
        || foks_crypto::BackupKey::from_phrase(response.phrase.as_str()).is_err()
    {
        return Err(invalid_response(
            "The agent returned an invalid owner-backup phrase.",
        ));
    }
    Ok(BackupPhraseDto {
        backup_alias: response.backup_alias,
        phrase: response.phrase,
    })
}

fn account_sync_response(value: serde_json::Value) -> Result<MutationDto, AgentError> {
    let report: AccountSyncResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    // FOKS normalizes the account lookup name before signing it, so the
    // authenticated display username is not required to byte-match the user's
    // input. It must still be a bounded single-line fact.
    if !valid_response_text(&report.username, 256)
        || report.directories > MAXIMUM_SYNC_FACTS
        || report.entries > MAXIMUM_SYNC_FACTS
        || (report.directories == 0 && report.entries != 0)
    {
        return Err(invalid_response(
            "The agent returned an invalid account synchronization report.",
        ));
    }
    let _sequence = report.user_chain_sequence;
    Ok(MutationDto { applied: true })
}

fn passphrase_response(value: serde_json::Value) -> Result<MutationDto, AgentError> {
    passphrase_report_response(value).map(|_| MutationDto { applied: true })
}

fn passphrase_report_response(value: serde_json::Value) -> Result<PassphraseReportDto, AgentError> {
    let report: PassphraseResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    // First-run publishes only v0.1.9 profiles. The client rejects the
    // test-only stretch before returning this production report.
    if report.generation == 0 || report.stretch_version != "v1" || !report.verified {
        return Err(invalid_response(
            "The agent returned an invalid passphrase-verification report.",
        ));
    }
    Ok(PassphraseReportDto {
        generation: report.generation,
        stretch_version: report.stretch_version,
        verified: report.verified,
    })
}

fn checked_server_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<CheckedServerDto, AgentError> {
    let report: CheckedProbeResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if !matches!(
        report.acceptance.as_str(),
        "inserted" | "advanced" | "unchanged"
    ) || !valid_response_text(&report.lookup_name, 256)
        || !valid_response_text(&report.canonical_name, 256)
        || !valid_typed_entity_id_hex(&report.host_id_hex, HOST_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid server-check report.",
        ));
    }
    Ok(CheckedServerDto {
        profile: expected_profile.to_owned(),
        acceptance: report.acceptance,
        lookup_name: report.lookup_name,
        canonical_name: report.canonical_name,
        host_id: report.host_id_hex,
        chain: report.host_chain_sequence,
        epoch: report.merkle_epoch,
    })
}

fn added_server_response(
    value: serde_json::Value,
    expected_profile: &str,
    expected_probe: &str,
) -> Result<AddedServerDto, AgentError> {
    let profile: ProfileSummary = serde_json::from_value(value.clone())
        .map_err(|error| invalid_response(error.to_string()))?;
    let canonical =
        serde_json::to_value(&profile).map_err(|error| invalid_response(error.to_string()))?;
    if canonical != value
        || profile.name != expected_profile
        || profile.probe != expected_probe
        || !valid_profile_summary(&profile)
        || profile.protocol != ProfileProtocolSummary::V019
        || profile.trust != ProfileTrustSummary::WebPki
    {
        return Err(invalid_response(
            "The agent returned an invalid added-server record.",
        ));
    }
    Ok(AddedServerDto {
        profile: profile.name,
        configured_probe: profile.probe,
    })
}

fn forgotten_server_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<ForgottenServerDto, AgentError> {
    let response: RemovedProfileResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.profile != expected_profile || !response.removed {
        return Err(invalid_response(
            "The agent did not confirm removal of the selected server profile.",
        ));
    }
    Ok(ForgottenServerDto {
        profile: response.profile,
        removed: response.removed,
    })
}

fn server_status_response(
    value: serde_json::Value,
    expected_profile: &str,
    expected_probe: &str,
    expected_lease_required: bool,
) -> Result<ServerStatusSnapshotDto, AgentError> {
    let report: ServerStatusResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if report.profile != expected_profile
        || report.configured_probe != expected_probe
        || report.lease_required != expected_lease_required
        || (!report.lease_required && report.lease_expires_at.is_some())
        || !valid_probe_target(&report.configured_probe)
    {
        return Err(invalid_response(
            "The agent returned an invalid server-status identity.",
        ));
    }
    let expected_lookup = normalized_probe_hostname(expected_probe)
        .ok_or_else(|| invalid_response("The retained server probe is invalid."))?;
    let host = report
        .host
        .map(|host| {
            if host.lookup_name != expected_lookup
                || !valid_response_text(&host.canonical_name, 256)
                || !valid_typed_entity_id_hex(&host.host_id_hex, HOST_ID_PREFIX)
            {
                return Err(invalid_response(
                    "The agent returned invalid retained host facts.",
                ));
            }
            Ok(StoredHostDto {
                lookup_name: host.lookup_name,
                canonical_name: host.canonical_name,
                host_id: host.host_id_hex,
                chain: host.host_chain_sequence,
                epoch: host.merkle_epoch,
            })
        })
        .transpose()?;
    Ok(ServerStatusSnapshotDto {
        profile: report.profile,
        configured_probe: report.configured_probe,
        host,
        lease_required: report.lease_required,
        lease_expires_at: report.lease_expires_at,
    })
}

fn device_dtos(value: serde_json::Value) -> Result<Vec<DeviceDto>, AgentError> {
    require_response_row_cap(&value, "devices")?;
    let rows: Vec<DeviceResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut ids = std::collections::HashSet::new();
    let mut current_count = 0usize;
    let devices = rows
        .into_iter()
        .map(|row| {
            if !valid_device_member_id_hex(&row.id_hex)
                || !ids.insert(row.id_hex.clone())
                || row
                    .name
                    .as_ref()
                    .is_some_and(|name| !valid_response_text(name, 256))
            {
                return Err(invalid_response(
                    "The agent returned an invalid or duplicate device.",
                ));
            }
            let role = match row.role.as_str() {
                "owner" => "owner",
                "admin" => "admin",
                "member" => "member",
                _ => {
                    return Err(invalid_response(
                        "The agent returned an unknown device role.",
                    ))
                }
            };
            current_count += usize::from(row.current);
            Ok(DeviceDto {
                id: row.id_hex,
                name: row.name,
                role,
                current: row.current,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if current_count != 1 {
        return Err(invalid_response(
            "The agent did not return exactly one current device.",
        ));
    }
    Ok(devices)
}

fn backup_enrollment_dtos(
    value: serde_json::Value,
    expected_account: &str,
) -> Result<Vec<BackupEnrollmentDto>, AgentError> {
    require_response_row_cap(&value, "backup enrollments")?;
    let rows: Vec<BackupEnrollmentResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut aliases = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    rows.into_iter()
        .map(|row| {
            if row.account_alias != expected_account
                || !valid_local_name(&row.backup_alias)
                || !valid_local_name(&row.account_alias)
                || !valid_typed_entity_id_hex(&row.backup_id_hex, BACKUP_ID_PREFIX)
                || !aliases.insert(row.backup_alias.clone())
                || !ids.insert(row.backup_id_hex.clone())
            {
                return Err(invalid_response(
                    "The agent returned an invalid or duplicate backup enrollment.",
                ));
            }
            Ok(BackupEnrollmentDto {
                backup_alias: row.backup_alias,
                account_alias: row.account_alias,
                backup_id: row.backup_id_hex,
            })
        })
        .collect()
}

fn yubi_card_dtos(value: serde_json::Value) -> Result<Vec<YubiCardDto>, AgentError> {
    require_response_row_cap(&value, "connected security keys")?;
    let rows: Vec<YubiCardResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut serials = std::collections::HashSet::new();
    rows.into_iter()
        .map(|row| {
            if row.serial == 0
                || !valid_response_text(&row.name, 256)
                || !serials.insert(row.serial)
            {
                return Err(invalid_response(
                    "The agent returned an invalid or duplicate connected security key.",
                ));
            }
            Ok(YubiCardDto { serial: row.serial })
        })
        .collect()
}

fn yubi_enrollment_dtos(value: serde_json::Value) -> Result<Vec<YubiEnrollmentDto>, AgentError> {
    require_response_row_cap(&value, "security-key enrollments")?;
    let rows: Vec<YubiEnrollmentResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    let mut enrollments = std::collections::HashSet::new();
    rows.into_iter()
        .map(|row| {
            let state = match row.state.as_str() {
                "pending" => "pending",
                "complete" => "complete",
                _ => {
                    return Err(invalid_response(
                        "The agent returned an unknown security-key enrollment state.",
                    ))
                }
            };
            if !valid_local_name(&row.alias) || !enrollments.insert((row.alias.clone(), state)) {
                return Err(invalid_response(
                    "The agent returned an invalid or duplicate security-key enrollment.",
                ));
            }
            Ok(YubiEnrollmentDto {
                alias: row.alias,
                state,
            })
        })
        .collect()
}

fn require_yubi_enrollment(
    enrollments: &[YubiEnrollmentDto],
    alias: &str,
    expected_state: &str,
) -> Result<(), AgentError> {
    let has_pending = enrollments
        .iter()
        .any(|entry| entry.alias == alias && entry.state == "pending");
    let has_complete = enrollments
        .iter()
        .any(|entry| entry.alias == alias && entry.state == "complete");
    if !has_pending && !has_complete {
        return Err(AgentError::new(
            "security-key-not-found",
            "Refresh Security keys before using this enrollment.",
            false,
        ));
    }
    match expected_state {
        "pending" if has_pending => Ok(()),
        "pending" => Err(AgentError::new(
            "security-key-state-changed",
            "Refresh Security keys: this setup is already complete.",
            false,
        )),
        "complete" if has_complete && !has_pending => Ok(()),
        "complete" => Err(AgentError::new(
            "security-key-state-changed",
            "Resume the pending security-key setup, then refresh Security keys before continuing.",
            false,
        )),
        _ => Err(invalid_response(
            "The desktop requested an unsupported security-key enrollment state.",
        )),
    }
}

fn require_software_revocation_signer(devices: &[DeviceDto]) -> Result<(), AgentError> {
    match devices.iter().find(|device| device.current) {
        Some(device) if device.id.starts_with(DEVICE_ID_PREFIX) => Ok(()),
        Some(_) => Err(AgentError::new(
            "security-key-current",
            "A security key cannot sign its own revocation. Choose a software account on this Mac.",
            false,
        )),
        None => Err(invalid_response(
            "The agent did not identify the current signing device.",
        )),
    }
}

fn yubi_account_response(
    value: serde_json::Value,
    expected_alias: &str,
    expected_username: Option<&str>,
) -> Result<YubiAccountDto, AgentError> {
    let response: YubiAccountResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !valid_response_text(&response.username, 256)
        || expected_username.is_some_and(|username| response.username != username)
        || !valid_typed_yubi_id_hex(&response.yubi_id_hex)
        || !valid_typed_entity_id_hex(&response.subkey_id_hex, SUBKEY_ID_PREFIX)
        || !response.management_enrolled
    {
        return Err(invalid_response(
            "The agent returned an invalid security-key account report.",
        ));
    }
    Ok(YubiAccountDto {
        alias: response.alias,
        username: response.username,
        yubi_id: response.yubi_id_hex,
        subkey_id: response.subkey_id_hex,
        user_chain_sequence: response.user_chain_sequence,
        management_enrolled: response.management_enrolled,
    })
}

fn valid_typed_yubi_id_hex(value: &str) -> bool {
    value.len() == YUBI_ID_HEX_BYTES
        && value.starts_with(YUBI_ID_PREFIX)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_sync_report(report: &AccountSyncResponse) -> bool {
    valid_response_text(&report.username, 256)
        && report.directories <= MAXIMUM_SYNC_FACTS
        && report.entries <= MAXIMUM_SYNC_FACTS
        && (report.directories != 0 || report.entries == 0)
}

fn yubi_sync_response(
    value: serde_json::Value,
    expected_profile: &str,
    with_federation: bool,
) -> Result<YubiSyncDto, AgentError> {
    let (sync, federation) = if with_federation {
        require_nested_response_row_cap(&value, "federation", "federation refreshes")?;
        let response: YubiFederationSyncResponse =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        let mut teams = std::collections::HashSet::new();
        let federation = response
            .federation
            .into_iter()
            .map(|entry| {
                let deferred_valid = entry
                    .deferred
                    .as_deref()
                    .is_none_or(|reason| valid_response_text(reason, 512));
                if entry.local_profile != expected_profile
                    || !valid_local_name(&entry.local_team_alias)
                    || !teams.insert(entry.local_team_alias.clone())
                    || !deferred_valid
                    || entry.refreshed == entry.deferred.is_some()
                {
                    return Err(invalid_response(
                        "The agent returned an invalid federation refresh report.",
                    ));
                }
                Ok(YubiFederationRefreshDto {
                    local_profile: entry.local_profile,
                    local_team_alias: entry.local_team_alias,
                    refreshed: entry.refreshed,
                    deferred: entry.deferred,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        (response.sync, federation)
    } else {
        let response: AccountSyncResponse =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        (response, Vec::new())
    };
    if !valid_sync_report(&sync) {
        return Err(invalid_response(
            "The agent returned an invalid security-key synchronization report.",
        ));
    }
    Ok(YubiSyncDto {
        username: sync.username,
        user_chain_sequence: sync.user_chain_sequence,
        directories: sync.directories,
        entries: sync.entries,
        federation,
    })
}

fn yubi_pin_status_response(value: serde_json::Value) -> Result<YubiPinStatusDto, AgentError> {
    let response: YubiPinStatusResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.blocked != (response.remaining == 0) {
        return Err(invalid_response(
            "The agent returned inconsistent security-key PIN retry facts.",
        ));
    }
    Ok(YubiPinStatusDto {
        remaining: response.remaining,
        blocked: response.blocked,
    })
}

fn yubi_lifecycle_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiLifecycleDto, AgentError> {
    let response: YubiLifecycleResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !response.management_enrolled
        || response
            .management_generation
            .is_none_or(|generation| generation == 0)
    {
        return Err(invalid_response(
            "The agent returned an invalid security-key management report.",
        ));
    }
    Ok(YubiLifecycleDto {
        alias: response.alias,
        management_enrolled: response.management_enrolled,
        management_generation: response.management_generation,
    })
}

fn yubi_subkey_recovery_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiSubkeyRecoveryDto, AgentError> {
    let response: YubiSubkeyRecoveryResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !valid_typed_entity_id_hex(&response.subkey_id_hex, SUBKEY_ID_PREFIX)
        || response.certificate_count > MAXIMUM_FIRST_RUN_ROWS
    {
        return Err(invalid_response(
            "The agent returned an invalid security-key subkey recovery report.",
        ));
    }
    Ok(YubiSubkeyRecoveryDto {
        alias: response.alias,
        subkey_id: response.subkey_id_hex,
        certificate_count: response.certificate_count,
    })
}

fn yubi_revocation_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiRevocationDto, AgentError> {
    let response: YubiRevocationResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias || !response.removed_local_credential {
        return Err(invalid_response(
            "The agent did not confirm removal of the selected security-key credential.",
        ));
    }
    Ok(YubiRevocationDto {
        alias: response.alias,
        user_chain_sequence: response.user_chain_sequence,
        removed_local_credential: response.removed_local_credential,
    })
}

fn yubi_changed_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<YubiChangedDto, AgentError> {
    let response: YubiChangedResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias || !response.changed {
        return Err(invalid_response(
            "The agent did not confirm the selected security-key change.",
        ));
    }
    Ok(YubiChangedDto {
        alias: response.alias,
        changed: response.changed,
    })
}

fn pairing_offer_response(
    value: serde_json::Value,
    expected_account: &str,
) -> Result<PairingOfferDto, AgentError> {
    let response: PairingOfferResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.account_alias != expected_account
        || response.phrase.len() > MAXIMUM_RECOVERY_PHRASE_BYTES
        || foks_crypto::KexSecret::from_phrase(response.phrase.as_str()).is_err()
    {
        return Err(invalid_response(
            "The agent returned an invalid device-pairing phrase.",
        ));
    }
    Ok(PairingOfferDto {
        account_alias: response.account_alias,
        phrase: response.phrase,
    })
}

fn device_provision_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<DeviceProvisionDto, AgentError> {
    let response: DeviceProvisionResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.alias != expected_alias
        || !valid_typed_entity_id_hex(&response.device_id_hex, DEVICE_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid paired-device report.",
        ));
    }
    Ok(DeviceProvisionDto {
        alias: response.alias,
        device_id: response.device_id_hex,
        user_chain_sequence: response.user_chain_sequence,
    })
}

fn device_removal_response(
    value: serde_json::Value,
    expected_device: &str,
) -> Result<DeviceRemovalDto, AgentError> {
    let response: DeviceRemovalResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.device_id_hex != expected_device {
        return Err(invalid_response(
            "The agent returned a different removed-device identity.",
        ));
    }
    Ok(DeviceRemovalDto {
        device_id: response.device_id_hex,
        user_chain_sequence: response.user_chain_sequence,
        already_absent: response.already_absent,
    })
}

fn reset_preview_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<ResetPreviewDto, AgentError> {
    require_nested_response_row_cap(&value, "resumables", "reset resumables")?;
    require_nested_response_row_cap(&value, "artifacts", "reset artifacts")?;
    let response: ResetPreviewResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.profile != expected_profile
        || response.artifacts.len() > 16
        || response.token.is_empty()
        || response.token.len() > 1024
        || response.token.contains(['\0', '\r', '\n'])
        || response.expires_in_seconds == 0
        || response.expires_in_seconds > 24 * 60 * 60
    {
        return Err(invalid_response(
            "The agent returned an invalid reset preview.",
        ));
    }
    let pending = response
        .resumables
        .into_iter()
        .map(Into::into)
        .collect::<Vec<PendingOperationSummary>>();
    let resumables = validated_pending_dtos(&pending)?;
    let mut artifacts: Vec<ResetArtifactDto> = Vec::new();
    for artifact in response.artifacts {
        let kind = match artifact.kind {
            foks_agent_proto::ResetArtifactKind::HardState => "hard-state",
            foks_agent_proto::ResetArtifactKind::SoftState => "soft-state",
            foks_agent_proto::ResetArtifactKind::ProtectedMutations => "protected-mutations",
            foks_agent_proto::ResetArtifactKind::CredentialsAndResumables => {
                "credentials-and-resumables"
            }
            foks_agent_proto::ResetArtifactKind::ExternalRollbackCheckpoint => {
                "external-rollback-checkpoint"
            }
            foks_agent_proto::ResetArtifactKind::ExternalDatabaseClaim => "external-database-claim",
            foks_agent_proto::ResetArtifactKind::ExternalPublicationAuthorization => {
                "external-publication-authorization"
            }
        };
        if let Some(aggregate) = artifacts.iter_mut().find(|entry| entry.kind == kind) {
            aggregate.entries =
                aggregate
                    .entries
                    .checked_add(artifact.entries)
                    .ok_or_else(|| {
                        invalid_response("The agent returned overflowing reset artifact totals.")
                    })?;
            aggregate.bytes = aggregate.bytes.checked_add(artifact.bytes).ok_or_else(|| {
                invalid_response("The agent returned overflowing reset artifact totals.")
            })?;
        } else {
            artifacts.push(ResetArtifactDto {
                kind,
                entries: artifact.entries,
                bytes: artifact.bytes,
            });
        }
    }
    Ok(ResetPreviewDto {
        profile: response.profile,
        resumables,
        artifacts,
        token: response.token,
        expires_in_seconds: response.expires_in_seconds,
    })
}

fn reset_result_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<MutationDto, AgentError> {
    let response: ResetResultResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.profile != expected_profile || !response.hard_state_reset {
        return Err(invalid_response(
            "The agent did not bind reset completion to the selected server.",
        ));
    }
    Ok(MutationDto { applied: true })
}

fn backup_commit_response(
    value: serde_json::Value,
    expected_account: &str,
    expected_backup: &str,
) -> Result<MutationDto, AgentError> {
    let report: BackupCommitResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if report.account_alias != expected_account
        || report.backup_alias != expected_backup
        || !valid_typed_entity_id_hex(&report.backup_id_hex, BACKUP_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid owner-backup completion report.",
        ));
    }
    let _sequence = report.user_chain_sequence;
    Ok(MutationDto { applied: true })
}

fn recovery_response(
    value: serde_json::Value,
    expected_alias: &str,
) -> Result<MutationDto, AgentError> {
    let report: RecoveryResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if report.alias != expected_alias
        || !valid_typed_entity_id_hex(&report.device_id_hex, DEVICE_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid owner-recovery completion report.",
        ));
    }
    let _sequence = report.user_chain_sequence;
    Ok(MutationDto { applied: true })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredGroupDto {
    pub alias: String,
    pub account_alias: String,
    pub team_id_hex: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDiscoveryDto {
    pub account_alias: String,
    pub groups: Vec<DiscoveredGroupDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupDiscoveryResponse {
    account_alias: String,
    teams: Vec<DiscoveredGroupResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoveredGroupResponse {
    alias: String,
    account_alias: String,
    team_id_hex: String,
    kind: String,
    name: Option<String>,
    active: bool,
}

impl GroupDiscoveryDto {
    fn from_response(
        expected_account: &str,
        response: GroupDiscoveryResponse,
    ) -> Result<Self, AgentError> {
        if response.account_alias != expected_account
            || !valid_local_name(&response.account_alias)
            || response.teams.len() > MAXIMUM_FIRST_RUN_ROWS
        {
            return Err(invalid_response(
                "The agent returned discovery results for a different account.",
            ));
        }
        let mut aliases = std::collections::HashSet::new();
        let mut team_ids = std::collections::HashSet::new();
        let groups = response
            .teams
            .into_iter()
            .map(|team| {
                let kind = match team.kind.as_str() {
                    "named" => "named",
                    "ad-hoc" => "adhoc",
                    _ => {
                        return Err(invalid_response(
                            "The agent returned an unsupported discovered group kind.",
                        ));
                    }
                };
                let team_id_is_valid = match kind {
                    "named" => valid_typed_entity_id_hex(&team.team_id_hex, NAMED_TEAM_ID_PREFIX),
                    "adhoc" => valid_typed_entity_id_hex(&team.team_id_hex, AD_HOC_TEAM_ID_PREFIX),
                    _ => unreachable!("discovered group kinds are normalized above"),
                };
                if team.account_alias != expected_account
                    || !valid_local_name(&team.alias)
                    || !team_id_is_valid
                    || team
                        .name
                        .as_ref()
                        .is_some_and(|name| !valid_response_text(name, 512))
                    || (kind == "named" && team.name.is_none())
                    || (kind == "adhoc" && team.name.is_some())
                    || !aliases.insert(team.alias.clone())
                    || !team_ids.insert(team.team_id_hex.clone())
                {
                    return Err(invalid_response(
                        "The agent returned an invalid discovered group.",
                    ));
                }
                Ok(DiscoveredGroupDto {
                    alias: team.alias,
                    account_alias: team.account_alias,
                    team_id_hex: team.team_id_hex,
                    kind,
                    name: team.name,
                    active: team.active,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            account_alias: response.account_alias,
            groups,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub agent_socket: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_profile: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDto {
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<i16>,
}

impl From<KvRole> for RoleDto {
    fn from(role: KvRole) -> Self {
        match role {
            KvRole::Member { visibility } => Self {
                role: "Member",
                visibility: Some(visibility),
            },
            KvRole::Admin => Self {
                role: "Admin",
                visibility: None,
            },
            KvRole::Owner => Self {
                role: "Owner",
                visibility: None,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoreDto {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub server: String,
    pub account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id_hex: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ItemDto {
    pub store: String,
    pub path: String,
    pub kind: &'static str,
    pub size: u64,
    pub version: u64,
    pub read: RoleDto,
    pub write: RoleDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFailureDto {
    pub scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    pub error: AgentError,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogInventoryDto {
    pub profile: String,
    pub accounts_complete: bool,
    pub teams_complete: bool,
}

impl From<&CatalogInventoryState> for CatalogInventoryDto {
    fn from(state: &CatalogInventoryState) -> Self {
        Self {
            profile: state.profile.clone(),
            accounts_complete: state.accounts_complete,
            teams_complete: state.teams_complete,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDto {
    pub profiles: Vec<String>,
    pub stores: Vec<StoreDto>,
    pub known_stores: Vec<StoreDto>,
    pub inventory: Vec<CatalogInventoryDto>,
    pub items: Vec<ItemDto>,
    pub failures: Vec<CatalogFailureDto>,
    pub blocked_profiles: Vec<String>,
}

impl CatalogDto {
    fn from_snapshot(snapshot: &CatalogSnapshot) -> Result<Self, AgentError> {
        Ok(Self {
            profiles: snapshot.profiles.clone(),
            stores: snapshot
                .stores
                .iter()
                .map(store_dto)
                .collect::<Result<Vec<_>, _>>()?,
            known_stores: snapshot
                .known_stores
                .iter()
                .map(store_dto)
                .collect::<Result<Vec<_>, _>>()?,
            inventory: snapshot
                .inventory
                .iter()
                .map(CatalogInventoryDto::from)
                .collect(),
            items: snapshot
                .items
                .iter()
                .map(item_dto)
                .collect::<Result<Vec<_>, _>>()?,
            failures: snapshot
                .failures
                .iter()
                .map(|failure| {
                    let (scope, profile, source, store) = match &failure.scope {
                        CatalogFailureScope::Profile { profile, source } => {
                            ("profile", Some(profile.clone()), Some(source.clone()), None)
                        }
                        CatalogFailureScope::Store(store) => (
                            "store",
                            Some(store.profile().to_owned()),
                            None,
                            Some(store_id(store)),
                        ),
                    };
                    CatalogFailureDto {
                        scope,
                        profile,
                        source,
                        store,
                        error: AgentError::from_desktop(failure.error.clone()),
                    }
                })
                .collect(),
            blocked_profiles: snapshot.blocked_profiles.clone(),
        })
    }
}

fn store_id(store: &CatalogStoreRef) -> String {
    match store {
        CatalogStoreRef::Account(store) => serde_json::json!({
            "kind": "account",
            "profile": store.profile,
            "accountAlias": store.account_alias,
        }),
        CatalogStoreRef::Team(store) => serde_json::json!({
            "kind": "team",
            "profile": store.profile,
            "accountAlias": store.account_alias,
            "teamAlias": store.team_alias,
            "teamId": store.team_id,
        }),
    }
    .to_string()
}

fn store_dto(store: &CatalogStoreSummary) -> Result<StoreDto, AgentError> {
    Ok(match store {
        CatalogStoreSummary::Account { store } => StoreDto {
            id: store_id(&CatalogStoreRef::Account(store.clone())),
            kind: "account",
            name: store.account_alias.clone(),
            server: store.profile.clone(),
            account: store.account_alias.clone(),
            alias: None,
            active: None,
            team_kind: None,
            team_id_hex: None,
        },
        CatalogStoreSummary::Team {
            store,
            kind,
            name,
            active,
        } => {
            let team_kind = match kind.as_str() {
                "named" => "named",
                "ad-hoc" => "adhoc",
                other => {
                    return Err(AgentError::new(
                        "invalid-response",
                        format!("The agent returned unsupported group kind {other:?}."),
                        false,
                    ));
                }
            };
            StoreDto {
                id: store_id(&CatalogStoreRef::Team(store.clone())),
                kind: "team",
                name: name.clone().unwrap_or_else(|| store.team_alias.clone()),
                server: store.profile.clone(),
                account: store.account_alias.clone(),
                alias: Some(store.team_alias.clone()),
                active: Some(*active),
                team_kind: Some(team_kind.to_owned()),
                team_id_hex: Some(store.team_id.clone()),
            }
        }
    })
}

fn item_dto(item: &CatalogItem) -> Result<ItemDto, AgentError> {
    let size = match (item.metadata.node_type.as_str(), item.metadata.size) {
        ("directory", size) => size.unwrap_or(0),
        (_, Some(size)) => size,
        _ => {
            return Err(AgentError::new(
                "invalid-response",
                "The agent omitted a catalog item size.",
                false,
            ));
        }
    };
    Ok(ItemDto {
        store: store_id(&item.store),
        path: item.metadata.path.clone(),
        kind: match item.metadata.node_type.as_str() {
            "small-file" => "Secret",
            "file" => "File",
            "symlink" => "Link",
            "directory" => "Folder",
            other => {
                return Err(AgentError::new(
                    "invalid-response",
                    format!("The agent returned unsupported catalog node type {other:?}."),
                    false,
                ));
            }
        },
        size,
        version: item.metadata.version,
        read: item.metadata.read_role.into(),
        write: item.metadata.write_role.into(),
    })
}

#[derive(Debug, Serialize)]
pub struct ReadItemDto {
    pub store: String,
    pub path: String,
    pub version: u64,
    #[serde(serialize_with = "serialize_secret")]
    pub value: Zeroizing<String>,
}

fn serialize_secret<S>(value: &Zeroizing<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

fn deserialize_secret<'de, D>(deserializer: D) -> Result<Zeroizing<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Zeroizing::new)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandAck {
    pub ok: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadResult {
    pub saved: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MutationDto {
    pub applied: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MutationKind {
    Create,
    Guarded,
    Resume,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileSummary {
    name: String,
    probe: String,
    protocol: ProfileProtocolSummary,
    trust: ProfileTrustSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "generation", rename_all = "kebab-case", deny_unknown_fields)]
enum ProfileProtocolSummary {
    V019,
    CurrentProbeOnly {
        canary_public_key: String,
        lease_url: String,
        last_artifact: Option<Box<foks_compat_artifact::SignedCanaryArtifact>>,
    },
    CurrentValidated {
        canary_public_key: String,
        lease_url: String,
        artifact: Box<foks_compat_artifact::SignedCanaryArtifact>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum ProfileTrustSummary {
    WebPki,
    CertificateDer { path: PathBuf },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingOperationResponse {
    kind: PendingOperationKind,
    alias: String,
    target: Option<String>,
}

impl From<PendingOperationResponse> for PendingOperationSummary {
    fn from(response: PendingOperationResponse) -> Self {
        Self {
            kind: response.kind,
            alias: response.alias,
            target: response.target,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitializationResponse {
    backend: CredentialBackend,
}

#[derive(Debug, Serialize)]
pub struct ServerDto {
    pub id: String,
    pub name: String,
    pub label: Option<String>,
    pub host_id: Option<String>,
    pub chain: Option<u64>,
    pub epoch: Option<u64>,
    pub lease: Option<serde_json::Value>,
    pub accounts: Vec<String>,
    pub state: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedServerDto {
    pub profile: String,
    pub configured_probe: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredHostDto {
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id: String,
    pub chain: u64,
    pub epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusSnapshotDto {
    pub profile: String,
    pub configured_probe: String,
    pub host: Option<StoredHostDto>,
    pub lease_required: bool,
    pub lease_expires_at: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedServerDto {
    pub profile: String,
    pub acceptance: String,
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id: String,
    pub chain: u64,
    pub epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgottenServerDto {
    pub profile: String,
    pub removed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDto {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub role: &'static str,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRemovalDto {
    pub device_id: String,
    pub user_chain_sequence: u64,
    pub already_absent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEnrollmentDto {
    pub backup_alias: String,
    pub account_alias: String,
    pub backup_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiCardDto {
    pub serial: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiEnrollmentDto {
    pub alias: String,
    pub state: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiAccountDto {
    pub alias: String,
    pub username: String,
    pub yubi_id: String,
    pub subkey_id: String,
    pub user_chain_sequence: u64,
    pub management_enrolled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiSyncDto {
    pub username: String,
    pub user_chain_sequence: u64,
    pub directories: usize,
    pub entries: usize,
    pub federation: Vec<YubiFederationRefreshDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiFederationRefreshDto {
    pub local_profile: String,
    pub local_team_alias: String,
    pub refreshed: bool,
    pub deferred: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiPinStatusDto {
    pub remaining: u8,
    pub blocked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiLifecycleDto {
    pub alias: String,
    pub management_enrolled: bool,
    pub management_generation: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiSubkeyRecoveryDto {
    pub alias: String,
    pub subkey_id: String,
    pub certificate_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YubiRevocationDto {
    pub alias: String,
    pub user_chain_sequence: u64,
    pub removed_local_credential: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct YubiChangedDto {
    pub alias: String,
    pub changed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOfferDto {
    pub account_alias: String,
    #[serde(serialize_with = "serialize_secret")]
    pub phrase: Zeroizing<String>,
}

impl std::fmt::Debug for PairingOfferDto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PairingOfferDto")
            .field("account_alias", &self.account_alias)
            .field("phrase", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProvisionDto {
    pub alias: String,
    pub device_id: String,
    pub user_chain_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassphraseReportDto {
    pub generation: u64,
    pub stretch_version: String,
    pub verified: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetArtifactDto {
    pub kind: &'static str,
    pub entries: u64,
    pub bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPreviewDto {
    pub profile: String,
    pub resumables: Vec<PendingOperationDto>,
    pub artifacts: Vec<ResetArtifactDto>,
    #[serde(serialize_with = "serialize_secret")]
    pub token: Zeroizing<String>,
    pub expires_in_seconds: u64,
}

impl std::fmt::Debug for ResetPreviewDto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResetPreviewDto")
            .field("profile", &self.profile)
            .field("resumables", &self.resumables)
            .field("artifacts", &self.artifacts)
            .field("token", &"[REDACTED]")
            .field("expires_in_seconds", &self.expires_in_seconds)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerStatusResponse {
    profile: String,
    configured_probe: String,
    host: Option<StoredHostResponse>,
    lease_required: bool,
    lease_expires_at: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredHostResponse {
    lookup_name: String,
    canonical_name: String,
    host_id_hex: String,
    host_chain_sequence: u64,
    merkle_epoch: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceResponse {
    id_hex: String,
    name: Option<String>,
    role: String,
    current: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceRemovalResponse {
    device_id_hex: String,
    user_chain_sequence: u64,
    already_absent: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupEnrollmentResponse {
    backup_alias: String,
    account_alias: String,
    backup_id_hex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiCardResponse {
    name: String,
    serial: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiEnrollmentResponse {
    alias: String,
    state: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiAccountResponse {
    alias: String,
    username: String,
    yubi_id_hex: String,
    subkey_id_hex: String,
    user_chain_sequence: u64,
    management_enrolled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiFederationSyncResponse {
    sync: AccountSyncResponse,
    federation: Vec<YubiFederationRefreshResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiFederationRefreshResponse {
    local_profile: String,
    local_team_alias: String,
    refreshed: bool,
    deferred: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiPinStatusResponse {
    remaining: u8,
    blocked: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiLifecycleResponse {
    alias: String,
    management_enrolled: bool,
    management_generation: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiSubkeyRecoveryResponse {
    alias: String,
    subkey_id_hex: String,
    certificate_count: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiRevocationResponse {
    alias: String,
    user_chain_sequence: u64,
    removed_local_credential: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YubiChangedResponse {
    alias: String,
    changed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingOfferResponse {
    account_alias: String,
    #[serde(deserialize_with = "deserialize_secret")]
    phrase: Zeroizing<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceProvisionResponse {
    alias: String,
    device_id_hex: String,
    user_chain_sequence: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovedProfileResponse {
    profile: String,
    removed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetPreviewResponse {
    profile: String,
    resumables: Vec<PendingOperationResponse>,
    artifacts: Vec<ResetArtifactResponse>,
    #[serde(deserialize_with = "deserialize_secret")]
    token: Zeroizing<String>,
    expires_in_seconds: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetArtifactResponse {
    kind: foks_agent_proto::ResetArtifactKind,
    entries: u64,
    bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetResultResponse {
    profile: String,
    hard_state_reset: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemberRole {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl<'de> Deserialize<'de> for MemberRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum WireRole {
            Member(MemberFields),
            Admin,
            Owner,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct MemberFields {
            visibility: i16,
        }

        Ok(match WireRole::deserialize(deserializer)? {
            WireRole::Member(fields) => Self::Member {
                visibility: fields.visibility,
            },
            WireRole::Admin => Self::Admin,
            WireRole::Owner => Self::Owner,
        })
    }
}

impl MemberRole {
    fn is_strictly_lower_than(self, current: Self) -> bool {
        match (self, current) {
            (
                Self::Member {
                    visibility: destination,
                },
                Self::Member {
                    visibility: current,
                },
            ) => destination < current,
            (Self::Member { .. }, Self::Admin | Self::Owner) | (Self::Admin, Self::Owner) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleInput {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl<'de> Deserialize<'de> for RoleInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum RoleName {
            Member,
            Admin,
            Owner,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireRole {
            role: RoleName,
            visibility: Option<i16>,
        }

        let wire = WireRole::deserialize(deserializer)?;
        match (wire.role, wire.visibility) {
            (RoleName::Member, Some(visibility)) => Ok(Self::Member { visibility }),
            (RoleName::Admin, None) => Ok(Self::Admin),
            (RoleName::Owner, None) => Ok(Self::Owner),
            (RoleName::Member, None) => Err(serde::de::Error::missing_field("visibility")),
            (RoleName::Admin | RoleName::Owner, Some(_)) => Err(serde::de::Error::custom(
                "visibility is valid only for a Member role",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GroupKindInput {
    Named,
    Adhoc,
}

impl RoleInput {
    fn parts(self) -> (TeamRole, i16) {
        match self {
            Self::Member { visibility } => (TeamRole::Member, visibility),
            Self::Admin => (TeamRole::Admin, 0),
            Self::Owner => (TeamRole::Owner, 0),
        }
    }
}

impl From<RoleInput> for MemberRole {
    fn from(role: RoleInput) -> Self {
        match role {
            RoleInput::Member { visibility } => Self::Member { visibility },
            RoleInput::Admin => Self::Admin,
            RoleInput::Owner => Self::Owner,
        }
    }
}

impl From<MemberRole> for RoleDto {
    fn from(role: MemberRole) -> Self {
        match role {
            MemberRole::Member { visibility } => KvRole::Member { visibility }.into(),
            MemberRole::Admin => KvRole::Admin.into(),
            MemberRole::Owner => KvRole::Owner.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberResponse {
    username: Option<String>,
    party_id_hex: String,
    scoped_host_id_hex: Option<String>,
    party_kind: String,
    source_role: MemberRole,
    destination_role: MemberRole,
    generation: u64,
    locally_manageable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PartyDto {
    pub store: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub party_kind: String,
    pub generation: u64,
    pub locally_manageable: bool,
    pub party_id_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoped_host_id_hex: Option<String>,
    pub source_role: RoleDto,
    pub destination_role: RoleDto,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FederationResponse {
    local_team_alias: String,
    remote_profile: String,
    remote_team_alias: String,
    remote_host_id_hex: String,
    remote_team_id_hex: String,
    destination: MemberRole,
    operation_id_hex: Option<String>,
    active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FederationEntryDto {
    pub store: String,
    pub remote_profile: String,
    pub remote_team_alias: String,
    pub remote_host_id_hex: String,
    pub remote_team_id_hex: String,
    pub destination: RoleDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id_hex: Option<String>,
    pub active: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountResponse {
    profile: String,
    alias: String,
    username: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccountDto {
    pub store: String,
    pub profile: String,
    pub alias: String,
    pub username: String,
}

fn invalid_response(message: impl Into<String>) -> AgentError {
    AgentError::new("invalid-response", message, false)
}

fn ambiguous_mutation_response(state: &AppState, message: impl Into<String>) -> AgentError {
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    let mut error = AgentError::new("response-binding", message, false);
    error.ambiguous = true;
    error.fatal = true;
    error
}

fn load_accounts(
    transport: &dyn foks_desktop::AgentTransport,
    catalog: &CatalogSnapshot,
) -> Result<Vec<AccountDto>, AgentError> {
    let mut expected = HashMap::new();
    for summary in &catalog.stores {
        if let CatalogStoreSummary::Account { store } = summary {
            // A failed rollback/lease catalog can retain the store summary so
            // the shell can explain which server is blocked. Do not turn that
            // display fact into an account read, and do not fail identities
            // from still-available profiles beside it.
            if catalog.profile_blocked(&store.profile) {
                continue;
            }
            if !valid_response_text(&store.profile, 64) || !valid_local_name(&store.account_alias) {
                return Err(invalid_response(
                    "The catalog returned an invalid account identity.",
                ));
            }
            let identity = (store.profile.clone(), store.account_alias.clone());
            if expected
                .insert(identity, store_id(&CatalogStoreRef::Account(store.clone())))
                .is_some()
            {
                return Err(invalid_response(
                    "The catalog returned a duplicate account identity.",
                ));
            }
        }
    }
    if expected.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response(
            "The catalog returned too many account identities.",
        ));
    }
    let mut profiles = expected
        .keys()
        .map(|(profile, _)| profile.clone())
        .collect::<Vec<_>>();
    profiles.sort();
    profiles.dedup();
    let mut accounts = Vec::with_capacity(expected.len());
    for profile in profiles {
        let value = transport
            .call(Operation::ListAccounts {
                profile: profile.clone(),
            })
            .map_err(AgentError::from_desktop)?;
        require_response_row_cap(&value, "account identities")?;
        let rows: Vec<AccountResponse> =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        for row in rows {
            if row.profile != profile
                || !valid_local_name(&row.alias)
                || !valid_response_text(&row.username, 256)
            {
                return Err(invalid_response(
                    "The agent returned an invalid account identity.",
                ));
            }
            let identity = (row.profile.clone(), row.alias.clone());
            let Some(store) = expected.remove(&identity) else {
                return Err(invalid_response(
                    "The agent returned an account outside the current catalog.",
                ));
            };
            accounts.push(AccountDto {
                store,
                profile: row.profile,
                alias: row.alias,
                username: row.username,
            });
        }
    }
    if !expected.is_empty() {
        return Err(invalid_response(
            "The agent omitted an account from the current catalog.",
        ));
    }
    accounts
        .sort_by(|left, right| (&left.profile, &left.alias).cmp(&(&right.profile, &right.alias)));
    Ok(accounts)
}

fn load_account_devices(
    transport: &dyn foks_desktop::AgentTransport,
    account: &foks_agent_proto::AccountStoreRef,
) -> Result<Vec<DeviceDto>, AgentError> {
    require_transport_profile(transport, &account.profile)?;
    let value = transport
        .call(Operation::ListDevices {
            profile: account.profile.clone(),
            alias: account.account_alias.clone(),
        })
        .map_err(AgentError::from_desktop)?;
    device_dtos(value)
}

fn load_backup_enrollments(
    transport: &dyn foks_desktop::AgentTransport,
    account: &foks_agent_proto::AccountStoreRef,
) -> Result<Vec<BackupEnrollmentDto>, AgentError> {
    require_transport_profile(transport, &account.profile)?;
    let value = transport
        .call(Operation::ListBackupEnrollments {
            profile: account.profile.clone(),
            account_alias: account.account_alias.clone(),
        })
        .map_err(AgentError::from_desktop)?;
    backup_enrollment_dtos(value, &account.account_alias)
}

fn party_dtos(store: &str, members: Vec<MemberResponse>) -> Result<Vec<PartyDto>, AgentError> {
    if members.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response(
            "The agent returned too many group roster entries.",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    members
        .into_iter()
        .map(|member| {
            let (party_shape_is_valid, party_id_is_valid) = match member.party_kind.as_str() {
                "user" => (
                    !member.locally_manageable
                        || (member.username.is_some() && member.scoped_host_id_hex.is_none()),
                    valid_typed_entity_id_hex(&member.party_id_hex, USER_ID_PREFIX),
                ),
                "named-team" => (
                    member.username.is_none() && !member.locally_manageable,
                    valid_typed_entity_id_hex(&member.party_id_hex, NAMED_TEAM_ID_PREFIX),
                ),
                "ad-hoc-team" => (
                    member.username.is_none() && !member.locally_manageable,
                    valid_typed_entity_id_hex(&member.party_id_hex, AD_HOC_TEAM_ID_PREFIX),
                ),
                _ => (false, false),
            };
            if !party_shape_is_valid
                || !party_id_is_valid
                || member
                    .username
                    .as_ref()
                    .is_some_and(|name| !valid_response_text(name, 256))
                || member
                    .scoped_host_id_hex
                    .as_ref()
                    .is_some_and(|host| !valid_typed_entity_id_hex(host, HOST_ID_PREFIX))
                || !ids.insert(member.party_id_hex.clone())
            {
                return Err(invalid_response(
                    "The agent returned an unsupported or inconsistent roster entry.",
                ));
            }
            Ok(PartyDto {
                store: store.to_owned(),
                username: member.username,
                party_kind: member.party_kind,
                generation: member.generation,
                locally_manageable: member.locally_manageable,
                party_id_hex: member.party_id_hex,
                scoped_host_id_hex: member.scoped_host_id_hex,
                source_role: member.source_role.into(),
                destination_role: member.destination_role.into(),
            })
        })
        .collect()
}

fn federation_dtos(
    store: &str,
    local_profile: &str,
    local_team_alias: &str,
    memberships: Vec<FederationResponse>,
) -> Result<Vec<FederationEntryDto>, AgentError> {
    if memberships.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response(
            "The agent returned too many federation entries.",
        ));
    }
    let mut remote_aliases = std::collections::HashSet::new();
    let mut remote_ids = std::collections::HashSet::new();
    let mut operation_ids = std::collections::HashSet::new();
    memberships
        .into_iter()
        .map(|membership| {
            if membership.local_team_alias != local_team_alias
                || membership.remote_profile == local_profile
                || !valid_local_name(&membership.local_team_alias)
                || !valid_response_text(&membership.remote_profile, 64)
                || !valid_local_name(&membership.remote_team_alias)
                || !valid_typed_entity_id_hex(&membership.remote_host_id_hex, HOST_ID_PREFIX)
                || !(valid_typed_entity_id_hex(
                    &membership.remote_team_id_hex,
                    NAMED_TEAM_ID_PREFIX,
                ) || valid_typed_entity_id_hex(
                    &membership.remote_team_id_hex,
                    AD_HOC_TEAM_ID_PREFIX,
                ))
                || membership
                    .operation_id_hex
                    .as_ref()
                    .is_some_and(|operation| !valid_operation_id_hex(operation))
                || !matches!(membership.destination, MemberRole::Member { .. })
                || !remote_aliases.insert((
                    membership.remote_profile.clone(),
                    membership.remote_team_alias.clone(),
                ))
                || !remote_ids.insert((
                    membership.remote_host_id_hex.clone(),
                    membership.remote_team_id_hex.clone(),
                ))
                || membership
                    .operation_id_hex
                    .as_ref()
                    .is_some_and(|operation| !operation_ids.insert(operation.clone()))
            {
                return Err(invalid_response(
                    "The agent returned an unsupported or inconsistent federation entry.",
                ));
            }
            Ok(FederationEntryDto {
                store: store.to_owned(),
                remote_profile: membership.remote_profile,
                remote_team_alias: membership.remote_team_alias,
                remote_host_id_hex: membership.remote_host_id_hex,
                remote_team_id_hex: membership.remote_team_id_hex,
                destination: membership.destination.into(),
                operation_id_hex: membership.operation_id_hex,
                active: membership.active,
            })
        })
        .collect()
}

#[tauri::command]
pub fn take_agent_connection_loss(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Option<String>, AgentError> {
    require_main_window(&webview)?;
    Ok(state.agent.take_connection_failure())
}

fn read_text(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
) -> Result<ReadItemDto, AgentError> {
    let KvItemRead {
        path,
        version,
        value,
        ..
    } = foks_desktop::read_catalog_item(transport, item).map_err(AgentError::from_desktop)?;
    let value = match value {
        KvItemValue::File(bytes) => String::from_utf8(bytes.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| {
                AgentError::new(
                    "not-text",
                    "This file is not UTF-8 text. Download it instead.",
                    false,
                )
            })?,
        KvItemValue::Symlink(target) => Zeroizing::new(target.to_string()),
        KvItemValue::Directory => {
            return Err(AgentError::new(
                "not-readable",
                "A folder has no value to show.",
                false,
            ))
        }
    };
    Ok(ReadItemDto {
        store: store_id(&item.store),
        path,
        version,
        value,
    })
}

const DOWNLOAD_CHUNK_BYTES: u32 = 128 * 1024;

fn protocol_store(store: &CatalogStoreRef) -> KvStoreRef {
    match store {
        CatalogStoreRef::Account(store) => KvStoreRef::Account(store.clone()),
        CatalogStoreRef::Team(store) => KvStoreRef::Team(store.clone()),
    }
}

fn download_to_path(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
    destination: &Path,
) -> Result<(), AgentError> {
    let store = protocol_store(&item.store);
    let value = transport
        .call(Operation::ReadKv {
            store: store.clone(),
            path: item.metadata.path.clone(),
            version: item.metadata.version,
        })
        .map_err(AgentError::from_desktop)?;
    let mut read: KvReadResult = serde_json::from_value(value)
        .map_err(|error| AgentError::new("invalid-response", error.to_string(), false))?;
    let metadata_matches = read.store == store
        && read.path == item.metadata.path
        && read.version == item.metadata.version
        && read.node_type == item.metadata.node_type
        && read.size == item.metadata.size
        && read.read_role == item.metadata.read_role
        && read.write_role == item.metadata.write_role;
    if !metadata_matches {
        if let Some(content) = &mut read.content {
            zeroize::Zeroize::zeroize(content);
        }
        if let Some(target) = &mut read.symlink_target {
            zeroize::Zeroize::zeroize(target);
        }
        return Err(AgentError::new(
            "response-binding",
            "The agent returned a value for a different catalog selection.",
            true,
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        AgentError::new(
            "download-path",
            "The chosen destination has no parent folder.",
            false,
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        AgentError::new(
            "download-write",
            format!("could not create the download: {error}"),
            true,
        )
    })?;
    match read.node_type.as_str() {
        "small-file" => {
            if read.symlink_target.is_some() {
                zeroize_read_payload(&mut read);
                return Err(AgentError::new(
                    "invalid-response",
                    "The agent mixed a link target into a file response.",
                    false,
                ));
            }
            let mut content = read.content.take().ok_or_else(|| {
                AgentError::new("invalid-response", "The agent omitted file content.", false)
            })?;
            if Some(content.len() as u64) != read.size {
                zeroize::Zeroize::zeroize(&mut content);
                return Err(AgentError::new(
                    "invalid-response",
                    "The inline file length did not match its catalog size.",
                    false,
                ));
            }
            temporary.write_all(&content).map_err(|error| {
                AgentError::new(
                    "download-write",
                    format!("could not save the file: {error}"),
                    true,
                )
            })?;
            zeroize::Zeroize::zeroize(&mut content);
        }
        "file" => {
            if read.content.is_some() || read.symlink_target.is_some() {
                zeroize_read_payload(&mut read);
                return Err(AgentError::new(
                    "invalid-response",
                    "The agent mixed inline data into a chunked file response.",
                    false,
                ));
            }
            let total = read.size.ok_or_else(|| {
                AgentError::new(
                    "invalid-response",
                    "The agent omitted the file size.",
                    false,
                )
            })?;
            let mut offset = 0u64;
            while offset < total {
                let length = u32::try_from((total - offset).min(u64::from(DOWNLOAD_CHUNK_BYTES)))
                    .expect("the download chunk bound fits u32");
                let value = transport
                    .call(Operation::ReadKvChunk {
                        store: store.clone(),
                        path: item.metadata.path.clone(),
                        version: item.metadata.version,
                        offset,
                        length,
                    })
                    .map_err(AgentError::from_desktop)?;
                let mut chunk: KvChunkResult = serde_json::from_value(value).map_err(|error| {
                    AgentError::new("invalid-response", error.to_string(), false)
                })?;
                let next = offset
                    .checked_add(chunk.content.len() as u64)
                    .ok_or_else(|| {
                        AgentError::new("invalid-response", "The file offset overflowed.", false)
                    })?;
                let valid = chunk.store == store
                    && chunk.path == item.metadata.path
                    && chunk.version == item.metadata.version
                    && chunk.offset == offset
                    && !chunk.content.is_empty()
                    && chunk.content.len() <= length as usize
                    && next <= total
                    && chunk.eof == (next == total);
                if !valid {
                    zeroize::Zeroize::zeroize(&mut chunk.content);
                    return Err(AgentError::new(
                        "response-binding",
                        "The agent returned an invalid or unbound file chunk.",
                        true,
                    ));
                }
                let write = temporary.write_all(&chunk.content);
                zeroize::Zeroize::zeroize(&mut chunk.content);
                write.map_err(|error| {
                    AgentError::new(
                        "download-write",
                        format!("could not save the file: {error}"),
                        true,
                    )
                })?;
                offset = next;
            }
        }
        _ => {
            zeroize_read_payload(&mut read);
            return Err(AgentError::new(
                "not-file",
                "Only files can be downloaded.",
                false,
            ));
        }
    }
    temporary.as_file().sync_all().map_err(|error| {
        AgentError::new(
            "download-write",
            format!("could not finish saving the file: {error}"),
            true,
        )
    })?;
    temporary.persist(destination).map_err(|error| {
        AgentError::new(
            "download-write",
            format!("could not place the downloaded file: {}", error.error),
            true,
        )
    })?;
    Ok(())
}

fn zeroize_read_payload(read: &mut KvReadResult) {
    if let Some(content) = &mut read.content {
        zeroize::Zeroize::zeroize(content);
    }
    if let Some(target) = &mut read.symlink_target {
        zeroize::Zeroize::zeroize(target);
    }
}

fn invalid_request(message: impl Into<String>) -> AgentError {
    AgentError::new("invalid-request", message, false)
}

fn parse_item_role(value: &str) -> Result<KvRole, AgentError> {
    match value {
        "Owner" => Ok(KvRole::Owner),
        "Admin" => Ok(KvRole::Admin),
        value => {
            let Some(visibility) = value.strip_prefix("Member:") else {
                return Err(invalid_request(
                    "Use Owner, Admin, or Member:<signed visibility> for an item role.",
                ));
            };
            let parsed = visibility.parse::<i16>().map_err(|_| {
                invalid_request(
                    "A Member item role needs a signed 16-bit visibility after Member:.",
                )
            })?;
            if parsed.to_string() != visibility {
                return Err(invalid_request(
                    "Use the canonical signed integer form after Member:.",
                ));
            }
            Ok(KvRole::Member { visibility: parsed })
        }
    }
}

fn create_item_roles(
    store: &CatalogStoreRef,
    read_role: Option<&str>,
    write_role: Option<&str>,
) -> Result<(KvRole, KvRole), AgentError> {
    match store {
        CatalogStoreRef::Account(_) => match (read_role, write_role) {
            (None, None) => Ok((KvRole::Owner, KvRole::Owner)),
            _ => Err(invalid_request(
                "Account item roles are fixed to Owner; omit readRole and writeRole.",
            )),
        },
        CatalogStoreRef::Team(_) => match (read_role, write_role) {
            (Some(read_role), Some(write_role)) => {
                Ok((parse_item_role(read_role)?, parse_item_role(write_role)?))
            }
            _ => Err(invalid_request(
                "Group item creation requires both readRole and writeRole.",
            )),
        },
    }
}

fn set_create_operation_roles(
    mut operation: Operation,
    read_role: KvRole,
    write_role: KvRole,
) -> Result<Operation, AgentError> {
    set_create_operation_roles_in_place(&mut operation, read_role, write_role)?;
    Ok(operation)
}

fn set_create_operation_roles_in_place(
    operation: &mut Operation,
    read_role: KvRole,
    write_role: KvRole,
) -> Result<(), AgentError> {
    match operation {
        Operation::PutKv {
            read_role: operation_read,
            write_role: operation_write,
            ..
        }
        | Operation::PutKvSymlink {
            read_role: operation_read,
            write_role: operation_write,
            ..
        }
        | Operation::MkdirKv {
            read_role: operation_read,
            write_role: operation_write,
            ..
        } => {
            *operation_read = read_role;
            *operation_write = write_role;
            Ok(())
        }
        _ => Err(AgentError::new(
            "invalid-builder",
            "The desktop create builder returned an unsupported operation shape.",
            false,
        )),
    }
}

fn set_create_mutation_roles(
    mut mutation: KvAccountMutation,
    read_role: KvRole,
    write_role: KvRole,
) -> Result<KvAccountMutation, AgentError> {
    match &mut mutation {
        KvAccountMutation::Inline(operation) => {
            set_create_operation_roles_in_place(operation, read_role, write_role)?;
        }
        KvAccountMutation::Stream { header, .. } => {
            header.read_role = read_role;
            header.write_role = write_role;
        }
    }
    Ok(mutation)
}

fn take_text_value(value: String) -> Result<Vec<u8>, AgentError> {
    let mut value = Zeroizing::new(value);
    if value.len() > MAXIMUM_TEXT_ITEM_BYTES {
        return Err(invalid_request(
            "Password and Resource values must be at most 128 KiB.",
        ));
    }
    Ok(std::mem::take(&mut *value).into_bytes())
}

fn require_file_item(item: &CatalogItem) -> Result<(), AgentError> {
    if matches!(item.metadata.node_type.as_str(), "file" | "small-file") {
        Ok(())
    } else {
        Err(invalid_request(
            "Only a File can be replaced from a native file source.",
        ))
    }
}

fn require_text_item(item: &CatalogItem) -> Result<(), AgentError> {
    if item.metadata.node_type == "small-file" {
        Ok(())
    } else {
        Err(invalid_request(
            "Only a Password or Resource can be edited as text.",
        ))
    }
}

fn remove_item_operation(item: &CatalogItem) -> Result<Operation, AgentError> {
    // This product release has no folder-deletion surface. Keep recursive
    // deletion out of the untrusted-renderer contract entirely.
    if item.metadata.node_type == "directory" {
        return Err(invalid_request(
            "Folder removal is not available in this desktop yet.",
        ));
    }
    foks_desktop::remove_kv_operation(item, false).map_err(invalid_request)
}

fn create_group_operation(
    account: foks_agent_proto::AccountStoreRef,
    team_alias: &str,
    name: &str,
    kind: GroupKindInput,
) -> Result<Operation, AgentError> {
    let team_alias = required_field(team_alias, "Enter a local alias for the group.")?;
    let (name, kind) = match kind {
        GroupKindInput::Named => (
            required_field(name, "Enter the group's FOKS name.")?,
            TeamKind::Named,
        ),
        // An ad-hoc group's user-facing local name is represented by the
        // caller-derived local alias. The protocol requires no server name.
        GroupKindInput::Adhoc => (String::new(), TeamKind::AdHoc),
    };
    Ok(Operation::CreateTeam {
        profile: account.profile,
        account_alias: account.account_alias,
        team_alias,
        name,
        kind,
    })
}

fn add_group_member_operation(
    team: foks_agent_proto::TeamStoreRef,
    username: &str,
    destination: RoleInput,
) -> Result<Operation, AgentError> {
    let username = required_field(username, "Enter the member username.")?;
    let (role, visibility) = destination.parts();
    Ok(Operation::AddTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        username,
        role,
        visibility,
    })
}

fn demote_group_member_operation(
    team: foks_agent_proto::TeamStoreRef,
    party_id_hex: &str,
    current: MemberRole,
    destination: RoleInput,
) -> Result<Operation, AgentError> {
    let destination_role = MemberRole::from(destination);
    if !destination_role.is_strictly_lower_than(current) {
        return Err(AgentError::new(
            "not-a-demotion",
            "Choose a role strictly below the member's current role.",
            false,
        ));
    }
    if !valid_typed_entity_id_hex(party_id_hex, USER_ID_PREFIX) {
        return Err(invalid_request("Choose an authenticated user identity."));
    }
    let party_id_hex = party_id_hex.to_owned();
    let (role, visibility) = destination.parts();
    Ok(Operation::DemoteTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        party_id_hex,
        role,
        visibility,
    })
}

fn remove_group_member_operation(
    team: foks_agent_proto::TeamStoreRef,
    party_id_hex: &str,
) -> Result<Operation, AgentError> {
    if !valid_typed_entity_id_hex(party_id_hex, USER_ID_PREFIX) {
        return Err(invalid_request("Choose an authenticated user identity."));
    }
    Ok(Operation::RemoveTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        party_id_hex: party_id_hex.to_owned(),
    })
}

fn admit_group_operation(
    local: foks_agent_proto::TeamStoreRef,
    remote: foks_agent_proto::TeamStoreRef,
    visibility: i16,
) -> Operation {
    Operation::AdmitFederatedTeam {
        local_profile: local.profile,
        local_team_alias: local.team_alias,
        remote_profile: remote.profile,
        remote_team_alias: remote.team_alias,
        role: FederationRole::Member,
        visibility,
    }
}

fn rerun_group_admission_operation(
    local: foks_agent_proto::TeamStoreRef,
    entry: FederationEntryDto,
) -> Result<Operation, AgentError> {
    let visibility = match (entry.destination.role, entry.destination.visibility) {
        ("Member", Some(visibility)) => visibility,
        _ => {
            return Err(invalid_response(
                "The retained federation destination is unsupported.",
            ));
        }
    };
    Ok(Operation::AdmitFederatedTeam {
        local_profile: local.profile,
        local_team_alias: local.team_alias,
        remote_profile: entry.remote_profile,
        remote_team_alias: entry.remote_team_alias,
        role: FederationRole::Member,
        visibility,
    })
}

fn ambiguous_worker_failure(state: &AppState, message: impl Into<String>) -> AgentError {
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    let mut error = AgentError::new("ambiguous", message, false);
    error.ambiguous = true;
    error
}

fn map_mutation_error(error: foks_desktop::AgentError, kind: MutationKind) -> AgentError {
    let mut error = AgentError::from_desktop(error);
    if error.ambiguous {
        error.code = "ambiguous".to_owned();
        error.retryable = false;
        return error;
    }
    if error.code == "io" {
        error.code = "agent-lost".to_owned();
        error.fatal = true;
        return error;
    }
    if error.code == "capability-denied"
        && error
            .details
            .as_deref()
            .and_then(|details| details.capability.as_deref())
            == Some("kv")
    {
        // The authenticated response proves that KV is unavailable, but the
        // current protocol does not say whether a lease lapsed or the profile
        // never granted this capability. Do not manufacture the former fact.
        error.code = "capability-unavailable".to_owned();
        error.message =
            "This server is not granting KV access. Check its compatibility status before reading or writing."
                .to_owned();
        error.retryable = false;
        return error;
    }
    if error.code == "conflict" {
        error.code = match kind {
            MutationKind::Create => "already-exists",
            MutationKind::Guarded | MutationKind::Resume => "conflict",
        }
        .to_owned();
        error.retryable = false;
    }
    error
}

fn execute_kv_mutation(
    transport: &dyn foks_desktop::AgentTransport,
    mutation: KvAccountMutation,
    kind: MutationKind,
) -> Result<(), AgentError> {
    match mutation {
        KvAccountMutation::Inline(operation) => {
            transport
                .call(operation)
                .map_err(|error| map_mutation_error(error, kind))?;
        }
        KvAccountMutation::Stream {
            header,
            mut content,
        } => {
            let mut reader = std::io::Cursor::new(content.as_mut_slice());
            transport
                .put_kv_stream(header, &mut reader)
                .map_err(|error| map_mutation_error(error, kind))?;
        }
    }
    Ok(())
}

async fn apply_kv_mutation(
    state: &AppState,
    mutation: KvAccountMutation,
    kind: MutationKind,
) -> Result<MutationDto, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_kv_mutation(transport.as_ref(), mutation, kind)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The change worker stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(()) => Ok(MutationDto { applied: true }),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

async fn apply_operation(
    state: &AppState,
    operation: Operation,
    kind: MutationKind,
) -> Result<MutationDto, AgentError> {
    apply_operation_value(state, operation, kind)
        .await
        .map(|_| MutationDto { applied: true })
}

async fn apply_operation_value(
    state: &AppState,
    operation: Operation,
    kind: MutationKind,
) -> Result<serde_json::Value, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        transport
            .call(operation)
            .map_err(|error| map_mutation_error(error, kind))
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The change worker stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

fn require_transport_profile(
    transport: &dyn foks_desktop::AgentTransport,
    expected: &str,
) -> Result<(), AgentError> {
    transport_profile(transport, expected).map(|_| ())
}

fn transport_profile(
    transport: &dyn foks_desktop::AgentTransport,
    expected: &str,
) -> Result<ProfileSummary, AgentError> {
    let value = transport
        .call(Operation::ListProfiles)
        .map_err(AgentError::from_desktop)?;
    let profiles = profile_summaries(value)?;
    profiles
        .into_iter()
        .find(|profile| profile.name == expected)
        .ok_or_else(|| {
            AgentError::new(
                "profile-not-found",
                "Refresh the server list and choose an available server.",
                false,
            )
        })
}

fn valid_profile_summary(profile: &ProfileSummary) -> bool {
    valid_local_name(&profile.name)
        && valid_probe_target(&profile.probe)
        && match &profile.trust {
            ProfileTrustSummary::WebPki => true,
            ProfileTrustSummary::CertificateDer { path } => !path.as_os_str().is_empty(),
        }
        && match &profile.protocol {
            ProfileProtocolSummary::V019 => true,
            ProfileProtocolSummary::CurrentProbeOnly {
                canary_public_key,
                lease_url,
                last_artifact,
            } => {
                valid_current_profile_policy(
                    canary_public_key,
                    lease_url,
                    last_artifact.as_deref(),
                    &profile.probe,
                ) && last_artifact
                    .as_deref()
                    .is_none_or(|artifact| !canary_grants_desktop(artifact))
            }
            ProfileProtocolSummary::CurrentValidated {
                canary_public_key,
                lease_url,
                artifact,
            } => {
                canary_grants_desktop(artifact)
                    && valid_current_profile_policy(
                        canary_public_key,
                        lease_url,
                        Some(artifact),
                        &profile.probe,
                    )
            }
        }
}

fn canary_grants_desktop(artifact: &foks_compat_artifact::SignedCanaryArtifact) -> bool {
    artifact.artifact.outcome == foks_compat_artifact::Outcome::Compatible
        && artifact.artifact.protocol_metadata_sha256 == PINNED_PROTOCOL_METADATA_SHA256
        && artifact.artifact.capabilities.iter().all(|capability| {
            matches!(
                capability.as_str(),
                "signup"
                    | "user-sync"
                    | "kv"
                    | "device-administration"
                    | "recovery"
                    | "passphrases"
                    | "teams"
                    | "federation"
            )
        })
}

fn valid_current_profile_policy(
    public_key: &str,
    lease_url: &str,
    artifact: Option<&foks_compat_artifact::SignedCanaryArtifact>,
    probe: &str,
) -> bool {
    let Ok(public_key) = foks_compat_artifact::decode_public_key(public_key) else {
        return false;
    };
    if !valid_lease_url(lease_url) {
        return false;
    }
    artifact.is_none_or(|artifact| {
        artifact.artifact.target == probe && artifact.verify(&public_key).is_ok()
    })
}

fn valid_lease_url(value: &str) -> bool {
    if value.is_empty() || value.len() > 2 * 1024 {
        return false;
    }
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    parsed.scheme() == "https"
        && !parsed.cannot_be_a_base()
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
}

fn valid_probe_target(input: &str) -> bool {
    normalized_probe_endpoint(input).is_some()
}

fn normalized_probe_hostname(input: &str) -> Option<String> {
    normalized_probe_endpoint(input).map(|(hostname, _)| hostname)
}

fn normalized_probe_endpoint(input: &str) -> Option<(String, u16)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let (hostname, port, is_ipv6) = if let Some(bracketed) = input.strip_prefix('[') {
        let (hostname, suffix) = bracketed.split_once(']')?;
        let port = match suffix {
            "" => 4430,
            suffix => suffix
                .strip_prefix(':')
                .and_then(|port| port.parse::<u16>().ok())?,
        };
        let Ok(hostname) = hostname.parse::<std::net::Ipv6Addr>() else {
            return None;
        };
        (hostname.to_string(), port, true)
    } else if let Ok(address) = input.parse::<std::net::Ipv6Addr>() {
        (address.to_string(), 4430, true)
    } else {
        let (hostname, port) = match input.rsplit_once(':') {
            Some((hostname, port)) if !hostname.contains(':') => {
                let Ok(port) = port.parse::<u16>() else {
                    return None;
                };
                (hostname, port)
            }
            Some(_) => return None,
            None => (input, 4430),
        };
        (
            hostname.trim_end_matches('.').to_ascii_lowercase(),
            port,
            false,
        )
    };
    if port == 0 {
        return None;
    }
    let valid = is_ipv6
        || (!hostname.is_empty()
            && hostname.is_ascii()
            && hostname.len() <= 253
            && hostname.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            }));
    valid.then_some((hostname, port))
}

fn profile_summaries(value: serde_json::Value) -> Result<Vec<ProfileSummary>, AgentError> {
    require_response_row_cap(&value, "server profiles")?;
    let rows = value
        .as_array()
        .expect("the row cap accepted only a JSON array");
    let profiles = rows
        .iter()
        .map(|row| {
            let profile: ProfileSummary = serde_json::from_value(row.clone())
                .map_err(|error| invalid_response(error.to_string()))?;
            let canonical = serde_json::to_value(&profile)
                .map_err(|error| invalid_response(error.to_string()))?;
            if canonical != *row || !valid_profile_summary(&profile) {
                return Err(invalid_response(
                    "The agent returned an invalid server profile.",
                ));
            }
            Ok(profile)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = std::collections::HashSet::new();
    if profiles
        .iter()
        .any(|profile| !valid_local_name(&profile.name) || !names.insert(profile.name.clone()))
    {
        return Err(invalid_response(
            "The agent returned duplicate or invalid server identities.",
        ));
    }
    Ok(profiles)
}

fn checked_profile_response_error(message: impl Into<String>) -> AgentError {
    let mut error = AgentError::new("response-binding", message, false);
    error.ambiguous = true;
    error.fatal = true;
    error
}

fn check_existing_or_add_profile(
    transport: &dyn foks_desktop::AgentTransport,
    profile_name: String,
    probe: String,
) -> Result<CheckedProfileDto, AgentError> {
    let requested_endpoint = normalized_probe_endpoint(&probe)
        .ok_or_else(|| invalid_request("Enter a valid DNS name, IP address, or host and port."))?;
    let profiles = profile_summaries(
        transport
            .call(Operation::ListProfiles)
            .map_err(AgentError::from_desktop)?,
    )?;
    let mut matching = profiles.into_iter().filter(|profile| {
        normalized_probe_endpoint(&profile.probe) == Some(requested_endpoint.clone())
    });
    let existing = matching.next();
    if matching.next().is_some() {
        return Err(AgentError::new(
            "profile-conflict",
            "More than one saved server profile uses this address. Remove the duplicate before continuing setup.",
            false,
        ));
    }
    if let Some(profile) = existing {
        let value = transport
            .call(Operation::Probe {
                profile: profile.name.clone(),
            })
            .map_err(|error| map_mutation_error(error, MutationKind::Guarded))?;
        return CheckedProfileDto::from_existing_probe(&probe, &profile, value)
            .map_err(|error| checked_profile_response_error(error.message));
    }

    let expected_profile = profile_name.clone();
    let expected_probe = probe.clone();
    let value = transport
        .call(Operation::CheckAndAddProfile {
            name: profile_name,
            probe,
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        })
        .map_err(|error| map_mutation_error(error, MutationKind::Create))?;
    let response: CheckedProfileResponse = serde_json::from_value(value).map_err(|error| {
        checked_profile_response_error(format!("invalid checked-server response: {error}"))
    })?;
    CheckedProfileDto::from_response(&expected_profile, &expected_probe, response)
        .map_err(|error| checked_profile_response_error(error.message))
}

fn pending_operations(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
) -> Result<Vec<PendingOperationSummary>, AgentError> {
    require_transport_profile(transport, profile)?;
    let value = transport
        .call(Operation::ListPendingOperations {
            profile: profile.to_owned(),
        })
        .map_err(AgentError::from_desktop)?;
    require_response_row_cap(&value, "pending operations")?;
    let rows: Vec<PendingOperationResponse> =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    Ok(rows.into_iter().map(Into::into).collect())
}

fn validated_pending_dtos(
    pending: &[PendingOperationSummary],
) -> Result<Vec<PendingOperationDto>, AgentError> {
    if pending.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response(
            "The agent returned too many pending operations.",
        ));
    }
    let projected = pending
        .iter()
        .cloned()
        .map(PendingOperationDto::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let mut identities = std::collections::HashSet::new();
    if projected.iter().any(|operation| {
        !identities.insert((
            operation.kind,
            operation.alias.as_str(),
            operation.target.as_deref(),
        ))
    }) {
        return Err(invalid_response(
            "The agent returned a duplicate pending-operation identity.",
        ));
    }
    Ok(projected)
}

fn require_one_pending_operation(
    pending: &[PendingOperationSummary],
    kind: PendingOperationKind,
    alias: &str,
    target: Option<&str>,
) -> Result<(), AgentError> {
    let matches = pending
        .iter()
        .filter(|operation| {
            operation.kind == kind
                && operation.alias == alias
                && operation.target.as_deref() == target
        })
        .count();
    if matches == 1 {
        Ok(())
    } else if matches == 0 {
        Err(AgentError::new(
            "pending-operation-not-found",
            "Refresh first-run progress before resuming this operation.",
            false,
        ))
    } else {
        Err(invalid_response(
            "The agent returned a duplicate pending-operation identity.",
        ))
    }
}

fn execute_pending_operation(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
    pending_kind: PendingOperationKind,
    pending_alias: &str,
    pending_target: Option<&str>,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    let pending = pending_operations(transport, profile)?;
    validated_pending_dtos(&pending)?;
    require_one_pending_operation(&pending, pending_kind, pending_alias, pending_target)?;
    transport
        .call(operation)
        .map_err(|error| map_mutation_error(error, MutationKind::Resume))
}

async fn apply_pending_operation_value(
    state: &AppState,
    profile: String,
    pending_kind: PendingOperationKind,
    pending_alias: String,
    pending_target: Option<String>,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_pending_operation(
            transport.as_ref(),
            &profile,
            pending_kind,
            &pending_alias,
            pending_target.as_deref(),
            operation,
        )
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The first-run resume worker stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

async fn read_profile_operation_value(
    state: &AppState,
    profile: String,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        execute_read_profile_operation(transport.as_ref(), &profile, operation)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the first-run read did not finish: {error}")))?
}

fn execute_read_profile_operation(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    require_transport_profile(transport, profile)?;
    transport.call(operation).map_err(AgentError::from_desktop)
}

async fn apply_profile_operation_value(
    state: &AppState,
    profile: String,
    operation: Operation,
    kind: MutationKind,
) -> Result<serde_json::Value, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_profile_operation(transport.as_ref(), &profile, operation, kind)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The first-run worker stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

async fn apply_profile_operation_with_profile(
    state: &AppState,
    profile: String,
    operation: Operation,
    kind: MutationKind,
) -> Result<(serde_json::Value, ProfileSummary), AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<_, AgentError> {
        let configured = transport_profile(transport.as_ref(), &profile)?;
        let value = transport
            .call(operation)
            .map_err(|error| map_mutation_error(error, kind))?;
        Ok((value, configured))
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The server-check worker stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

fn execute_profile_operation(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
    operation: Operation,
    kind: MutationKind,
) -> Result<serde_json::Value, AgentError> {
    require_transport_profile(transport, profile)?;
    transport
        .call(operation)
        .map_err(|error| map_mutation_error(error, kind))
}

fn file_create_header(
    store: &CatalogStoreRef,
    path: &str,
    total_length: u64,
    read_role: Option<&str>,
    write_role: Option<&str>,
) -> Result<KvUploadHeader, AgentError> {
    let (read_role, write_role) = create_item_roles(store, read_role, write_role)?;
    let mut header =
        foks_desktop::create_kv_file_upload(store, path, total_length).map_err(invalid_request)?;
    header.read_role = read_role;
    header.write_role = write_role;
    Ok(header)
}

fn file_edit_header(item: &CatalogItem, total_length: u64) -> Result<KvUploadHeader, AgentError> {
    foks_desktop::edit_kv_file_upload(item, total_length).map_err(invalid_request)
}

fn open_regular_file(path: &Path) -> Result<(File, u64), AgentError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| {
        AgentError::new(
            "upload-source",
            format!("FOKS could not open the selected file: {error}"),
            false,
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        AgentError::new(
            "upload-source",
            format!("FOKS could not inspect the selected file: {error}"),
            false,
        )
    })?;
    if !metadata.is_file() {
        return Err(AgentError::new(
            "upload-source",
            "Only a regular file can be imported.",
            false,
        ));
    }
    Ok((file, metadata.len()))
}

struct SourceReader {
    file: File,
    read_error: Option<String>,
}

impl std::io::Read for SourceReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.file.read(buffer) {
            Ok(count) => Ok(count),
            Err(error) => {
                self.read_error = Some(error.to_string());
                Err(error)
            }
        }
    }
}

fn upload_file(
    transport: &dyn foks_desktop::AgentTransport,
    mut header: KvUploadHeader,
    source: &Path,
    kind: MutationKind,
) -> Result<(), AgentError> {
    let (file, total_length) = open_regular_file(source)?;
    header.total_length = total_length;
    let mut reader = SourceReader {
        file,
        read_error: None,
    };
    let result = transport.put_kv_stream(header, &mut reader);
    if let Some(detail) = reader.read_error {
        return Err(AgentError::new(
            "upload-source",
            format!("The selected file could not be read: {detail}"),
            false,
        ));
    }
    if matches!(result, Err(foks_desktop::AgentError::Transport(_))) {
        // A lost socket can stop the transport before it asks the reader for
        // the remaining bytes, so `bytes_read != total_length` is not evidence
        // that the source changed. Only metadata from this same opened handle
        // can establish a length change; otherwise preserve the transport
        // classification so the full-window agent-loss stop appears.
        if reader
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() != total_length)
        {
            return Err(AgentError::new(
                "upload-source-changed",
                "The selected file changed while FOKS was reading it. Drop or choose it again.",
                false,
            ));
        }
    }
    result
        .map(|_| ())
        .map_err(|error| map_mutation_error(error, kind))
}

async fn apply_file_upload(
    state: &AppState,
    header: KvUploadHeader,
    source: PathBuf,
    kind: MutationKind,
) -> Result<MutationDto, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        upload_file(transport.as_ref(), header, &source, kind)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The file-import worker stopped before reporting its outcome: {error}"),
        )
    })?;
    if result.as_ref().is_err_and(|error| error.ambiguous) {
        state
            .mutation_requires_refresh
            .store(true, Ordering::Release);
    }
    result.map(|()| MutationDto { applied: true })
}

async fn load_catalog(state: &AppState, include_items: bool) -> Result<CatalogDto, AgentError> {
    let (generation, token) = state.begin_catalog_load_checked()?;
    let transport = state.agent.transport();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        if include_items {
            foks_desktop::load_catalog_cancellable(transport, token)
        } else {
            foks_desktop::load_stores_cancellable(transport, token)
        }
        .map_err(AgentError::from_desktop)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the catalog load did not finish: {error}")))??;
    let dto = CatalogDto::from_snapshot(&snapshot)?;
    if include_items {
        state.accept_catalog(generation, snapshot);
    }
    Ok(dto)
}

#[tauri::command]
pub async fn agent_status(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AgentStatusDto, AgentError> {
    require_main_window(&webview)?;
    let value = success_value(
        state
            .agent
            .call(foks_agent_proto::Operation::AgentStatus)
            .await?,
    )?;
    serde_json::from_value::<foks_agent_proto::AgentStatus>(value)
        .map(AgentStatusDto::from)
        .map_err(|error| AgentError::unknown(format!("the agent status did not parse: {error}")))
}

#[tauri::command]
pub async fn retry_agent_connection(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AgentStatusDto, AgentError> {
    require_main_window(&webview)?;
    let agent = Arc::clone(&state.agent);
    let response = tauri::async_runtime::spawn_blocking(move || agent.ensure_started_blocking())
        .await
        .map_err(|error| {
            AgentError::unknown(format!("the agent restart did not finish: {error}"))
        })??;
    let value = success_value(response)?;
    state.invalidate_catalog();
    serde_json::from_value::<foks_agent_proto::AgentStatus>(value)
        .map(AgentStatusDto::from)
        .map_err(|error| AgentError::unknown(format!("the agent status did not parse: {error}")))
}

#[tauri::command]
pub async fn initialize_client_state(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AgentStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let value = apply_operation_value(
        &state,
        Operation::InitializeState {
            backend: CredentialBackend::Native,
        },
        MutationKind::Create,
    )
    .await?;
    let response: InitializationResponse = serde_json::from_value(value).map_err(|error| {
        ambiguous_mutation_response(&state, format!("invalid initialization response: {error}"))
    })?;
    if response.backend != CredentialBackend::Native {
        return Err(ambiguous_mutation_response(
            &state,
            "The agent initialized a different credential backend.",
        ));
    }
    let value = success_value(state.agent.call(Operation::AgentStatus).await?)?;
    let status: foks_agent_proto::AgentStatus = serde_json::from_value(value).map_err(|error| {
        ambiguous_mutation_response(&state, format!("invalid initialized status: {error}"))
    })?;
    if status != foks_agent_proto::AgentStatus::Ready {
        return Err(ambiguous_mutation_response(
            &state,
            "The agent did not become ready after initialization.",
        ));
    }
    Ok(status.into())
}

#[tauri::command]
pub async fn check_and_add_profile(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile_name: String,
    probe: String,
) -> Result<CheckedProfileDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile_name = bounded_local_name(
        &profile_name,
        "Use 1–64 letters, numbers, hyphens, or underscores for the server profile.",
    )?;
    let probe = bounded_field(
        &probe,
        2 * 1024,
        "Enter a server address of at most 2,048 bytes.",
    )?;
    if !valid_probe_target(&probe) {
        return Err(invalid_request(
            "Enter a valid DNS name, IP address, or host and port.",
        ));
    }
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        check_existing_or_add_profile(transport.as_ref(), profile_name, probe)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            &state,
            format!("The server-check worker stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(checked) => Ok(checked),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn add_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile_name: String,
    probe: String,
) -> Result<AddedServerDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile_name = bounded_local_name(
        &profile_name,
        "Use 1–64 letters, numbers, hyphens, or underscores for the server profile.",
    )?;
    let probe = bounded_field(
        &probe,
        2 * 1024,
        "Enter a server address of at most 2,048 bytes.",
    )?;
    if !valid_probe_target(&probe) {
        return Err(invalid_request(
            "Enter a valid DNS name, IP address, or host and port.",
        ));
    }
    let expected_profile = profile_name.clone();
    let expected_probe = probe.clone();
    let value = apply_operation_value(
        &state,
        Operation::AddProfile {
            name: profile_name,
            probe,
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        },
        MutationKind::Create,
    )
    .await?;
    added_server_response(value, &expected_profile, &expected_probe)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn forget_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    confirmation: String,
) -> Result<ForgottenServerDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    exact_profile_confirmation(&profile, &confirmation, "forget")?;
    state.ensure_profile_available(&profile)?;
    let expected = profile.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::RemoveProfile { name: profile },
        MutationKind::Guarded,
    )
    .await?;
    forgotten_server_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn describe_server_status(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<ServerStatusSnapshotDto, AgentError> {
    require_main_window(&webview)?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let expected = profile.clone();
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        let configured = transport_profile(transport.as_ref(), &profile)?;
        let value = transport
            .call(Operation::DescribeServerStatus {
                profile: profile.clone(),
            })
            .map_err(AgentError::from_desktop)?;
        let lease_required = !matches!(configured.protocol, ProfileProtocolSummary::V019);
        server_status_response(value, &expected, &configured.probe, lease_required)
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!("the server status load did not finish: {error}"))
    })?
}

#[tauri::command]
pub async fn check_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<CheckedServerDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let expected = profile.clone();
    let (value, configured) = apply_profile_operation_with_profile(
        &state,
        profile.clone(),
        Operation::Probe { profile },
        MutationKind::Guarded,
    )
    .await?;
    let checked = checked_server_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))?;
    let expected_lookup = normalized_probe_hostname(&configured.probe).ok_or_else(|| {
        ambiguous_mutation_response(&state, "The selected server profile has an invalid probe.")
    })?;
    if checked.lookup_name != expected_lookup {
        return Err(ambiguous_mutation_response(
            &state,
            "The server check returned facts for a different lookup name.",
        ));
    }
    Ok(checked)
}

#[tauri::command]
pub async fn list_account_devices(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<Vec<DeviceDto>, AgentError> {
    require_main_window(&webview)?;
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let account = state.selected_account(&account_store_id)?;
    let transport = state.agent.transport();
    let devices = tauri::async_runtime::spawn_blocking(move || {
        load_account_devices(transport.as_ref(), &account)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the device load did not finish: {error}")))??;
    state.retain_devices(generation, account_store_id, &devices)?;
    Ok(devices)
}

#[tauri::command]
pub async fn list_backup_enrollments(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<Vec<BackupEnrollmentDto>, AgentError> {
    require_main_window(&webview)?;
    let account = state.selected_account(&account_store_id)?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        load_backup_enrollments(transport.as_ref(), &account)
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!(
            "the backup enrollment load did not finish: {error}"
        ))
    })?
}

async fn yubi_enrollments_for_profile(
    state: &AppState,
    profile: String,
) -> Result<Vec<YubiEnrollmentDto>, AgentError> {
    let value = read_profile_operation_value(
        state,
        profile.clone(),
        Operation::ListYubiAccounts { profile },
    )
    .await?;
    yubi_enrollment_dtos(value)
}

async fn require_complete_yubi(
    state: &AppState,
    profile: String,
    alias: &str,
) -> Result<(), AgentError> {
    let enrollments = yubi_enrollments_for_profile(state, profile).await?;
    require_yubi_enrollment(&enrollments, alias, "complete")
}

#[tauri::command]
pub async fn list_yubi_cards(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<Vec<YubiCardDto>, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ListYubiCards { profile },
    )
    .await?;
    yubi_card_dtos(value)
}

#[tauri::command]
pub async fn list_yubi_accounts(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<Vec<YubiEnrollmentDto>, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    yubi_enrollments_for_profile(&state, profile).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_yubi_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    username: String,
    device_name: String,
    email: String,
    invite: String,
    passphrase: Option<String>,
    passphrase_confirmation: Option<String>,
    card_serial: u32,
    signing_slot: u8,
    pq_slot: u8,
    pin: String,
    puk: String,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<YubiAccountDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Enter a valid local security-key alias.")?;
    let username = bounded_field(&username, 256, "Enter a username of at most 256 bytes.")?;
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let email = optional_bounded_field(&email, 320, "Enter an email of at most 320 bytes.")?;
    let invite = Zeroizing::new(invite);
    let invite = Zeroizing::new(optional_bounded_field(
        invite.as_str(),
        MAXIMUM_INVITE_BYTES,
        "The invite must be one line of at most 4,096 bytes.",
    )?);
    let passphrase = optional_confirmed_passphrase(passphrase, passphrase_confirmation)?;
    let (signing_slot, pq_slot) = yubi_slots(signing_slot, pq_slot)?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    let retry_configuration = yubi_retry_configuration(puk, pin_attempts, puk_attempts)?;
    let enrollments = yubi_enrollments_for_profile(&state, profile.clone()).await?;
    if enrollments.iter().any(|entry| entry.alias == alias) {
        return Err(AgentError::new(
            "already-exists",
            "A security-key enrollment already uses this local alias.",
            false,
        ));
    }
    let cards_value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ListYubiCards {
            profile: profile.clone(),
        },
    )
    .await?;
    if !yubi_card_dtos(cards_value)?
        .iter()
        .any(|card| card.serial == card_serial)
    {
        return Err(AgentError::new(
            "security-key-not-connected",
            "Refresh connected keys and choose one that is still plugged in.",
            false,
        ));
    }
    let expected = alias.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::CreateYubiAccount {
            profile,
            alias,
            username,
            device_name,
            email,
            invite: SecretString::new(invite.as_str()),
            passphrase,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration: Some(retry_configuration),
        },
        MutationKind::Create,
    )
    .await?;
    yubi_account_response(value, &expected, None)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_yubi_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
) -> Result<YubiAccountDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid pending security-key alias.")?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    let enrollments = yubi_enrollments_for_profile(&state, profile.clone()).await?;
    require_yubi_enrollment(&enrollments, &alias, "pending")?;
    let expected = alias.clone();
    let value = apply_pending_operation_value(
        &state,
        profile.clone(),
        PendingOperationKind::YubiEnrollment,
        alias.clone(),
        None,
        Operation::ResumeYubiAccount {
            profile,
            alias,
            pin,
        },
    )
    .await?;
    yubi_account_response(value, &expected, None)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn provision_yubi_device(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    target_alias: String,
    device_name: String,
    card_serial: u32,
    signing_slot: u8,
    pq_slot: u8,
    pin: String,
    puk: String,
    pin_attempts: u8,
    puk_attempts: u8,
) -> Result<YubiAccountDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account_dto = state.selected_account_dto(&account_store_id)?;
    let account = foks_agent_proto::AccountStoreRef {
        profile: account_dto.profile.clone(),
        account_alias: account_dto.alias.clone(),
    };
    let target_alias =
        bounded_local_name(&target_alias, "Enter a valid local security-key alias.")?;
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let (signing_slot, pq_slot) = yubi_slots(signing_slot, pq_slot)?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    let retry_configuration = yubi_retry_configuration(puk, pin_attempts, puk_attempts)?;
    let enrollments = yubi_enrollments_for_profile(&state, account.profile.clone()).await?;
    if enrollments.iter().any(|entry| entry.alias == target_alias) {
        return Err(AgentError::new(
            "already-exists",
            "A security-key enrollment already uses this local alias.",
            false,
        ));
    }
    let cards_value = read_profile_operation_value(
        &state,
        account.profile.clone(),
        Operation::ListYubiCards {
            profile: account.profile.clone(),
        },
    )
    .await?;
    if !yubi_card_dtos(cards_value)?
        .iter()
        .any(|card| card.serial == card_serial)
    {
        return Err(AgentError::new(
            "security-key-not-connected",
            "Refresh connected keys and choose one that is still plugged in.",
            false,
        ));
    }
    let expected = target_alias.clone();
    let profile = account.profile;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ProvisionYubiDevice {
            profile,
            source_alias: account.account_alias,
            target_alias,
            device_name,
            serial: positive_recovery_serial()?,
            card_serial,
            signing_slot,
            pq_slot,
            pin,
            retry_configuration: Some(retry_configuration),
        },
        MutationKind::Create,
    )
    .await?;
    yubi_account_response(value, &expected, Some(&account_dto.username))
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn sync_yubi_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    with_federation: bool,
) -> Result<YubiSyncDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let expected_profile = profile.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::SyncYubiAccount {
            profile,
            alias,
            pin,
            with_federation,
        },
        MutationKind::Resume,
    )
    .await?;
    yubi_sync_response(value, &expected_profile, with_federation)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn yubi_pin_status(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
) -> Result<YubiPinStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::YubiPinStatus { profile, alias },
    )
    .await?;
    yubi_pin_status_response(value)
}

#[tauri::command]
pub async fn change_yubi_pin(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    old_pin: String,
    new_pin: String,
) -> Result<YubiPinStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let old_pin = bounded_secret(old_pin, 128, "Use one nonempty current PIN.")?;
    let new_pin = bounded_secret(
        new_pin,
        128,
        "Use one nonempty new PIN of at most 128 bytes.",
    )?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ChangeYubiPin {
            profile,
            alias,
            old_pin,
            new_pin,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_pin_status_response(value)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn yubi_passphrase_operation(
    state: &AppState,
    profile: String,
    alias: String,
    pin: SecretString,
    passphrase: SecretString,
    mode: &'static str,
) -> Result<PassphraseReportDto, AgentError> {
    require_complete_yubi(state, profile.clone(), &alias).await?;
    let operation = match mode {
        "set" => Operation::SetYubiPassphrase {
            profile: profile.clone(),
            alias,
            pin,
            passphrase,
        },
        "change" => Operation::ChangeYubiPassphrase {
            profile: profile.clone(),
            alias,
            pin,
            passphrase,
        },
        "verify" => Operation::VerifyYubiPassphrase {
            profile: profile.clone(),
            alias,
            pin,
            passphrase,
        },
        _ => return Err(invalid_request("Unknown security-key passphrase action.")),
    };
    let value =
        apply_profile_operation_value(state, profile, operation, MutationKind::Guarded).await?;
    passphrase_report_response(value)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn set_yubi_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    yubi_passphrase_operation(&state, profile, alias, pin, passphrase, "set").await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn change_yubi_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    yubi_passphrase_operation(&state, profile, alias, pin, passphrase, "change").await
}

#[tauri::command]
pub async fn verify_yubi_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
    passphrase: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    let passphrase = bounded_secret(
        passphrase,
        MAXIMUM_PASSPHRASE_BYTES,
        "Use one nonempty passphrase of at most 1,024 bytes.",
    )?;
    yubi_passphrase_operation(&state, profile, alias, pin, passphrase, "verify").await
}

#[tauri::command]
pub async fn change_yubi_puk(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    old_puk: String,
    new_puk: String,
) -> Result<YubiChangedDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let old_puk = bounded_secret(old_puk, 128, "Use one nonempty current unlock code.")?;
    let new_puk = bounded_secret(
        new_puk,
        128,
        "Use one nonempty new unlock code of at most 128 bytes.",
    )?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let expected = alias.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ChangeYubiPuk {
            profile,
            alias,
            old_puk,
            new_puk,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_changed_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn unblock_yubi_pin(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    puk: String,
    new_pin: String,
) -> Result<YubiPinStatusDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let puk = bounded_secret(puk, 128, "Use one nonempty unlock code.")?;
    let new_pin = bounded_secret(
        new_pin,
        128,
        "Use one nonempty new PIN of at most 128 bytes.",
    )?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::UnblockYubiPin {
            profile,
            alias,
            puk,
            new_pin,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_pin_status_response(value)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn yubi_lifecycle_operation(
    state: &AppState,
    profile: String,
    alias: String,
    operation: Operation,
    kind: MutationKind,
) -> Result<YubiLifecycleDto, AgentError> {
    require_complete_yubi(state, profile.clone(), &alias).await?;
    let value = apply_profile_operation_value(state, profile, operation, kind).await?;
    yubi_lifecycle_response(value, &alias)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[tauri::command]
pub async fn rotate_yubi_management_key(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
) -> Result<YubiLifecycleDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    yubi_lifecycle_operation(
        &state,
        profile.clone(),
        alias.clone(),
        Operation::RotateYubiManagementKey {
            profile,
            alias,
            pin,
        },
        MutationKind::Guarded,
    )
    .await
}

#[tauri::command]
pub async fn resume_yubi_management_key(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: Option<String>,
) -> Result<YubiLifecycleDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = pin
        .map(|pin| bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes."))
        .transpose()?;
    yubi_lifecycle_operation(
        &state,
        profile.clone(),
        alias.clone(),
        Operation::ResumeYubiManagementKey {
            profile,
            alias,
            pin,
        },
        MutationKind::Resume,
    )
    .await
}

#[tauri::command]
pub async fn recover_yubi_management_key(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    yubi_alias: String,
) -> Result<YubiLifecycleDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let yubi_alias = bounded_local_name(&yubi_alias, "Choose a valid security-key alias.")?;
    let profile = account.profile;
    let operation = Operation::RecoverYubiManagementKey {
        profile: profile.clone(),
        yubi_alias: yubi_alias.clone(),
        software_alias: account.account_alias,
    };
    yubi_lifecycle_operation(&state, profile, yubi_alias, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn recover_yubi_subkey(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    pin: String,
) -> Result<YubiSubkeyRecoveryDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid security-key alias.")?;
    let pin = bounded_secret(pin, 128, "Use one nonempty PIN of at most 128 bytes.")?;
    require_complete_yubi(&state, profile.clone(), &alias).await?;
    let expected = alias.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::RecoverYubiSubkey {
            profile,
            alias,
            pin,
        },
        MutationKind::Resume,
    )
    .await?;
    yubi_subkey_recovery_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn revoke_yubi_device(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    yubi_alias: String,
    confirmation: String,
) -> Result<YubiRevocationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let yubi_alias = bounded_local_name(&yubi_alias, "Choose a valid security-key alias.")?;
    if confirmation != yubi_alias {
        return Err(invalid_request(
            "Type the exact security-key alias to revoke it.",
        ));
    }
    require_complete_yubi(&state, account.profile.clone(), &yubi_alias).await?;
    let transport = state.agent.transport();
    let signer_account = account.clone();
    let devices = tauri::async_runtime::spawn_blocking(move || {
        load_account_devices(transport.as_ref(), &signer_account)
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!(
            "the security-key revocation preflight did not finish: {error}"
        ))
    })??;
    require_software_revocation_signer(&devices)?;
    let expected = yubi_alias.clone();
    let profile = account.profile;
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::RevokeYubiDevice {
            profile,
            yubi_alias,
            software_alias: account.account_alias,
        },
        MutationKind::Guarded,
    )
    .await?;
    yubi_revocation_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn remove_account_device(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    device_id: String,
) -> Result<DeviceRemovalDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_device_target(&account_store_id, &device_id)?;
    let expected = device_id.clone();
    let value = apply_profile_operation_value(
        &state,
        account.profile.clone(),
        Operation::RemoveDevice {
            profile: account.profile,
            signer_alias: account.account_alias,
            device_id,
        },
        MutationKind::Guarded,
    )
    .await?;
    device_removal_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn list_pending_operations(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<Vec<PendingOperationDto>, AgentError> {
    require_main_window(&webview)?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile name.")?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        let pending = pending_operations(transport.as_ref(), &profile)?;
        validated_pending_dtos(&pending)
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!(
            "the pending-operation load did not finish: {error}"
        ))
    })?
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_first_run_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    username: String,
    device_name: String,
    email: String,
    invite: String,
    passphrase: Option<String>,
    passphrase_confirmation: Option<String>,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Enter a valid local account alias.")?;
    let username = bounded_field(&username, 256, "Enter a FOKS username.")?;
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let email = optional_bounded_field(&email, 320, "Enter a valid email value.")?;
    let invite = Zeroizing::new(invite);
    let invite = Zeroizing::new(optional_bounded_field(
        invite.as_str(),
        MAXIMUM_INVITE_BYTES,
        "The invite must be one line of at most 4,096 bytes.",
    )?);
    let passphrase = optional_confirmed_passphrase(passphrase, passphrase_confirmation)?;
    let operation = Operation::CreateAccount {
        profile: profile.clone(),
        alias,
        username,
        device_name,
        email,
        invite: SecretString::new(invite.as_str()),
        passphrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Create).await?;
    account_sync_response(value).map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_first_run_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid pending account alias.")?;
    let operation = Operation::ResumeAccount {
        profile: profile.clone(),
        alias: alias.clone(),
    };
    let value = apply_pending_operation_value(
        &state,
        profile,
        PendingOperationKind::AccountSignup,
        alias,
        None,
        operation,
    )
    .await?;
    account_sync_response(value).map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn set_first_run_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    alias: String,
    passphrase: String,
    confirmation: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let alias = bounded_local_name(&alias, "Choose a valid account alias.")?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    let operation = Operation::SetPassphrase {
        profile: profile.clone(),
        alias,
        passphrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Guarded).await?;
    passphrase_response(value).map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn prepare_owner_backup(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    backup_alias: String,
) -> Result<BackupPhraseDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let account_alias = bounded_local_name(&account_alias, "Choose a valid account alias.")?;
    let backup_alias = bounded_local_name(&backup_alias, "Enter a valid backup alias.")?;
    let expected = backup_alias.clone();
    let operation = Operation::PrepareOwnerBackup {
        profile: profile.clone(),
        account_alias,
        backup_alias,
    };
    let value = read_profile_operation_value(&state, profile, operation).await?;
    backup_phrase_response(value, &expected)
}

#[tauri::command]
pub async fn commit_owner_backup(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    backup_alias: String,
    phrase: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let account_alias = bounded_local_name(&account_alias, "Choose a valid account alias.")?;
    let backup_alias = bounded_local_name(&backup_alias, "Choose a valid backup alias.")?;
    let expected_account = account_alias.clone();
    let expected_backup = backup_alias.clone();
    let phrase = bounded_secret(
        phrase,
        MAXIMUM_RECOVERY_PHRASE_BYTES,
        "The backup phrase must be one line of at most 4,096 bytes.",
    )?;
    let operation = Operation::CommitOwnerBackup {
        profile: profile.clone(),
        account_alias,
        backup_alias,
        phrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Guarded).await?;
    backup_commit_response(value, &expected_account, &expected_backup)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn recover_owner_account(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
    phrase: String,
    device_name: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let target_alias = bounded_local_name(&target_alias, "Enter a valid local account alias.")?;
    let expected_alias = target_alias.clone();
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let phrase = bounded_secret(
        phrase,
        MAXIMUM_RECOVERY_PHRASE_BYTES,
        "The recovery phrase must be one line of at most 4,096 bytes.",
    )?;
    let serial = positive_recovery_serial()?;
    let operation = Operation::RecoverOwnerAccount {
        profile: profile.clone(),
        target_alias,
        phrase,
        device_name,
        serial,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Create).await?;
    recovery_response(value, &expected_alias)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_owner_recovery(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
    phrase: String,
    device_name: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let target_alias = bounded_local_name(&target_alias, "Choose a valid pending recovery alias.")?;
    let expected_alias = target_alias.clone();
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let phrase = bounded_secret(
        phrase,
        MAXIMUM_RECOVERY_PHRASE_BYTES,
        "The recovery phrase must be one line of at most 4,096 bytes.",
    )?;
    let operation = Operation::ResumeOwnerRecovery {
        profile: profile.clone(),
        target_alias: target_alias.clone(),
        phrase,
        device_name,
    };
    let value = apply_pending_operation_value(
        &state,
        profile,
        PendingOperationKind::AccountRecovery,
        target_alias,
        None,
        operation,
    )
    .await?;
    recovery_response(value, &expected_alias)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn pairing_offer_operation(
    state: &AppState,
    account: foks_agent_proto::AccountStoreRef,
    resume: bool,
) -> Result<PairingOfferDto, AgentError> {
    let expected = account.account_alias.clone();
    let operation = if resume {
        Operation::RepublishDevicePairing {
            profile: account.profile.clone(),
            account_alias: account.account_alias.clone(),
        }
    } else {
        Operation::StartDevicePairing {
            profile: account.profile.clone(),
            account_alias: account.account_alias.clone(),
        }
    };
    let value = if resume {
        apply_pending_operation_value(
            state,
            account.profile,
            PendingOperationKind::PairingOffer,
            account.account_alias,
            None,
            operation,
        )
        .await?
    } else {
        apply_profile_operation_value(state, account.profile, operation, MutationKind::Create)
            .await?
    };
    pairing_offer_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[tauri::command]
pub async fn start_device_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<PairingOfferDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    pairing_offer_operation(&state, account, false).await
}

#[tauri::command]
pub async fn resume_device_pairing_offer(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<PairingOfferDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    pairing_offer_operation(&state, account, true).await
}

#[tauri::command]
pub async fn finish_device_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let expected = account.account_alias.clone();
    let operation = Operation::FinishDevicePairing {
        profile: account.profile.clone(),
        account_alias: account.account_alias.clone(),
    };
    let value = apply_pending_operation_value(
        &state,
        account.profile,
        PendingOperationKind::PairingOffer,
        account.account_alias,
        None,
        operation,
    )
    .await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn accept_device_pairing(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
    device_name: String,
    phrase: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let target_alias = bounded_local_name(&target_alias, "Enter a valid local account alias.")?;
    let expected = target_alias.clone();
    let device_name = bounded_field(&device_name, 256, "Enter a device name.")?;
    let phrase = pairing_phrase(phrase)?;
    let serial = positive_recovery_serial()?;
    let operation = Operation::AcceptDevicePairing {
        profile: profile.clone(),
        target_alias,
        device_name,
        serial,
        phrase,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Create).await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn resume_device_pairing_acceptance(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    target_alias: String,
) -> Result<DeviceProvisionDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let target_alias = bounded_local_name(
        &target_alias,
        "Choose a valid pending device-pairing alias.",
    )?;
    let expected = target_alias.clone();
    let operation = Operation::ResumeDevicePairingAcceptance {
        profile: profile.clone(),
        target_alias: target_alias.clone(),
    };
    let value = apply_pending_operation_value(
        &state,
        profile,
        PendingOperationKind::PairingAcceptance,
        target_alias,
        None,
        operation,
    )
    .await?;
    device_provision_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

async fn account_passphrase_mutation(
    state: &AppState,
    account: foks_agent_proto::AccountStoreRef,
    passphrase: SecretString,
    change: bool,
) -> Result<PassphraseReportDto, AgentError> {
    let operation = if change {
        Operation::ChangePassphrase {
            profile: account.profile.clone(),
            alias: account.account_alias,
            passphrase,
        }
    } else {
        Operation::SetPassphrase {
            profile: account.profile.clone(),
            alias: account.account_alias,
            passphrase,
        }
    };
    let value =
        apply_profile_operation_value(state, account.profile, operation, MutationKind::Guarded)
            .await?;
    passphrase_report_response(value)
        .map_err(|error| ambiguous_mutation_response(state, error.message))
}

#[tauri::command]
pub async fn set_account_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    account_passphrase_mutation(&state, account, passphrase, false).await
}

#[tauri::command]
pub async fn change_account_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    passphrase: String,
    confirmation: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let passphrase = confirmed_passphrase(passphrase, confirmation)?;
    account_passphrase_mutation(&state, account, passphrase, true).await
}

#[tauri::command]
pub async fn verify_account_passphrase(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    passphrase: String,
) -> Result<PassphraseReportDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let account = state.selected_account(&account_store_id)?;
    let passphrase = bounded_secret(
        passphrase,
        MAXIMUM_PASSPHRASE_BYTES,
        "Use one nonempty passphrase of at most 1,024 bytes.",
    )?;
    let operation = Operation::VerifyPassphrase {
        profile: account.profile.clone(),
        alias: account.account_alias,
        passphrase,
    };
    let value = read_profile_operation_value(&state, account.profile, operation).await?;
    passphrase_report_response(value)
}

#[tauri::command]
pub async fn describe_reset(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<ResetPreviewDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    // Serializes preview issuance with changes so its one-use token describes
    // one stable state. Losing an unused token is harmless and does not force
    // catalog reconciliation.
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let expected = profile.clone();
    let value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::DescribeResetHardState { profile },
    )
    .await?;
    reset_preview_response(value, &expected)
}

#[tauri::command]
pub async fn reset_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    confirmation: String,
    token: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    exact_profile_confirmation(&profile, &confirmation, "reset")?;
    let token = bounded_secret(
        token,
        1024,
        "The reset preview authorization is missing or invalid. Preview the reset again.",
    )?;
    let expected = profile.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ResetHardState { profile, token },
        MutationKind::Guarded,
    )
    .await?;
    reset_result_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn discover_groups(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
) -> Result<GroupDiscoveryDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Choose a valid server profile.")?;
    let account_alias = bounded_local_name(&account_alias, "Choose a valid account alias.")?;
    let expected = account_alias.clone();
    let operation = Operation::DiscoverTeams {
        profile: profile.clone(),
        account_alias,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Resume).await?;
    require_nested_response_row_cap(&value, "teams", "discovered groups")
        .map_err(|error| ambiguous_mutation_response(&state, error.message))?;
    let response: GroupDiscoveryResponse = serde_json::from_value(value).map_err(|error| {
        ambiguous_mutation_response(&state, format!("invalid discovery response: {error}"))
    })?;
    GroupDiscoveryDto::from_response(&expected, response)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub fn app_info(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<AppInfo, AgentError> {
    require_main_window(&webview)?;
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        agent_socket: state.agent.socket().display().to_string(),
        managed_profile: std::env::var("FOKS_MANAGED_PROFILE")
            .ok()
            .filter(|profile| {
                !profile.is_empty()
                    && profile.len() <= 64
                    && profile
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            }),
    })
}

#[tauri::command]
pub async fn list_stores(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<CatalogDto, AgentError> {
    require_main_window(&webview)?;
    load_catalog(&state, false).await
}

#[tauri::command]
pub async fn list_catalog(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<CatalogDto, AgentError> {
    require_main_window(&webview)?;
    load_catalog(&state, true).await
}

#[tauri::command]
pub async fn list_servers(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Vec<ServerDto>, AgentError> {
    require_main_window(&webview)?;
    let catalog = state
        .catalog
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::ListProfiles)
            .map_err(AgentError::from_desktop)?;
        let profiles = profile_summaries(value)?;
        Ok(profiles
            .into_iter()
            .map(|profile| {
                let accounts = catalog
                    .as_ref()
                    .map(|catalog| {
                        catalog
                            .stores
                            .iter()
                            .filter_map(|store| match store {
                                CatalogStoreSummary::Account { store }
                                    if store.profile == profile.name =>
                                {
                                    Some(store.account_alias.clone())
                                }
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let blocked = catalog
                    .as_ref()
                    .is_some_and(|catalog| catalog.profile_blocked(&profile.name));
                ServerDto {
                    id: profile.name.clone(),
                    name: profile.name,
                    label: None,
                    host_id: None,
                    chain: None,
                    epoch: None,
                    lease: None,
                    accounts,
                    state: if blocked { "blocked" } else { "never-probed" },
                }
            })
            .collect())
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the server load did not finish: {error}")))?
}

#[tauri::command]
pub async fn list_accounts(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<Vec<AccountDto>, AgentError> {
    require_main_window(&webview)?;
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let catalog = state
        .catalog
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or_else(|| {
            AgentError::new(
                "catalog-required",
                "Refresh the vault before loading account identities.",
                true,
            )
        })?;
    let transport = state.agent.transport();
    let accounts =
        tauri::async_runtime::spawn_blocking(move || load_accounts(transport.as_ref(), &catalog))
            .await
            .map_err(|error| {
                AgentError::unknown(format!("the account load did not finish: {error}"))
            })??;
    state.retain_accounts(generation, &accounts)?;
    Ok(accounts)
}

#[tauri::command]
pub async fn list_parties(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<Vec<PartyDto>, AgentError> {
    require_main_window(&webview)?;
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let (profile, team_alias) = state.selected_team(&store_id)?;
    let transport = state.agent.transport();
    let cache_key = store_id.clone();
    let parties = tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::ListTeamMembers {
                profile,
                team_alias,
            })
            .map_err(AgentError::from_desktop)?;
        require_response_row_cap(&value, "group roster entries")?;
        let members: Vec<MemberResponse> =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        party_dtos(&store_id, members)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the group roster did not finish: {error}")))??;
    state.retain_roster(generation, cache_key, &parties)?;
    Ok(parties)
}

#[tauri::command]
pub async fn list_federation(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<Vec<FederationEntryDto>, AgentError> {
    require_main_window(&webview)?;
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let (profile, team_alias) = state.selected_team(&store_id)?;
    let transport = state.agent.transport();
    let cache_key = store_id.clone();
    let expected_profile = profile.clone();
    let expected_team_alias = team_alias.clone();
    let entries = tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::ListFederatedTeams {
                profile,
                team_alias,
            })
            .map_err(AgentError::from_desktop)?;
        require_response_row_cap(&value, "federation entries")?;
        let memberships: Vec<FederationResponse> =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        federation_dtos(
            &store_id,
            &expected_profile,
            &expected_team_alias,
            memberships,
        )
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!("the federation load did not finish: {error}"))
    })??;
    state.retain_federation(generation, cache_key, &entries)?;
    Ok(entries)
}

#[tauri::command]
pub async fn read_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<ReadItemDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || read_text(transport.as_ref(), &item))
        .await
        .map_err(|error| AgentError::unknown(format!("the item read did not finish: {error}")))?
}

#[tauri::command]
pub async fn copy_item_value(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<CommandAck, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    let transport = state.agent.transport();
    let value = tauri::async_runtime::spawn_blocking(move || {
        read_text(transport.as_ref(), &item).map(|read| read.value)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the item read did not finish: {error}")))??;
    crate::clipboard::copy_with_hygiene(&app, value)
        .map_err(|error| AgentError::new("clipboard", error, true))?;
    Ok(CommandAck { ok: true })
}

#[tauri::command]
pub fn copy_item_path(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<CommandAck, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    crate::clipboard::copy_with_hygiene(&app, Zeroizing::new(item.metadata.path))
        .map_err(|error| AgentError::new("clipboard", error, true))?;
    Ok(CommandAck { ok: true })
}

#[tauri::command]
pub fn copy_text(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    text: String,
) -> Result<CommandAck, AgentError> {
    require_main_window(&webview)?;
    if text.len() > MAXIMUM_CLIPBOARD_TEXT_BYTES {
        return Err(invalid_request("The text is too large to copy."));
    }
    crate::clipboard::copy_with_hygiene(&app, Zeroizing::new(text))
        .map_err(|error| AgentError::new("clipboard", error, true))?;
    Ok(CommandAck { ok: true })
}

#[tauri::command]
pub async fn download_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<DownloadResult, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    let suggested = item
        .metadata
        .path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("download")
        .to_owned();
    let picker_app = app.clone();
    let destination = tauri::async_runtime::spawn_blocking(move || {
        picker_app
            .dialog()
            .file()
            .set_file_name(suggested)
            .blocking_save_file()
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the save dialog did not finish: {error}")))?;
    let Some(destination) = destination else {
        return Ok(DownloadResult { saved: false });
    };
    let destination = destination
        .into_path()
        .map_err(|error| AgentError::new("download-path", error.to_string(), false))?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        download_to_path(transport.as_ref(), &item, &destination)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the download did not finish: {error}")))??;
    Ok(DownloadResult { saved: true })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_text_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    value: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let store = state.selected_create_store(&store_id)?;
    let (read_role, write_role) =
        create_item_roles(&store, read_role.as_deref(), write_role.as_deref())?;
    let mutation = foks_desktop::create_kv_file_mutation(&store, &path, take_text_value(value)?)
        .map_err(invalid_request)?;
    let mutation = set_create_mutation_roles(mutation, read_role, write_role)?;
    apply_kv_mutation(&state, mutation, MutationKind::Create).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_link(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    target: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let store = state.selected_create_store(&store_id)?;
    let (read_role, write_role) =
        create_item_roles(&store, read_role.as_deref(), write_role.as_deref())?;
    let target = Zeroizing::new(target);
    let operation = foks_desktop::create_kv_symlink_operation(&store, &path, target.as_str())
        .map_err(invalid_request)?;
    let operation = set_create_operation_roles(operation, read_role, write_role)?;
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn create_folder(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let store = state.selected_create_store(&store_id)?;
    let (read_role, write_role) =
        create_item_roles(&store, read_role.as_deref(), write_role.as_deref())?;
    let operation =
        foks_desktop::create_kv_directory_operation(&store, &path).map_err(invalid_request)?;
    let operation = set_create_operation_roles(operation, read_role, write_role)?;
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn edit_text_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    value: String,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
    require_text_item(&item)?;
    let mutation = foks_desktop::edit_kv_file_mutation(&item, take_text_value(value)?)
        .map_err(invalid_request)?;
    apply_kv_mutation(&state, mutation, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn remove_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
    let operation = remove_item_operation(&item)?;
    apply_operation(&state, operation, MutationKind::Guarded).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn import_dropped_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    source_path: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let store = state.selected_create_store(&store_id)?;
    // Validate the destination through the desktop builder before consuming
    // the one-use native drop authorization.
    let header = file_create_header(
        &store,
        &path,
        0,
        read_role.as_deref(),
        write_role.as_deref(),
    )?;
    let source_path = Zeroizing::new(source_path);
    let source = state.take_drop_path(source_path.as_str())?;
    apply_file_upload(&state, header, source, MutationKind::Create).await
}

#[tauri::command]
pub async fn pick_and_import_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let store = state.selected_create_store(&store_id)?;
    let header = file_create_header(
        &store,
        &path,
        0,
        read_role.as_deref(),
        write_role.as_deref(),
    )?;
    let picker_app = app.clone();
    let source = tauri::async_runtime::spawn_blocking(move || {
        picker_app.dialog().file().blocking_pick_file()
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the file picker did not finish: {error}")))?;
    let Some(source) = source else {
        return Ok(MutationDto { applied: false });
    };
    let source = source
        .into_path()
        .map_err(|error| AgentError::new("upload-source", error.to_string(), false))?;
    apply_file_upload(&state, header, source, MutationKind::Create).await
}

#[tauri::command]
pub async fn replace_dropped_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    source_path: String,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
    require_file_item(&item)?;
    let header = file_edit_header(&item, 0)?;
    let source_path = Zeroizing::new(source_path);
    let source = state.take_drop_path(source_path.as_str())?;
    apply_file_upload(&state, header, source, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn pick_and_replace_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
    require_file_item(&item)?;
    let header = file_edit_header(&item, 0)?;
    let picker_app = app.clone();
    let source = tauri::async_runtime::spawn_blocking(move || {
        picker_app.dialog().file().blocking_pick_file()
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the file picker did not finish: {error}")))?;
    let Some(source) = source else {
        return Ok(MutationDto { applied: false });
    };
    let source = source
        .into_path()
        .map_err(|error| AgentError::new("upload-source", error.to_string(), false))?;
    apply_file_upload(&state, header, source, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn create_group(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    team_alias: String,
    name: String,
    kind: GroupKindInput,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let account = state.selected_account(&account_store_id)?;
    let operation = create_group_operation(account, &team_alias, &name, kind)?;
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn add_group_member(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
    destination: RoleInput,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let team = state.selected_active_team_for_mutation(&store_id)?;
    let operation = add_group_member_operation(team, &username, destination)?;
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn resume_group_member_addition(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let team = state.selected_active_team_for_mutation(&store_id)?;
    let operation = Operation::ResumeTeamMemberAddition {
        profile: team.profile,
        team_alias: team.team_alias,
        username: required_field(&username, "Enter the pending member username.")?,
    };
    apply_operation(&state, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn demote_group_member(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
    destination: RoleInput,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let (team, party_id_hex, current) = state.selected_member_target(&store_id, &username)?;
    let operation = demote_group_member_operation(team, &party_id_hex, current, destination)?;
    apply_operation(&state, operation, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn remove_group_member(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let (team, party_id_hex, _) = state.selected_member_target(&store_id, &username)?;
    let operation = remove_group_member_operation(team, &party_id_hex)?;
    apply_operation(&state, operation, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn resume_group_member_edit(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let team = state.selected_active_team_for_mutation(&store_id)?;
    let operation = Operation::ResumeTeamMemberEdit {
        profile: team.profile,
        team_alias: team.team_alias,
    };
    apply_operation(&state, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn admit_group(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    remote_store_id: String,
    visibility: i16,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let local = state.selected_active_team_for_mutation(&store_id)?;
    let remote = state.selected_remote_named_team(&local.profile, &remote_store_id)?;
    let operation = admit_group_operation(local, remote, visibility);
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn rerun_group_admission(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    operation_id: String,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let (local, entry) = state.selected_inactive_admission(&store_id, &operation_id)?;
    let operation = rerun_group_admission_operation(local, entry)?;
    apply_operation(&state, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn resume_group_creation(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<MutationDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let _permit = state.begin_mutation()?;
    let (store, active) = state.selected_store(&store_id)?;
    let CatalogStoreRef::Team(store) = store else {
        return Err(invalid_request("Only a group creation can be resumed."));
    };
    if active != Some(false) {
        return Err(invalid_request(
            "This group does not report an incomplete creation.",
        ));
    }
    state.ensure_profile_available(&store.profile)?;
    let operation = Operation::ResumeTeamCreation {
        profile: store.profile,
        team_alias: store.team_alias,
    };
    apply_operation(&state, operation, MutationKind::Resume).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_agent_proto::KvPrecondition;

    const READ_VALUE: &[u8] = b"guest-password";

    struct ReadTransport {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for ReadTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ReadKv {
                    store,
                    path,
                    version,
                } => serde_json::to_value(KvReadResult {
                    store,
                    path,
                    version,
                    node_type: "small-file".to_owned(),
                    size: Some(READ_VALUE.len() as u64),
                    read_role: KvRole::Owner,
                    write_role: KvRole::Owner,
                    content: Some(READ_VALUE.to_vec()),
                    symlink_target: None,
                })
                .map_err(|error| foks_desktop::AgentError::Transport(error.to_string())),
                other => panic!("unexpected reveal operation {other:?}"),
            }
        }
    }

    struct DownloadTransport {
        total: u64,
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for DownloadTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            let value = match operation {
                Operation::ReadKv {
                    store,
                    path,
                    version,
                } => serde_json::to_value(KvReadResult {
                    store,
                    path,
                    version,
                    node_type: "file".to_owned(),
                    size: Some(self.total),
                    read_role: KvRole::Owner,
                    write_role: KvRole::Owner,
                    content: None,
                    symlink_target: None,
                }),
                Operation::ReadKvChunk {
                    store,
                    path,
                    version,
                    offset,
                    length,
                } => {
                    let count =
                        usize::try_from((self.total - offset).min(u64::from(length))).unwrap();
                    serde_json::to_value(KvChunkResult {
                        store,
                        path,
                        version,
                        offset,
                        content: vec![0x5a; count],
                        eof: offset + count as u64 == self.total,
                    })
                }
                other => panic!("unexpected download operation {other:?}"),
            };
            value.map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))
        }
    }

    fn download_item(total: u64) -> CatalogItem {
        let store = foks_agent_proto::AccountStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
        };
        CatalogItem {
            store: CatalogStoreRef::Account(store),
            metadata: foks_agent_proto::KvEntryMetadata {
                path: "/large.bin".to_owned(),
                node_type: "file".to_owned(),
                version: 9,
                size: Some(total),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
            },
        }
    }

    #[test]
    fn mutation_gate_refuses_a_second_write_until_the_first_finishes() {
        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        let first = state.begin_mutation().unwrap();
        assert_eq!(
            state.begin_mutation().unwrap_err().code,
            "mutation-in-flight"
        );
        assert_eq!(
            state.begin_catalog_load_checked().unwrap_err().code,
            "mutation-in-flight"
        );
        drop(first);
        assert!(state.begin_mutation().is_ok());
    }

    #[test]
    fn ambiguous_mutations_require_a_fresh_catalog_before_another_write() {
        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        state
            .mutation_requires_refresh
            .store(true, Ordering::Release);
        let error = state.begin_mutation().unwrap_err();
        assert_eq!(error.code, "ambiguous");
        assert!(error.ambiguous);
        state.accept_catalog(0, CatalogSnapshot::default());
        assert!(state.begin_mutation().is_ok());
    }

    #[test]
    fn value_commands_only_accept_the_declared_main_window_label() {
        assert!(main_window_allowed("main"));
        assert!(!main_window_allowed("settings"));
        assert!(!main_window_allowed(""));
    }

    #[test]
    fn show_issues_exactly_one_version_bound_read() {
        let item = CatalogItem {
            store: CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
                profile: "foks.example".to_owned(),
                account_alias: "personal".to_owned(),
            }),
            metadata: foks_agent_proto::KvEntryMetadata {
                path: "/wifi/password".to_owned(),
                node_type: "small-file".to_owned(),
                version: 7,
                size: Some(READ_VALUE.len() as u64),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
            },
        };
        let transport = ReadTransport {
            calls: Mutex::new(Vec::new()),
        };

        let read = read_text(&transport, &item).unwrap();

        assert_eq!(read.value.as_str(), "guest-password");
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[Operation::ReadKv {
                store: KvStoreRef::Account(foks_agent_proto::AccountStoreRef {
                    profile: "foks.example".to_owned(),
                    account_alias: "personal".to_owned(),
                }),
                path: "/wifi/password".to_owned(),
                version: 7,
            }]
        );
    }

    #[test]
    fn wire_contract_fixture_matches_serialized_shapes() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../wire-contract.json")).unwrap();
        let app_info = AppInfo {
            version: "0.3.0".to_owned(),
            agent_socket: "/private/foks/agent.sock".to_owned(),
            managed_profile: Some("local".to_owned()),
        };
        assert_eq!(serde_json::to_value(app_info).unwrap(), fixture["appInfo"]);
        let error = AgentError::from_agent(
            foks_agent_proto::ErrorCode::VersionMismatch,
            "changed".to_owned(),
        );
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            fixture["commandError"]
        );
        let role: RoleDto = KvRole::Member { visibility: 0 }.into();
        assert_eq!(serde_json::to_value(role).unwrap(), fixture["memberRole"]);
        assert_eq!(
            serde_json::json!({"readRole": "Member:0", "writeRole": "Admin"}),
            fixture["teamItemCreateAccess"]
        );
        let account = AccountDto {
            store: "opaque-account-ref".to_owned(),
            profile: "foks.example".to_owned(),
            alias: "personal".to_owned(),
            username: "rae.chen".to_owned(),
        };
        assert_eq!(serde_json::to_value(account).unwrap(), fixture["account"]);
        let party = PartyDto {
            store: "opaque-team-ref".to_owned(),
            username: Some("dana.okafor".to_owned()),
            party_kind: "user".to_owned(),
            generation: 5,
            locally_manageable: true,
            party_id_hex: "01".repeat(33),
            scoped_host_id_hex: None,
            source_role: KvRole::Member { visibility: 0 }.into(),
            destination_role: KvRole::Member { visibility: 0 }.into(),
        };
        assert_eq!(serde_json::to_value(party).unwrap(), fixture["party"]);
        let federation = FederationEntryDto {
            store: "opaque-team-ref".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: "02".repeat(33),
            remote_team_id_hex: "03".repeat(33),
            destination: KvRole::Member { visibility: -2 }.into(),
            operation_id_hex: Some("07".repeat(16)),
            active: false,
        };
        assert_eq!(
            serde_json::to_value(federation).unwrap(),
            fixture["federationEntry"]
        );
        assert_eq!(
            serde_json::to_value(CommandAck { ok: true }).unwrap(),
            fixture["commandAck"]
        );
        assert_eq!(
            serde_json::to_value(DownloadResult { saved: false }).unwrap(),
            fixture["cancelledDownload"]
        );
        assert_eq!(
            serde_json::to_value(MutationDto { applied: true }).unwrap(),
            fixture["appliedMutation"]
        );
        assert_eq!(
            serde_json::to_value(MutationDto { applied: false }).unwrap(),
            fixture["cancelledMutation"]
        );
        let exists = map_mutation_error(
            foks_desktop::AgentError::Protocol {
                code: foks_agent_proto::ErrorCode::Conflict,
                message: "exists".to_owned(),
                fields: foks_agent_proto::ErrorFields::default(),
            },
            MutationKind::Create,
        );
        assert_eq!(
            serde_json::to_value(exists).unwrap(),
            fixture["alreadyExistsError"]
        );
        let capability = map_mutation_error(
            foks_desktop::AgentError::Protocol {
                code: foks_agent_proto::ErrorCode::CapabilityDenied,
                message: "denied".to_owned(),
                fields: foks_agent_proto::ErrorFields {
                    capability: Some("kv".to_owned()),
                    ..Default::default()
                },
            },
            MutationKind::Guarded,
        );
        assert_eq!(
            serde_json::to_value(capability).unwrap(),
            fixture["capabilityUnavailableError"]
        );
        let agent_lost = map_mutation_error(
            foks_desktop::AgentError::Transport("gone".to_owned()),
            MutationKind::Guarded,
        );
        assert_eq!(
            serde_json::to_value(agent_lost).unwrap(),
            fixture["agentLostError"]
        );
        let store = StoreDto {
            id: "opaque-store-ref".to_owned(),
            kind: "account",
            name: "Personal".to_owned(),
            server: "foks.example".to_owned(),
            account: "personal".to_owned(),
            alias: None,
            active: None,
            team_kind: None,
            team_id_hex: None,
        };
        let catalog = CatalogDto {
            profiles: vec!["foks.example".to_owned()],
            stores: vec![store.clone()],
            known_stores: vec![store],
            inventory: vec![CatalogInventoryDto {
                profile: "foks.example".to_owned(),
                accounts_complete: true,
                teams_complete: true,
            }],
            items: vec![ItemDto {
                store: "opaque-store-ref".to_owned(),
                path: "/wifi/password".to_owned(),
                kind: "Secret",
                size: 42,
                version: 7,
                read: KvRole::Member { visibility: -16384 }.into(),
                write: KvRole::Admin.into(),
            }],
            failures: vec![],
            blocked_profiles: vec![],
        };
        assert_eq!(serde_json::to_value(catalog).unwrap(), fixture["catalog"]);
        let read = ReadItemDto {
            store: "opaque-store-ref".to_owned(),
            path: "/wifi/password".to_owned(),
            version: 7,
            value: Zeroizing::new("secret".to_owned()),
        };
        assert_eq!(serde_json::to_value(read).unwrap(), fixture["readItem"]);
        let checked = CheckedProfileDto {
            profile: "work".to_owned(),
            acceptance: "inserted".to_owned(),
            lookup_name: "foks.example".to_owned(),
            canonical_name: "foks.example".to_owned(),
            host_id: "02".repeat(33),
            chain: 9,
            epoch: 12,
        };
        assert_eq!(
            serde_json::to_value(checked).unwrap(),
            fixture["checkedProfile"]
        );
        let pending = PendingOperationDto {
            kind: "account-recovery",
            alias: "recovered".to_owned(),
            target: Some("owner".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(pending).unwrap(),
            fixture["pendingOperation"]
        );
        let backup = BackupPhraseDto {
            backup_alias: "paper".to_owned(),
            phrase: Zeroizing::new(
                "abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon"
                    .to_owned(),
            ),
        };
        assert_eq!(
            serde_json::to_value(backup).unwrap(),
            fixture["backupPhrase"]
        );
        let discovery = GroupDiscoveryDto {
            account_alias: "personal".to_owned(),
            groups: vec![DiscoveredGroupDto {
                alias: "engineering".to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: "03".repeat(33),
                kind: "named",
                name: Some("Engineering".to_owned()),
                active: true,
            }],
        };
        assert_eq!(
            serde_json::to_value(discovery).unwrap(),
            fixture["groupDiscovery"]
        );
        let status = ServerStatusSnapshotDto {
            profile: "work".to_owned(),
            configured_probe: "foks.example".to_owned(),
            host: Some(StoredHostDto {
                lookup_name: "foks.example".to_owned(),
                canonical_name: "foks.example".to_owned(),
                host_id: "02".repeat(33),
                chain: 11,
                epoch: 42,
            }),
            lease_required: true,
            lease_expires_at: Some(1_900_000_000),
        };
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            fixture["serverStatus"]
        );
        let added = AddedServerDto {
            profile: "partner".to_owned(),
            configured_probe: "foks.partner.example".to_owned(),
        };
        assert_eq!(serde_json::to_value(added).unwrap(), fixture["addedServer"]);
        let forgotten = ForgottenServerDto {
            profile: "partner".to_owned(),
            removed: true,
        };
        assert_eq!(
            serde_json::to_value(forgotten).unwrap(),
            fixture["forgottenServer"]
        );
        let checked = CheckedServerDto {
            profile: "work".to_owned(),
            acceptance: "advanced".to_owned(),
            lookup_name: "foks.example".to_owned(),
            canonical_name: "foks.example".to_owned(),
            host_id: "02".repeat(33),
            chain: 12,
            epoch: 43,
        };
        assert_eq!(
            serde_json::to_value(checked).unwrap(),
            fixture["checkedServer"]
        );
        let device = DeviceDto {
            id: "04".repeat(33),
            name: Some("This Mac".to_owned()),
            role: "owner",
            current: true,
        };
        assert_eq!(serde_json::to_value(device).unwrap(), fixture["device"]);
        let removal = DeviceRemovalDto {
            device_id: "04".repeat(33),
            user_chain_sequence: 15,
            already_absent: false,
        };
        assert_eq!(
            serde_json::to_value(removal).unwrap(),
            fixture["deviceRemoval"]
        );
        let enrollment = BackupEnrollmentDto {
            backup_alias: "paper".to_owned(),
            account_alias: "personal".to_owned(),
            backup_id: "10".repeat(33),
        };
        assert_eq!(
            serde_json::to_value(enrollment).unwrap(),
            fixture["backupEnrollment"]
        );
        assert_eq!(
            serde_json::to_value(YubiCardDto { serial: 424_242 }).unwrap(),
            fixture["yubiCard"]
        );
        assert_eq!(
            serde_json::to_value(YubiEnrollmentDto {
                alias: "work_key".to_owned(),
                state: "complete",
            })
            .unwrap(),
            fixture["yubiEnrollment"]
        );
        assert_eq!(
            serde_json::to_value(YubiAccountDto {
                alias: "work_key".to_owned(),
                username: "rae".to_owned(),
                yubi_id: "08".repeat(34),
                subkey_id: "0d".repeat(33),
                user_chain_sequence: 21,
                management_enrolled: true,
            })
            .unwrap(),
            fixture["yubiAccount"]
        );
        assert_eq!(
            serde_json::to_value(YubiSyncDto {
                username: "rae".to_owned(),
                user_chain_sequence: 22,
                directories: 2,
                entries: 7,
                federation: vec![YubiFederationRefreshDto {
                    local_profile: "work".to_owned(),
                    local_team_alias: "engineering".to_owned(),
                    refreshed: true,
                    deferred: None,
                }],
            })
            .unwrap(),
            fixture["yubiSync"]
        );
        assert_eq!(
            serde_json::to_value(YubiPinStatusDto {
                remaining: 3,
                blocked: false,
            })
            .unwrap(),
            fixture["yubiPinStatus"]
        );
        assert_eq!(
            serde_json::to_value(YubiLifecycleDto {
                alias: "work_key".to_owned(),
                management_enrolled: true,
                management_generation: Some(4),
            })
            .unwrap(),
            fixture["yubiLifecycle"]
        );
        assert_eq!(
            serde_json::to_value(YubiSubkeyRecoveryDto {
                alias: "work_key".to_owned(),
                subkey_id: "0d".repeat(33),
                certificate_count: 2,
            })
            .unwrap(),
            fixture["yubiSubkeyRecovery"]
        );
        assert_eq!(
            serde_json::to_value(YubiRevocationDto {
                alias: "work_key".to_owned(),
                user_chain_sequence: 23,
                removed_local_credential: true,
            })
            .unwrap(),
            fixture["yubiRevocation"]
        );
        assert_eq!(
            serde_json::to_value(YubiChangedDto {
                alias: "work_key".to_owned(),
                changed: true,
            })
            .unwrap(),
            fixture["yubiChanged"]
        );
        let offer = PairingOfferDto {
            account_alias: "personal".to_owned(),
            phrase: Zeroizing::new(
                "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount".to_owned(),
            ),
        };
        assert_eq!(
            serde_json::to_value(offer).unwrap(),
            fixture["pairingOffer"]
        );
        let provision = DeviceProvisionDto {
            alias: "paired".to_owned(),
            device_id: "04".repeat(33),
            user_chain_sequence: 14,
        };
        assert_eq!(
            serde_json::to_value(provision).unwrap(),
            fixture["deviceProvision"]
        );
        let passphrase = PassphraseReportDto {
            generation: 2,
            stretch_version: "v1".to_owned(),
            verified: true,
        };
        assert_eq!(
            serde_json::to_value(passphrase).unwrap(),
            fixture["passphraseReport"]
        );
        let preview = ResetPreviewDto {
            profile: "work".to_owned(),
            resumables: vec![PendingOperationDto {
                kind: "pairing-offer",
                alias: "personal".to_owned(),
                target: None,
            }],
            artifacts: vec![ResetArtifactDto {
                kind: "hard-state",
                entries: 3,
                bytes: 4096,
            }],
            token: Zeroizing::new("one-use-reset-token".to_owned()),
            expires_in_seconds: 300,
        };
        assert_eq!(
            serde_json::to_value(preview).unwrap(),
            fixture["resetPreview"]
        );
    }

    #[test]
    fn first_run_response_projection_fails_closed() {
        let checked = CheckedProfileResponse {
            profile: CheckedProfileIdentity {
                name: "different".to_owned(),
                probe: "foks.example".to_owned(),
                protocol: serde_json::json!({"generation":"v019"}),
                trust: serde_json::json!({"kind":"web-pki"}),
            },
            probe: CheckedProbeResponse {
                acceptance: "inserted".to_owned(),
                lookup_name: "foks.example".to_owned(),
                canonical_name: "foks.example".to_owned(),
                host_id_hex: "01".to_owned(),
                host_chain_sequence: 1,
                merkle_epoch: 2,
            },
        };
        assert_eq!(
            CheckedProfileDto::from_response("work", "foks.example", checked)
                .unwrap_err()
                .code,
            "invalid-response"
        );

        let wrong_prefix_host = CheckedProfileResponse {
            profile: CheckedProfileIdentity {
                name: "work".to_owned(),
                probe: "foks.example".to_owned(),
                protocol: serde_json::json!({"generation":"v019"}),
                trust: serde_json::json!({"kind":"web-pki"}),
            },
            probe: CheckedProbeResponse {
                acceptance: "inserted".to_owned(),
                lookup_name: "foks.example".to_owned(),
                canonical_name: "foks.example".to_owned(),
                host_id_hex: "01".repeat(33),
                host_chain_sequence: 1,
                merkle_epoch: 2,
            },
        };
        assert_eq!(
            CheckedProfileDto::from_response("work", "foks.example", wrong_prefix_host)
                .unwrap_err()
                .code,
            "invalid-response"
        );
        assert!(CheckedProfileDto::from_response(
            "work",
            "foks.example",
            CheckedProfileResponse {
                profile: CheckedProfileIdentity {
                    name: "work".to_owned(),
                    probe: "foks.example".to_owned(),
                    protocol: serde_json::json!({"generation":"v019"}),
                    trust: serde_json::json!({"kind":"web-pki"}),
                },
                probe: CheckedProbeResponse {
                    acceptance: "inserted".to_owned(),
                    lookup_name: "foks.example".to_owned(),
                    canonical_name: "foks.example".to_owned(),
                    host_id_hex: "02".repeat(33),
                    host_chain_sequence: 1,
                    merkle_epoch: 2,
                },
            }
        )
        .is_ok());

        for acceptance in ["advanced", "unchanged"] {
            let response = CheckedProfileResponse {
                profile: CheckedProfileIdentity {
                    name: "work".to_owned(),
                    probe: "foks.example".to_owned(),
                    protocol: serde_json::json!({"generation":"v019"}),
                    trust: serde_json::json!({"kind":"web-pki"}),
                },
                probe: CheckedProbeResponse {
                    acceptance: acceptance.to_owned(),
                    lookup_name: "foks.example".to_owned(),
                    canonical_name: "foks.example".to_owned(),
                    host_id_hex: "02".repeat(33),
                    host_chain_sequence: 1,
                    merkle_epoch: 2,
                },
            };
            assert_eq!(
                CheckedProfileDto::from_response("work", "foks.example", response)
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }

        let discovery = GroupDiscoveryResponse {
            account_alias: "personal".to_owned(),
            teams: vec![DiscoveredGroupResponse {
                alias: "engineering".to_owned(),
                account_alias: "other".to_owned(),
                team_id_hex: "03".to_owned(),
                kind: "named".to_owned(),
                name: Some("Engineering".to_owned()),
                active: true,
            }],
        };
        assert_eq!(
            GroupDiscoveryDto::from_response("personal", discovery)
                .unwrap_err()
                .code,
            "invalid-response"
        );

        let duplicate_alias = GroupDiscoveryResponse {
            account_alias: "personal".to_owned(),
            teams: vec![
                DiscoveredGroupResponse {
                    alias: "engineering".to_owned(),
                    account_alias: "personal".to_owned(),
                    team_id_hex: "03".repeat(33),
                    kind: "named".to_owned(),
                    name: Some("Engineering".to_owned()),
                    active: true,
                },
                DiscoveredGroupResponse {
                    alias: "engineering".to_owned(),
                    account_alias: "personal".to_owned(),
                    team_id_hex: format!("03{}", "04".repeat(32)),
                    kind: "named".to_owned(),
                    name: Some("Other".to_owned()),
                    active: true,
                },
            ],
        };
        assert_eq!(
            GroupDiscoveryDto::from_response("personal", duplicate_alias)
                .unwrap_err()
                .code,
            "invalid-response"
        );
        let duplicate_id = GroupDiscoveryResponse {
            account_alias: "personal".to_owned(),
            teams: ["engineering", "operations"]
                .into_iter()
                .map(|alias| DiscoveredGroupResponse {
                    alias: alias.to_owned(),
                    account_alias: "personal".to_owned(),
                    team_id_hex: "03".repeat(33),
                    kind: "named".to_owned(),
                    name: Some(alias.to_owned()),
                    active: true,
                })
                .collect(),
        };
        assert_eq!(
            GroupDiscoveryDto::from_response("personal", duplicate_id)
                .unwrap_err()
                .code,
            "invalid-response"
        );
        let invalid_local_alias = GroupDiscoveryResponse {
            account_alias: "personal".to_owned(),
            teams: vec![DiscoveredGroupResponse {
                alias: "engineering/group".to_owned(),
                account_alias: "personal".to_owned(),
                team_id_hex: "03".repeat(33),
                kind: "named".to_owned(),
                name: Some("Engineering".to_owned()),
                active: true,
            }],
        };
        assert_eq!(
            GroupDiscoveryDto::from_response("personal", invalid_local_alias)
                .unwrap_err()
                .code,
            "invalid-response"
        );

        for (kind, name, wrong_prefix) in [
            ("named", Some("Engineering".to_owned()), "14"),
            ("ad-hoc", None, "03"),
        ] {
            let wrong_type = GroupDiscoveryResponse {
                account_alias: "personal".to_owned(),
                teams: vec![DiscoveredGroupResponse {
                    alias: "engineering".to_owned(),
                    account_alias: "personal".to_owned(),
                    team_id_hex: wrong_prefix.repeat(33),
                    kind: kind.to_owned(),
                    name,
                    active: true,
                }],
            };
            assert_eq!(
                GroupDiscoveryDto::from_response("personal", wrong_type)
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }
        let valid_typed_groups = GroupDiscoveryResponse {
            account_alias: "personal".to_owned(),
            teams: vec![
                DiscoveredGroupResponse {
                    alias: "engineering".to_owned(),
                    account_alias: "personal".to_owned(),
                    team_id_hex: "03".repeat(33),
                    kind: "named".to_owned(),
                    name: Some("Engineering".to_owned()),
                    active: true,
                },
                DiscoveredGroupResponse {
                    alias: "friends".to_owned(),
                    account_alias: "personal".to_owned(),
                    team_id_hex: "14".repeat(33),
                    kind: "ad-hoc".to_owned(),
                    name: None,
                    active: true,
                },
            ],
        };
        assert_eq!(
            GroupDiscoveryDto::from_response("personal", valid_typed_groups)
                .unwrap()
                .groups
                .len(),
            2
        );

        let duplicate_pending = vec![
            PendingOperationSummary {
                kind: PendingOperationKind::AccountSignup,
                alias: "personal".to_owned(),
                target: None,
            },
            PendingOperationSummary {
                kind: PendingOperationKind::AccountSignup,
                alias: "personal".to_owned(),
                target: None,
            },
        ];
        assert_eq!(
            validated_pending_dtos(&duplicate_pending).unwrap_err().code,
            "invalid-response"
        );
        let too_many_pending = vec![
            PendingOperationSummary {
                kind: PendingOperationKind::AccountSignup,
                alias: "personal".to_owned(),
                target: None,
            };
            MAXIMUM_FIRST_RUN_ROWS + 1
        ];
        assert_eq!(
            validated_pending_dtos(&too_many_pending).unwrap_err().code,
            "invalid-response"
        );
        assert!(
            serde_json::from_value::<PendingOperationResponse>(serde_json::json!({
                "kind":"account-signup",
                "alias":"personal",
                "target":null,
                "invented":true
            }))
            .is_err()
        );
        assert_eq!(
            require_nested_response_row_cap(
                &serde_json::json!({
                    "teams": vec![serde_json::Value::Null; MAXIMUM_FIRST_RUN_ROWS + 1]
                }),
                "teams",
                "discovered groups",
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
    }

    #[test]
    fn recovery_serial_is_positive_and_not_renderer_controlled() {
        let mut fills = vec![[0u8; 8], 7u64.to_le_bytes()].into_iter();
        let serial = positive_recovery_serial_with(|bytes| {
            bytes.copy_from_slice(&fills.next().unwrap());
            Ok(())
        })
        .unwrap();
        assert_eq!(serial, 7);

        let error = positive_recovery_serial_with(|bytes| {
            bytes.fill(0);
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "randomness-unavailable");
    }

    #[test]
    fn phase_five_local_names_match_the_client_boundary() {
        let maximum = "x".repeat(64);
        for valid in ["a", "work_profile-2", maximum.as_str()] {
            assert_eq!(bounded_local_name(valid, "bad").unwrap(), valid);
        }
        let too_long = "x".repeat(65);
        for invalid in ["", "has space", "has/slash", "dot.name", too_long.as_str()] {
            assert_eq!(
                bounded_local_name(invalid, "bad").unwrap_err().code,
                "invalid-request"
            );
        }
    }

    struct ProfileListTransport {
        value: serde_json::Value,
    }

    fn test_profile(name: impl Into<String>) -> ProfileSummary {
        ProfileSummary {
            name: name.into(),
            probe: "foks.example".to_owned(),
            protocol: ProfileProtocolSummary::V019,
            trust: ProfileTrustSummary::WebPki,
        }
    }

    fn test_profile_value(name: impl Into<String>) -> serde_json::Value {
        serde_json::to_value(test_profile(name)).unwrap()
    }

    impl foks_desktop::AgentTransport for ProfileListTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            assert_eq!(operation, Operation::ListProfiles);
            Ok(self.value.clone())
        }
    }

    struct SetupProfileTransport {
        profiles: Vec<ProfileSummary>,
        probe_error: Option<String>,
        calls: Mutex<Vec<Operation>>,
    }

    impl SetupProfileTransport {
        fn new(profiles: Vec<ProfileSummary>) -> Self {
            Self {
                profiles,
                probe_error: None,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl foks_desktop::AgentTransport for SetupProfileTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ListProfiles => serde_json::to_value(&self.profiles)
                    .map_err(|error| foks_desktop::AgentError::Transport(error.to_string())),
                Operation::Probe { profile } => {
                    if let Some(error) = &self.probe_error {
                        return Err(foks_desktop::AgentError::Protocol {
                            code: foks_agent_proto::ErrorCode::OperationFailed,
                            message: error.clone(),
                            fields: foks_agent_proto::ErrorFields::default(),
                        });
                    }
                    let configured = self
                        .profiles
                        .iter()
                        .find(|candidate| candidate.name == profile)
                        .expect("the setup probe must select a listed profile");
                    Ok(serde_json::json!({
                        "acceptance":"unchanged",
                        "lookup_name":normalized_probe_hostname(&configured.probe).unwrap(),
                        "canonical_name":"localhost",
                        "host_id_hex":"02".repeat(33),
                        "host_chain_sequence":5,
                        "merkle_epoch":9
                    }))
                }
                Operation::CheckAndAddProfile {
                    name,
                    probe,
                    protocol,
                    trust,
                } => {
                    assert_eq!(protocol, ProfileProtocol::V019);
                    assert_eq!(trust, ProfileTrust::WebPki);
                    let lookup_name = normalized_probe_hostname(&probe).unwrap();
                    Ok(serde_json::json!({
                        "profile":{
                            "name":name,
                            "probe":probe,
                            "protocol":{"generation":"v019"},
                            "trust":{"kind":"web-pki"}
                        },
                        "probe":{
                            "acceptance":"inserted",
                            "lookup_name":lookup_name,
                            "canonical_name":lookup_name,
                            "host_id_hex":"02".repeat(33),
                            "host_chain_sequence":1,
                            "merkle_epoch":2
                        }
                    }))
                }
                other => panic!("unexpected setup profile operation {other:?}"),
            }
        }
    }

    #[test]
    fn setup_reuses_the_one_profile_at_the_normalized_endpoint() {
        let mut local = test_profile("local");
        local.probe = "localhost:4430".to_owned();
        local.trust = ProfileTrustSummary::CertificateDer {
            path: PathBuf::from("/private/foks-dev-ca.der"),
        };
        let transport = SetupProfileTransport::new(vec![local]);

        let checked = check_existing_or_add_profile(
            &transport,
            "setup-localhost".to_owned(),
            "localhost".to_owned(),
        )
        .unwrap();

        assert_eq!(checked.profile, "local");
        assert_eq!(checked.acceptance, "unchanged");
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::Probe {
                    profile: "local".to_owned()
                }
            ]
        );
    }

    #[test]
    fn setup_creates_webpki_only_when_no_endpoint_matches() {
        let mut local = test_profile("local");
        local.probe = "localhost:4430".to_owned();
        local.trust = ProfileTrustSummary::CertificateDer {
            path: PathBuf::from("/private/foks-dev-ca.der"),
        };
        let transport = SetupProfileTransport::new(vec![local]);

        let checked = check_existing_or_add_profile(
            &transport,
            "setup-localhost".to_owned(),
            "localhost:4431".to_owned(),
        )
        .unwrap();

        assert_eq!(checked.profile, "setup-localhost");
        assert_eq!(checked.acceptance, "inserted");
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::CheckAndAddProfile {
                    name: "setup-localhost".to_owned(),
                    probe: "localhost:4431".to_owned(),
                    protocol: ProfileProtocol::V019,
                    trust: ProfileTrust::WebPki,
                }
            ]
        );
    }

    #[test]
    fn setup_fails_closed_for_ambiguous_or_failed_saved_profiles() {
        let mut first = test_profile("first");
        first.probe = "LOCALHOST.".to_owned();
        let mut second = test_profile("second");
        second.probe = "localhost:4430".to_owned();
        let duplicate = SetupProfileTransport::new(vec![first, second]);
        let error = check_existing_or_add_profile(
            &duplicate,
            "setup-localhost".to_owned(),
            "localhost".to_owned(),
        )
        .unwrap_err();
        assert_eq!(error.code, "profile-conflict");
        assert_eq!(
            duplicate.calls.lock().unwrap().as_slice(),
            &[Operation::ListProfiles]
        );

        let mut local = test_profile("local");
        local.probe = "localhost:4430".to_owned();
        local.trust = ProfileTrustSummary::CertificateDer {
            path: PathBuf::from("/private/foks-dev-ca.der"),
        };
        let failed = SetupProfileTransport {
            probe_error: Some("saved profile rejected its certificate".to_owned()),
            ..SetupProfileTransport::new(vec![local])
        };
        let error = check_existing_or_add_profile(
            &failed,
            "setup-localhost".to_owned(),
            "localhost".to_owned(),
        )
        .unwrap_err();
        assert_eq!(error.code, "operation-failed");
        assert_eq!(
            failed.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::Probe {
                    profile: "local".to_owned()
                }
            ]
        );
    }

    #[test]
    fn setup_endpoint_normalization_keeps_ports_in_the_identity() {
        assert_eq!(
            normalized_probe_endpoint("LOCALHOST."),
            normalized_probe_endpoint("localhost:4430")
        );
        assert_ne!(
            normalized_probe_endpoint("localhost"),
            normalized_probe_endpoint("localhost:4431")
        );
        assert_eq!(
            normalized_probe_endpoint("::1"),
            normalized_probe_endpoint("[::1]:4430")
        );
    }

    #[test]
    fn profile_preflight_is_exact_bounded_and_unique() {
        let real_wire = serde_json::to_value(vec![test_profile("work")]).unwrap();
        assert_eq!(
            real_wire,
            serde_json::json!([{
                "name":"work",
                "probe":"foks.example",
                "protocol":{"generation":"v019"},
                "trust":{"kind":"web-pki"}
            }])
        );
        assert!(
            require_transport_profile(&ProfileListTransport { value: real_wire }, "work").is_ok()
        );

        let mut unknown = test_profile_value("work");
        unknown
            .as_object_mut()
            .unwrap()
            .insert("invented".to_owned(), serde_json::Value::Bool(true));
        let error = require_transport_profile(
            &ProfileListTransport {
                value: serde_json::Value::Array(vec![unknown]),
            },
            "work",
        )
        .unwrap_err();
        assert_eq!(error.code, "invalid-response");

        assert_eq!(
            require_transport_profile(
                &ProfileListTransport {
                    value: serde_json::json!([{"name":"work"}]),
                },
                "work",
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        for value in [
            serde_json::Value::Array(vec![test_profile_value("has space")]),
            serde_json::Value::Array(vec![test_profile_value("work"), test_profile_value("work")]),
        ] {
            assert_eq!(
                require_transport_profile(&ProfileListTransport { value }, "work")
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }

        let mut invalid_probe = test_profile("work");
        invalid_probe.probe = "https://foks.example".to_owned();
        assert_eq!(
            require_transport_profile(
                &ProfileListTransport {
                    value: serde_json::json!([invalid_probe]),
                },
                "work",
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        let mut invalid_trust = test_profile("work");
        invalid_trust.trust = ProfileTrustSummary::CertificateDer {
            path: PathBuf::new(),
        };
        assert_eq!(
            require_transport_profile(
                &ProfileListTransport {
                    value: serde_json::json!([invalid_trust]),
                },
                "work",
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        let mut unknown_protocol = test_profile_value("work");
        unknown_protocol["protocol"]
            .as_object_mut()
            .unwrap()
            .insert("invented".to_owned(), serde_json::Value::Bool(true));
        assert_eq!(
            require_transport_profile(
                &ProfileListTransport {
                    value: serde_json::Value::Array(vec![unknown_protocol]),
                },
                "work",
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        let too_many = (0..=MAXIMUM_FIRST_RUN_ROWS)
            .map(|index| test_profile_value(format!("p{index}")))
            .collect::<Vec<_>>();
        assert_eq!(
            require_transport_profile(
                &ProfileListTransport {
                    value: serde_json::Value::Array(too_many),
                },
                "work",
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
    }

    #[test]
    fn phase_six_wire_responses_are_exact_bounded_and_request_bound() {
        assert!(exact_profile_confirmation("work", "work", "reset").is_ok());
        for confirmation in [" work", "work ", "WORK", "other"] {
            assert_eq!(
                exact_profile_confirmation("work", confirmation, "reset")
                    .unwrap_err()
                    .code,
                "invalid-request"
            );
        }
        let added = added_server_response(test_profile_value("partner"), "partner", "foks.example")
            .unwrap();
        assert_eq!(added.profile, "partner");
        assert_eq!(added.configured_probe, "foks.example");
        assert_eq!(
            added_server_response(
                serde_json::json!({
                    "name":"partner",
                    "probe":"foks.example",
                    "protocol":{"generation":"v019"},
                    "trust":{"kind":"web-pki"},
                    "invented":true
                }),
                "partner",
                "foks.example"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
        assert!(forgotten_server_response(
            serde_json::json!({"profile":"partner","removed":true}),
            "partner"
        )
        .is_ok());
        for malformed in [
            serde_json::json!({"profile":"other","removed":true}),
            serde_json::json!({"profile":"partner","removed":false}),
            serde_json::json!({"profile":"partner","removed":true,"invented":true}),
        ] {
            assert_eq!(
                forgotten_server_response(malformed, "partner")
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }

        let status = server_status_response(
            serde_json::json!({
                "profile":"work",
                "configured_probe":"foks.example",
                "host":{
                    "lookup_name":"foks.example",
                    "canonical_name":"foks.example",
                    "host_id_hex":"02".repeat(33),
                    "host_chain_sequence":4,
                    "merkle_epoch":8
                },
                "lease_required":true,
                "lease_expires_at":1_900_000_000u64
            }),
            "work",
            "foks.example",
            true,
        )
        .unwrap();
        assert_eq!(status.host.unwrap().chain, 4);
        assert_eq!(
            server_status_response(
                serde_json::json!({
                    "profile":"work",
                    "configured_probe":"foks.example",
                    "host":null,
                    "lease_required":false,
                    "lease_expires_at":null
                }),
                "work",
                "foks.example",
                true,
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        for acceptance in ["inserted", "advanced", "unchanged"] {
            assert_eq!(
                checked_server_response(
                    serde_json::json!({
                        "acceptance":acceptance,
                        "lookup_name":"foks.example",
                        "canonical_name":"foks.example",
                        "host_id_hex":"02".repeat(33),
                        "host_chain_sequence":5,
                        "merkle_epoch":9
                    }),
                    "work",
                )
                .unwrap()
                .acceptance,
                acceptance
            );
        }
        assert_eq!(
            checked_server_response(
                serde_json::json!({
                    "acceptance":"invented",
                    "lookup_name":"foks.example",
                    "canonical_name":"foks.example",
                    "host_id_hex":"02".repeat(33),
                    "host_chain_sequence":5,
                    "merkle_epoch":9
                }),
                "work",
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        for malformed in [
            serde_json::json!({
                "profile":"other",
                "configured_probe":"foks.example",
                "host":null,
                "lease_required":false,
                "lease_expires_at":null
            }),
            serde_json::json!({
                "profile":"work",
                "configured_probe":"foks.example",
                "host":null,
                "lease_required":false,
                "lease_expires_at":1
            }),
            serde_json::json!({
                "profile":"work",
                "configured_probe":"foks.example",
                "host":null,
                "lease_required":false,
                "lease_expires_at":null,
                "invented":true
            }),
            serde_json::json!({
                "profile":"work",
                "configured_probe":"foks.example",
                "host":{
                    "lookup_name":"foks.example",
                    "canonical_name":"foks.example",
                    "host_id_hex":"bad",
                    "host_chain_sequence":0,
                    "merkle_epoch":0
                },
                "lease_required":false,
                "lease_expires_at":null
            }),
        ] {
            assert_eq!(
                server_status_response(malformed, "work", "foks.example", false)
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }

        let devices = device_dtos(serde_json::json!([
            {"id_hex":"04".repeat(33),"name":"This Mac","role":"owner","current":true},
            {"id_hex":format!("04{}", "06".repeat(32)),"name":null,"role":"admin","current":false},
            {"id_hex":"08".repeat(34),"name":"Security key","role":"owner","current":false}
        ]))
        .unwrap();
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].name.as_deref(), Some("This Mac"));
        assert_eq!(devices[1].name, None);
        for malformed in [
            serde_json::json!([
                {"id_hex":"04".repeat(33),"role":"owner","current":true},
                {"id_hex":"04".repeat(33),"role":"owner","current":false}
            ]),
            serde_json::json!([{"id_hex":"04".repeat(33),"role":"robot","current":true}]),
            serde_json::json!([{"id_hex":"04".repeat(33),"role":"owner","current":false}]),
            serde_json::json!([{"id_hex":"04".repeat(33),"name":"bad\nname","role":"owner","current":true}]),
            serde_json::json!([{"id_hex":"04".repeat(33),"name":"x".repeat(257),"role":"owner","current":true}]),
        ] {
            assert_eq!(device_dtos(malformed).unwrap_err().code, "invalid-response");
        }

        assert!(backup_enrollment_dtos(
            serde_json::json!([{
                "backup_alias":"paper",
                "account_alias":"personal",
                "backup_id_hex":"10".repeat(33)
            }]),
            "personal"
        )
        .is_ok());
        assert_eq!(
            backup_enrollment_dtos(
                serde_json::json!([{
                    "backup_alias":"paper",
                    "account_alias":"other",
                    "backup_id_hex":"10".repeat(33)
                }]),
                "personal"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        let phrase = "cage 32 advice 4 letter 128 avoid 16 acoustic 2 doctor 64 amount";
        let offer = pairing_offer_response(
            serde_json::json!({"account_alias":"personal","phrase":phrase}),
            "personal",
        )
        .unwrap();
        assert!(!format!("{offer:?}").contains(phrase));
        assert!(device_provision_response(
            serde_json::json!({
                "alias":"paired",
                "device_id_hex":"04".repeat(33),
                "user_chain_sequence":0
            }),
            "paired"
        )
        .is_ok());
        assert!(device_removal_response(
            serde_json::json!({
                "device_id_hex":"04".repeat(33),
                "user_chain_sequence":0,
                "already_absent":false
            }),
            &"04".repeat(33)
        )
        .is_ok());
        assert_eq!(
            device_removal_response(
                serde_json::json!({
                    "device_id_hex":"04".repeat(33),
                    "user_chain_sequence":0,
                    "already_absent":false,
                    "invented":true
                }),
                &"04".repeat(33)
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        let preview = reset_preview_response(
            serde_json::json!({
                "profile":"work",
                "resumables":[{"kind":"pairing-offer","alias":"personal","target":null}],
                "artifacts":[
                    {"kind":"external-database-claim","entries":2,"bytes":100},
                    {"kind":"hard-state","entries":1,"bytes":25},
                    {"kind":"external-database-claim","entries":3,"bytes":250}
                ],
                "token":"one-use-token",
                "expires_in_seconds":300
            }),
            "work",
        )
        .unwrap();
        assert_eq!(preview.resumables[0].kind, "pairing-offer");
        assert_eq!(
            preview.artifacts,
            vec![
                ResetArtifactDto {
                    kind: "external-database-claim",
                    entries: 5,
                    bytes: 350,
                },
                ResetArtifactDto {
                    kind: "hard-state",
                    entries: 1,
                    bytes: 25,
                },
            ]
        );
        assert!(!format!("{preview:?}").contains("one-use-token"));
        for malformed in [
            serde_json::json!({
                "profile":"work","resumables":[],
                "artifacts":[
                    {"kind":"external-database-claim","entries":u64::MAX,"bytes":1},
                    {"kind":"external-database-claim","entries":1,"bytes":1}
                ],
                "token":"token","expires_in_seconds":300
            }),
            serde_json::json!({
                "profile":"work","resumables":[],"artifacts":[],
                "token":"","expires_in_seconds":300
            }),
            serde_json::json!({
                "profile":"work","resumables":[],"artifacts":[],
                "token":"token","expires_in_seconds":300,"invented":true
            }),
        ] {
            assert_eq!(
                reset_preview_response(malformed, "work").unwrap_err().code,
                "invalid-response"
            );
        }
    }

    #[test]
    fn security_key_responses_are_exact_typed_and_state_bound() {
        assert_eq!(yubi_slots(0x82, 0x83).unwrap(), (0x82, 0x83));
        for (signing, pq) in [(0x82, 0x82), (0x81, 0x83), (0x82, 0x96)] {
            assert_eq!(yubi_slots(signing, pq).unwrap_err().code, "invalid-request");
        }
        assert_eq!(
            yubi_retry_configuration("12345678".to_owned(), 0, 3)
                .unwrap_err()
                .code,
            "invalid-request"
        );
        let cards = yubi_card_dtos(serde_json::json!([
            {"name":"YubiKey 5","serial":42},
            {"name":"YubiKey Bio","serial":43}
        ]))
        .unwrap();
        assert_eq!(cards[0], YubiCardDto { serial: 42 });
        for malformed in [
            serde_json::json!([{"name":"YubiKey","serial":0}]),
            serde_json::json!([
                {"name":"YubiKey","serial":42},
                {"name":"Other","serial":42}
            ]),
            serde_json::json!([{"name":"YubiKey","serial":42,"status":"ready"}]),
        ] {
            assert_eq!(
                yubi_card_dtos(malformed).unwrap_err().code,
                "invalid-response"
            );
        }

        let enrollments = yubi_enrollment_dtos(serde_json::json!([
            {"alias":"pending_key","state":"pending"},
            {"alias":"ready_key","state":"complete"}
        ]))
        .unwrap();
        assert!(require_yubi_enrollment(&enrollments, "pending_key", "pending").is_ok());
        assert!(require_yubi_enrollment(&enrollments, "ready_key", "complete").is_ok());
        let completed_for_resume =
            require_yubi_enrollment(&enrollments, "ready_key", "pending").unwrap_err();
        assert_eq!(completed_for_resume.code, "security-key-state-changed");
        assert!(completed_for_resume.message.contains("Refresh"));
        assert!(!completed_for_resume.message.contains("Resume"));
        assert_eq!(
            require_yubi_enrollment(&enrollments, "unknown", "complete")
                .unwrap_err()
                .code,
            "security-key-not-found"
        );
        for crash_recovery_rows in [
            serde_json::json!([
                {"alias":"resumable_key","state":"pending"},
                {"alias":"resumable_key","state":"complete"}
            ]),
            serde_json::json!([
                {"alias":"resumable_key","state":"complete"},
                {"alias":"resumable_key","state":"pending"}
            ]),
        ] {
            let crash_recovery = yubi_enrollment_dtos(crash_recovery_rows).unwrap();
            assert!(require_yubi_enrollment(&crash_recovery, "resumable_key", "pending").is_ok());
            let error =
                require_yubi_enrollment(&crash_recovery, "resumable_key", "complete").unwrap_err();
            assert_eq!(error.code, "security-key-state-changed");
            assert!(error.message.contains("Resume"));
            assert!(error.message.contains("refresh"));
        }
        for malformed in [
            serde_json::json!([
                {"alias":"same","state":"pending"},
                {"alias":"same","state":"pending"}
            ]),
            serde_json::json!([
                {"alias":"same","state":"complete"},
                {"alias":"same","state":"complete"}
            ]),
            serde_json::json!([{"alias":"ready","state":"maybe"}]),
            serde_json::json!([{"alias":"ready","state":"complete","serial":42}]),
        ] {
            assert_eq!(
                yubi_enrollment_dtos(malformed).unwrap_err().code,
                "invalid-response"
            );
        }

        let account = yubi_account_response(
            serde_json::json!({
                "alias":"ready_key",
                "username":"rae",
                "yubi_id_hex":"08".repeat(34),
                "subkey_id_hex":"0d".repeat(33),
                "user_chain_sequence":0,
                "management_enrolled":true
            }),
            "ready_key",
            None,
        )
        .unwrap();
        assert_eq!(account.yubi_id.len(), YUBI_ID_HEX_BYTES);
        assert_eq!(
            yubi_account_response(
                serde_json::json!({
                    "alias":"ready_key","username":"other",
                    "yubi_id_hex":"08".repeat(34),"subkey_id_hex":"0d".repeat(33),
                    "user_chain_sequence":0,"management_enrolled":true
                }),
                "ready_key",
                Some("rae"),
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
        for malformed in [
            serde_json::json!({
                "alias":"other","username":"rae","yubi_id_hex":"08".repeat(34),
                "subkey_id_hex":"0d".repeat(33),"user_chain_sequence":0,
                "management_enrolled":true
            }),
            serde_json::json!({
                "alias":"ready_key","username":"rae","yubi_id_hex":"04".repeat(34),
                "subkey_id_hex":"0d".repeat(33),"user_chain_sequence":0,
                "management_enrolled":true
            }),
            serde_json::json!({
                "alias":"ready_key","username":"rae","yubi_id_hex":"08".repeat(34),
                "subkey_id_hex":"0d".repeat(33),"user_chain_sequence":0,
                "management_enrolled":false
            }),
        ] {
            assert_eq!(
                yubi_account_response(malformed, "ready_key", None)
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }

        assert!(
            yubi_pin_status_response(serde_json::json!({"remaining":3,"blocked":false})).is_ok()
        );
        assert_eq!(
            yubi_pin_status_response(serde_json::json!({"remaining":0,"blocked":false}))
                .unwrap_err()
                .code,
            "invalid-response"
        );
        assert!(yubi_lifecycle_response(
            serde_json::json!({
                "alias":"ready_key","management_enrolled":true,"management_generation":1
            }),
            "ready_key"
        )
        .is_ok());
        for malformed in [
            serde_json::json!({
                "alias":"ready_key","management_enrolled":true,"management_generation":0
            }),
            serde_json::json!({
                "alias":"ready_key","management_enrolled":true,"management_generation":null
            }),
            serde_json::json!({
                "alias":"ready_key","management_enrolled":false,"management_generation":1
            }),
        ] {
            assert_eq!(
                yubi_lifecycle_response(malformed, "ready_key")
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }
        assert!(yubi_revocation_response(
            serde_json::json!({
                "alias":"ready_key","user_chain_sequence":0,"removed_local_credential":true
            }),
            "ready_key"
        )
        .is_ok());
        assert_eq!(
            yubi_revocation_response(
                serde_json::json!({
                    "alias":"ready_key","user_chain_sequence":0,
                    "removed_local_credential":false
                }),
                "ready_key"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        let software_current = vec![DeviceDto {
            id: "04".repeat(33),
            name: None,
            role: "owner",
            current: true,
        }];
        assert!(require_software_revocation_signer(&software_current).is_ok());
        let yubi_current = vec![DeviceDto {
            id: "08".repeat(34),
            name: None,
            role: "owner",
            current: true,
        }];
        assert_eq!(
            require_software_revocation_signer(&yubi_current)
                .unwrap_err()
                .code,
            "security-key-current"
        );
    }

    #[test]
    fn security_key_sync_and_lifecycle_reports_reject_malformed_success() {
        let sync = yubi_sync_response(
            serde_json::json!({
                "sync":{
                    "username":"rae","user_chain_sequence":4,"directories":2,"entries":3
                },
                "federation":[{
                    "local_profile":"work","local_team_alias":"engineering",
                    "refreshed":true,"deferred":null
                }]
            }),
            "work",
            true,
        )
        .unwrap();
        assert_eq!(sync.federation.len(), 1);
        for malformed in [
            serde_json::json!({
                "sync":{"username":"rae","user_chain_sequence":4,"directories":2,"entries":3},
                "federation":[{
                    "local_profile":"other","local_team_alias":"engineering",
                    "refreshed":true,"deferred":null
                }]
            }),
            serde_json::json!({
                "sync":{"username":"rae","user_chain_sequence":4,"directories":2,"entries":3},
                "federation":[{
                    "local_profile":"work","local_team_alias":"engineering",
                    "refreshed":false,"deferred":null
                }]
            }),
            serde_json::json!({
                "sync":{"username":"rae","user_chain_sequence":4,"directories":2,"entries":3},
                "federation":[{
                    "local_profile":"work","local_team_alias":"engineering",
                    "refreshed":true,"deferred":null,"invented":true
                }]
            }),
        ] {
            assert_eq!(
                yubi_sync_response(malformed, "work", true)
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }
        assert!(yubi_subkey_recovery_response(
            serde_json::json!({
                "alias":"ready_key","subkey_id_hex":"0d".repeat(33),"certificate_count":2
            }),
            "ready_key"
        )
        .is_ok());
        assert!(yubi_changed_response(
            serde_json::json!({"alias":"ready_key","changed":true}),
            "ready_key"
        )
        .is_ok());

        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        let error = yubi_revocation_response(
            serde_json::json!({
                "alias":"other","user_chain_sequence":9,"removed_local_credential":true
            }),
            "ready_key",
        )
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
        .unwrap_err();
        assert_eq!(error.code, "response-binding");
        assert!(error.ambiguous && error.fatal);
    }

    struct PhaseSixReadTransport {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for PhaseSixReadTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ListProfiles => {
                    Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
                }
                Operation::ListDevices { .. } => Ok(serde_json::json!([
                    {"id_hex":"04".repeat(33),"role":"owner","current":true},
                    {"id_hex":format!("04{}", "06".repeat(32)),"role":"owner","current":false}
                ])),
                Operation::ListBackupEnrollments { .. } => Ok(serde_json::json!([{
                    "backup_alias":"paper",
                    "account_alias":"personal",
                    "backup_id_hex":"10".repeat(33)
                }])),
                other => panic!("unexpected Phase 6 read operation {other:?}"),
            }
        }
    }

    #[test]
    fn phase_six_account_reads_resolve_profile_then_issue_one_bound_call() {
        let account = account_ref("work", "personal");
        let transport = PhaseSixReadTransport {
            calls: Mutex::new(Vec::new()),
        };
        assert_eq!(load_account_devices(&transport, &account).unwrap().len(), 2);
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::ListDevices {
                    profile: "work".to_owned(),
                    alias: "personal".to_owned()
                }
            ]
        );

        let transport = PhaseSixReadTransport {
            calls: Mutex::new(Vec::new()),
        };
        assert_eq!(
            load_backup_enrollments(&transport, &account).unwrap().len(),
            1
        );
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::ListBackupEnrollments {
                    profile: "work".to_owned(),
                    account_alias: "personal".to_owned()
                }
            ]
        );
    }

    struct YubiResumeTransport {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for YubiResumeTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ListProfiles => {
                    Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
                }
                Operation::ListPendingOperations { .. } => Ok(serde_json::json!([{
                    "kind":"yubi-enrollment","alias":"work_key","target":null
                }])),
                Operation::ResumeYubiAccount { .. } => Ok(serde_json::json!({
                    "alias":"work_key","username":"rae",
                    "yubi_id_hex":"08".repeat(34),"subkey_id_hex":"0d".repeat(33),
                    "user_chain_sequence":8,"management_enrolled":true
                })),
                other => panic!("unexpected security-key resume operation {other:?}"),
            }
        }
    }

    #[test]
    fn security_key_resume_re_resolves_one_pending_journal_before_one_attempt() {
        let transport = YubiResumeTransport {
            calls: Mutex::new(Vec::new()),
        };
        let value = execute_pending_operation(
            &transport,
            "work",
            PendingOperationKind::YubiEnrollment,
            "work_key",
            None,
            Operation::ResumeYubiAccount {
                profile: "work".to_owned(),
                alias: "work_key".to_owned(),
                pin: SecretString::new("123456"),
            },
        )
        .unwrap();
        assert!(yubi_account_response(value, "work_key", None).is_ok());
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::ListPendingOperations {
                    profile: "work".to_owned(),
                },
                Operation::ResumeYubiAccount {
                    profile: "work".to_owned(),
                    alias: "work_key".to_owned(),
                    pin: SecretString::new("123456"),
                },
            ]
        );
    }

    #[test]
    fn every_first_run_mutation_success_is_shape_and_request_bound() {
        assert!(backup_phrase_response(
            serde_json::json!({
                "backup_alias":"paper",
                "phrase":"abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon"
            }),
            "paper"
        )
        .is_ok());
        for malformed in [
            serde_json::json!({"backup_alias":"other","phrase":"abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon"}),
            serde_json::json!({"backup_alias":"paper","phrase":"not a recovery phrase"}),
            serde_json::json!({"backup_alias":"paper","phrase":"abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon 0 abandon","invented":true}),
        ] {
            assert_eq!(
                backup_phrase_response(malformed, "paper").unwrap_err().code,
                "invalid-response"
            );
        }
        assert!(account_sync_response(serde_json::json!({
            "username": "Sol",
            "user_chain_sequence": 0,
            "directories": 1,
            "entries": 4
        }))
        .is_ok());
        assert_eq!(
            account_sync_response(serde_json::json!({
                "username": "Sol",
                "user_chain_sequence": 1,
                "directories": 1,
                "entries": 4,
                "invented": true
            }))
            .unwrap_err()
            .code,
            "invalid-response"
        );
        assert!(passphrase_response(serde_json::json!({
            "generation": 1,
            "stretch_version": "v1",
            "verified": true
        }))
        .is_ok());
        for malformed in [
            serde_json::json!({
                "generation": 0,
                "stretch_version": "v1",
                "verified": true
            }),
            serde_json::json!({
                "generation": 1,
                "stretch_version": "test",
                "verified": true
            }),
            serde_json::json!({
                "generation": 1,
                "stretch_version": "v1",
                "verified": false
            }),
        ] {
            assert_eq!(
                passphrase_response(malformed).unwrap_err().code,
                "invalid-response"
            );
        }

        assert!(backup_commit_response(
            serde_json::json!({
                "backup_alias": "paper",
                "account_alias": "personal",
                "backup_id_hex": "10".repeat(33),
                "user_chain_sequence": 0
            }),
            "personal",
            "paper"
        )
        .is_ok());
        assert_eq!(
            backup_commit_response(
                serde_json::json!({
                    "backup_alias": "other",
                    "account_alias": "personal",
                    "backup_id_hex": "10".repeat(33),
                    "user_chain_sequence": 1
                }),
                "personal",
                "paper"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
        assert_eq!(
            backup_commit_response(
                serde_json::json!({
                    "backup_alias": "paper",
                    "account_alias": "personal",
                    "backup_id_hex": "04".repeat(33),
                    "user_chain_sequence": 1
                }),
                "personal",
                "paper"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        assert!(recovery_response(
            serde_json::json!({
                "alias": "recovered",
                "device_id_hex": "04".repeat(33),
                "user_chain_sequence": 0
            }),
            "recovered"
        )
        .is_ok());
        assert_eq!(
            recovery_response(
                serde_json::json!({
                    "alias": "different",
                    "device_id_hex": "04".repeat(33),
                    "user_chain_sequence": 1
                }),
                "recovered"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
        assert_eq!(
            recovery_response(
                serde_json::json!({
                    "alias": "recovered",
                    "device_id_hex": "05".repeat(33),
                    "user_chain_sequence": 1
                }),
                "recovered"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
    }

    #[test]
    fn malformed_post_mutation_success_is_ambiguous_fatal_and_requires_refresh() {
        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        let error = account_sync_response(serde_json::json!({
            "username": "",
            "user_chain_sequence": 1,
            "directories": 0,
            "entries": 0
        }))
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
        .unwrap_err();
        assert_eq!(error.code, "response-binding");
        assert!(error.ambiguous);
        assert!(error.fatal);
        assert!(!error.retryable);
        let blocked = state.begin_mutation().unwrap_err();
        assert_eq!(blocked.code, "ambiguous");
    }

    struct FirstRunTransport {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for FirstRunTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ListProfiles => {
                    Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
                }
                Operation::ListPendingOperations { .. } => Ok(serde_json::json!([{
                    "kind": "account-signup",
                    "alias": "personal",
                    "target": null
                }])),
                Operation::ResumeAccount { .. } => Ok(serde_json::json!({
                    "username": "sol",
                    "user_chain_sequence": 3,
                    "directories": 1,
                    "entries": 0
                })),
                other => panic!("unexpected first-run operation {other:?}"),
            }
        }
    }

    #[test]
    fn first_run_mutation_resolves_profile_then_issues_one_explicit_resume() {
        let transport = FirstRunTransport {
            calls: Mutex::new(Vec::new()),
        };
        execute_pending_operation(
            &transport,
            "work",
            PendingOperationKind::AccountSignup,
            "personal",
            None,
            Operation::ResumeAccount {
                profile: "work".to_owned(),
                alias: "personal".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::ListPendingOperations {
                    profile: "work".to_owned(),
                },
                Operation::ResumeAccount {
                    profile: "work".to_owned(),
                    alias: "personal".to_owned(),
                },
            ]
        );
    }

    struct ReadOnlyPrepareLoss {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for ReadOnlyPrepareLoss {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ListProfiles => {
                    Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
                }
                Operation::PrepareOwnerBackup { .. } => Err(foks_desktop::AgentError::Transport(
                    "agent disappeared".to_owned(),
                )),
                other => panic!("unexpected preparation operation {other:?}"),
            }
        }
    }

    #[test]
    fn backup_prepare_failure_is_read_only_and_never_mutation_ambiguous() {
        let transport = ReadOnlyPrepareLoss {
            calls: Mutex::new(Vec::new()),
        };
        let error = execute_read_profile_operation(
            &transport,
            "work",
            Operation::PrepareOwnerBackup {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
                backup_alias: "paper".to_owned(),
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "io");
        assert!(!error.ambiguous);
        assert_eq!(transport.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn download_streams_files_larger_than_the_reveal_limit_in_bound_chunks() {
        let total = 16 * 1024 * 1024 + 1;
        let item = download_item(total);
        let transport = DownloadTransport {
            total,
            calls: Mutex::new(Vec::new()),
        };
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("large.bin");
        download_to_path(&transport, &item, &destination).unwrap();
        assert_eq!(std::fs::metadata(destination).unwrap().len(), total);
        let calls = transport.calls.lock().unwrap();
        assert!(matches!(
            calls.first(),
            Some(Operation::ReadKv { version: 9, .. })
        ));
        assert!(calls[1..].iter().all(|call| matches!(
            call,
            Operation::ReadKvChunk { version: 9, length, .. } if *length <= DOWNLOAD_CHUNK_BYTES
        )));
        assert_eq!(
            calls.len(),
            1 + total.div_ceil(u64::from(DOWNLOAD_CHUNK_BYTES)) as usize
        );
    }

    #[test]
    fn catalog_dto_rejects_unknown_kinds_and_missing_non_directory_sizes() {
        let account = foks_agent_proto::AccountStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
        };
        let mut snapshot = CatalogSnapshot {
            profiles: vec!["foks.example".to_owned()],
            stores: vec![CatalogStoreSummary::Account {
                store: account.clone(),
            }],
            items: vec![CatalogItem {
                store: CatalogStoreRef::Account(account.clone()),
                metadata: foks_agent_proto::KvEntryMetadata {
                    path: "/bad".to_owned(),
                    node_type: "socket".to_owned(),
                    version: 1,
                    size: Some(1),
                    read_role: KvRole::Owner,
                    write_role: KvRole::Owner,
                },
            }],
            ..CatalogSnapshot::default()
        };
        assert_eq!(
            CatalogDto::from_snapshot(&snapshot).unwrap_err().code,
            "invalid-response"
        );
        snapshot.items[0].metadata.node_type = "small-file".to_owned();
        snapshot.items[0].metadata.size = None;
        assert_eq!(
            CatalogDto::from_snapshot(&snapshot).unwrap_err().code,
            "invalid-response"
        );
        snapshot.items[0].metadata.node_type = "directory".to_owned();
        assert_eq!(
            CatalogDto::from_snapshot(&snapshot).unwrap().items[0].size,
            0
        );

        snapshot.stores = vec![CatalogStoreSummary::Team {
            store: foks_agent_proto::TeamStoreRef {
                profile: "foks.example".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "group".to_owned(),
                team_id: "03".to_owned(),
            },
            kind: "mystery".to_owned(),
            name: Some("Group".to_owned()),
            active: true,
        }];
        assert_eq!(
            CatalogDto::from_snapshot(&snapshot).unwrap_err().code,
            "invalid-response"
        );
    }

    struct MutationTransport {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for MutationTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation);
            Ok(serde_json::Value::Null)
        }
    }

    #[test]
    fn item_role_strings_are_strict_and_account_defaults_cannot_be_overridden() {
        let account = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
        });
        assert_eq!(
            create_item_roles(&account, None, None).unwrap(),
            (KvRole::Owner, KvRole::Owner)
        );
        assert_eq!(
            create_item_roles(&account, Some("Owner"), Some("Owner"))
                .unwrap_err()
                .code,
            "invalid-request"
        );
        let team = CatalogStoreRef::Team(foks_agent_proto::TeamStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "03".repeat(33),
        });
        for roles in [(None, None), (Some("Owner"), None), (None, Some("Admin"))] {
            assert_eq!(
                create_item_roles(&team, roles.0, roles.1).unwrap_err().code,
                "invalid-request"
            );
        }
        assert_eq!(parse_item_role("Admin").unwrap(), KvRole::Admin);
        assert_eq!(
            parse_item_role("Member:-16384").unwrap(),
            KvRole::Member { visibility: -16384 }
        );
        for invalid in [
            "Member",
            "Member:32768",
            "Member:+1",
            "Member:00",
            "member:0",
            "Owner:0",
            " Admin",
        ] {
            assert_eq!(
                parse_item_role(invalid).unwrap_err().code,
                "invalid-request"
            );
        }
    }

    #[test]
    fn team_create_edit_link_folder_and_remove_transcripts_bind_roles_and_guards() {
        let team = foks_agent_proto::TeamStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "03".repeat(33),
        };
        let store = CatalogStoreRef::Team(team.clone());
        let (read_role, write_role) =
            create_item_roles(&store, Some("Member:0"), Some("Admin")).unwrap();
        let item = CatalogItem {
            store: store.clone(),
            metadata: foks_agent_proto::KvEntryMetadata {
                path: "/wifi/password".to_owned(),
                node_type: "small-file".to_owned(),
                version: 19,
                size: Some(3),
                read_role: KvRole::Member { visibility: -2 },
                write_role: KvRole::Admin,
            },
        };
        let transport = MutationTransport {
            calls: Mutex::new(Vec::new()),
        };
        execute_kv_mutation(
            &transport,
            set_create_mutation_roles(
                foks_desktop::create_kv_file_mutation(&store, "/new", b"new".to_vec()).unwrap(),
                read_role,
                write_role,
            )
            .unwrap(),
            MutationKind::Create,
        )
        .unwrap();
        foks_desktop::AgentTransport::call(
            &transport,
            set_create_operation_roles(
                foks_desktop::create_kv_symlink_operation(&store, "/docs", "/shared/docs").unwrap(),
                read_role,
                write_role,
            )
            .unwrap(),
        )
        .unwrap();
        foks_desktop::AgentTransport::call(
            &transport,
            set_create_operation_roles(
                foks_desktop::create_kv_directory_operation(&store, "/folder").unwrap(),
                read_role,
                write_role,
            )
            .unwrap(),
        )
        .unwrap();
        execute_kv_mutation(
            &transport,
            foks_desktop::edit_kv_file_mutation(&item, b"changed".to_vec()).unwrap(),
            MutationKind::Guarded,
        )
        .unwrap();
        foks_desktop::AgentTransport::call(&transport, remove_item_operation(&item).unwrap())
            .unwrap();
        let calls = transport.calls.lock().unwrap();
        assert_eq!(
            calls.as_slice(),
            &[
                Operation::PutKv {
                    store: KvStoreRef::Team(team.clone()),
                    path: "/new".to_owned(),
                    content: b"new".to_vec(),
                    read_role: KvRole::Member { visibility: 0 },
                    write_role: KvRole::Admin,
                    precondition: KvPrecondition::Create,
                },
                Operation::PutKvSymlink {
                    store: KvStoreRef::Team(team.clone()),
                    path: "/docs".to_owned(),
                    target: "/shared/docs".to_owned(),
                    read_role: KvRole::Member { visibility: 0 },
                    write_role: KvRole::Admin,
                    precondition: KvPrecondition::Create,
                },
                Operation::MkdirKv {
                    store: KvStoreRef::Team(team.clone()),
                    path: "/folder".to_owned(),
                    read_role: KvRole::Member { visibility: 0 },
                    write_role: KvRole::Admin,
                    precondition: KvPrecondition::Create,
                },
                Operation::PutKv {
                    store: KvStoreRef::Team(team.clone()),
                    path: "/wifi/password".to_owned(),
                    content: b"changed".to_vec(),
                    read_role: KvRole::Member { visibility: -2 },
                    write_role: KvRole::Admin,
                    precondition: KvPrecondition::ExactVersion { version: 19 },
                },
                Operation::RemoveKv {
                    store: KvStoreRef::Team(team.clone()),
                    path: "/wifi/password".to_owned(),
                    recursive: false,
                    precondition: KvPrecondition::ExactVersion { version: 19 },
                },
            ]
        );
        drop(calls);
        let mut file = item.clone();
        file.metadata.node_type = "file".to_owned();
        let replacement = file_edit_header(&file, 84 * 1024 * 1024).unwrap();
        assert_eq!(replacement.store, KvStoreRef::Team(team));
        assert_eq!(replacement.read_role, KvRole::Member { visibility: -2 });
        assert_eq!(replacement.write_role, KvRole::Admin);
        assert_eq!(
            replacement.precondition,
            KvPrecondition::ExactVersion { version: 19 }
        );
    }

    #[test]
    fn mutation_failures_have_stable_ui_codes() {
        let conflict = || foks_desktop::AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::Conflict,
            message: "precondition failed".to_owned(),
            fields: foks_agent_proto::ErrorFields::default(),
        };
        assert_eq!(
            map_mutation_error(conflict(), MutationKind::Create).code,
            "already-exists"
        );
        assert_eq!(
            map_mutation_error(conflict(), MutationKind::Guarded).code,
            "conflict"
        );
        let lease = map_mutation_error(
            foks_desktop::AgentError::Protocol {
                code: foks_agent_proto::ErrorCode::CapabilityDenied,
                message: "denied".to_owned(),
                fields: foks_agent_proto::ErrorFields {
                    capability: Some("kv".to_owned()),
                    ..Default::default()
                },
            },
            MutationKind::Guarded,
        );
        assert_eq!(lease.code, "capability-unavailable");
        assert_eq!(
            map_mutation_error(
                foks_desktop::AgentError::Transport("gone".to_owned()),
                MutationKind::Guarded,
            )
            .code,
            "agent-lost"
        );
        let ambiguous = map_mutation_error(
            foks_desktop::AgentError::Ambiguous("unknown outcome".to_owned()),
            MutationKind::Guarded,
        );
        assert_eq!(ambiguous.code, "ambiguous");
        assert!(ambiguous.ambiguous);
        assert!(!ambiguous.retryable);
    }

    struct ConflictTransport {
        calls: AtomicU64,
    }

    impl foks_desktop::AgentTransport for ConflictTransport {
        fn call(
            &self,
            _operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Err(foks_desktop::AgentError::Protocol {
                code: foks_agent_proto::ErrorCode::Conflict,
                message: "precondition failed".to_owned(),
                fields: foks_agent_proto::ErrorFields::default(),
            })
        }
    }

    #[test]
    fn team_create_and_edit_conflicts_are_returned_after_one_attempt_without_retry() {
        let team = foks_agent_proto::TeamStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "03".repeat(33),
        };
        let store = CatalogStoreRef::Team(team.clone());
        let item = CatalogItem {
            store,
            metadata: foks_agent_proto::KvEntryMetadata {
                path: "/wifi/password".to_owned(),
                node_type: "small-file".to_owned(),
                version: 19,
                size: Some(3),
                read_role: KvRole::Owner,
                write_role: KvRole::Owner,
            },
        };
        let create_transport = ConflictTransport {
            calls: AtomicU64::new(0),
        };
        let create = set_create_mutation_roles(
            foks_desktop::create_kv_file_mutation(&item.store, "/new", b"new".to_vec()).unwrap(),
            KvRole::Member { visibility: 0 },
            KvRole::Admin,
        )
        .unwrap();
        let error =
            execute_kv_mutation(&create_transport, create, MutationKind::Create).unwrap_err();
        assert_eq!(error.code, "already-exists");
        assert_eq!(create_transport.calls.load(Ordering::Acquire), 1);

        let edit_transport = ConflictTransport {
            calls: AtomicU64::new(0),
        };
        let error = execute_kv_mutation(
            &edit_transport,
            foks_desktop::edit_kv_file_mutation(&item, b"new".to_vec()).unwrap(),
            MutationKind::Guarded,
        )
        .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(edit_transport.calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn text_values_cannot_silently_turn_into_streamed_files() {
        assert_eq!(
            take_text_value("x".repeat(MAXIMUM_TEXT_ITEM_BYTES + 1))
                .unwrap_err()
                .code,
            "invalid-request"
        );
        assert_eq!(
            take_text_value("x".repeat(MAXIMUM_TEXT_ITEM_BYTES))
                .unwrap()
                .len(),
            MAXIMUM_TEXT_ITEM_BYTES
        );
    }

    #[test]
    fn renderer_text_edits_accept_only_small_file_catalog_nodes() {
        let mut item = download_item(1);
        assert_eq!(
            require_text_item(&item).unwrap_err().code,
            "invalid-request"
        );
        item.metadata.node_type = "symlink".to_owned();
        assert_eq!(
            require_text_item(&item).unwrap_err().code,
            "invalid-request"
        );
        item.metadata.node_type = "small-file".to_owned();
        assert!(require_text_item(&item).is_ok());
    }

    #[test]
    fn native_file_replacement_accepts_both_file_encodings() {
        let mut item = download_item(1);
        assert!(require_file_item(&item).is_ok());
        item.metadata.node_type = "small-file".to_owned();
        assert!(require_file_item(&item).is_ok());
        item.metadata.node_type = "symlink".to_owned();
        assert_eq!(
            require_file_item(&item).unwrap_err().code,
            "invalid-request"
        );
    }

    #[test]
    fn renderer_remove_boundary_rejects_folders_and_is_never_recursive() {
        let file = download_item(1);
        assert!(matches!(
            remove_item_operation(&file).unwrap(),
            Operation::RemoveKv {
                recursive: false,
                precondition: KvPrecondition::ExactVersion { version: 9 },
                ..
            }
        ));
        let mut folder = file;
        folder.metadata.node_type = "directory".to_owned();
        assert_eq!(
            remove_item_operation(&folder).unwrap_err().code,
            "invalid-request"
        );
    }

    struct UploadTransport {
        calls: Mutex<Vec<Operation>>,
        header: Mutex<Option<KvUploadHeader>>,
        total_read: AtomicU64,
        largest_read: AtomicU64,
    }

    impl foks_desktop::AgentTransport for UploadTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation);
            Ok(serde_json::Value::Null)
        }

        fn put_kv_stream(
            &self,
            header: KvUploadHeader,
            reader: &mut dyn std::io::Read,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            let total = header.total_length;
            *self.header.lock().unwrap() = Some(header);
            let mut buffer = [0u8; 128 * 1024];
            let mut seen = 0u64;
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))?;
                if count == 0 {
                    break;
                }
                seen += count as u64;
                self.largest_read.fetch_max(count as u64, Ordering::AcqRel);
            }
            assert_eq!(seen, total);
            self.total_read.store(seen, Ordering::Release);
            Ok(serde_json::Value::Null)
        }
    }

    #[test]
    fn eighty_four_megabyte_drop_uses_only_the_bounded_stream_boundary() {
        const TOTAL: u64 = 84 * 1024 * 1024;
        let source = tempfile::NamedTempFile::new().unwrap();
        source.as_file().set_len(TOTAL).unwrap();
        let team = foks_agent_proto::TeamStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "03".repeat(33),
        };
        let store = CatalogStoreRef::Team(team.clone());
        let transport = UploadTransport {
            calls: Mutex::new(Vec::new()),
            header: Mutex::new(None),
            total_read: AtomicU64::new(0),
            largest_read: AtomicU64::new(0),
        };

        let header =
            file_create_header(&store, "/bundle.tar", 0, Some("Member:0"), Some("Admin")).unwrap();
        upload_file(&transport, header, source.path(), MutationKind::Create).unwrap();

        assert!(transport.calls.lock().unwrap().is_empty());
        let header = transport.header.lock().unwrap().clone().unwrap();
        assert_eq!(header.store, KvStoreRef::Team(team));
        assert_eq!(header.read_role, KvRole::Member { visibility: 0 });
        assert_eq!(header.write_role, KvRole::Admin);
        assert_eq!(header.total_length, TOTAL);
        assert_eq!(header.precondition, KvPrecondition::Create);
        assert_eq!(transport.total_read.load(Ordering::Acquire), TOTAL);
        assert!(transport.largest_read.load(Ordering::Acquire) <= 128 * 1024);
    }

    struct EarlyUploadLoss;

    impl foks_desktop::AgentTransport for EarlyUploadLoss {
        fn call(
            &self,
            _operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            unreachable!("file upload must use put_kv_stream")
        }

        fn put_kv_stream(
            &self,
            _header: KvUploadHeader,
            reader: &mut dyn std::io::Read,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            let mut first_frame = [0u8; 64 * 1024];
            reader
                .read_exact(&mut first_frame)
                .map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))?;
            Err(foks_desktop::AgentError::Transport(
                "socket disappeared before the next frame".to_owned(),
            ))
        }
    }

    #[test]
    fn early_transport_loss_with_an_unchanged_source_stays_agent_lost() {
        let source = tempfile::NamedTempFile::new().unwrap();
        source.as_file().set_len(1024 * 1024).unwrap();
        let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
        });
        let header = file_create_header(&store, "/bundle.tar", 0, None, None).unwrap();

        let error = upload_file(
            &EarlyUploadLoss,
            header,
            source.path(),
            MutationKind::Create,
        )
        .unwrap_err();

        assert_eq!(error.code, "agent-lost");
        assert!(error.fatal);
    }

    struct ResizeDuringUpload {
        path: PathBuf,
        length: u64,
    }

    impl foks_desktop::AgentTransport for ResizeDuringUpload {
        fn call(
            &self,
            _operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            unreachable!("file upload must use put_kv_stream")
        }

        fn put_kv_stream(
            &self,
            _header: KvUploadHeader,
            _reader: &mut dyn std::io::Read,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            OpenOptions::new()
                .write(true)
                .open(&self.path)
                .unwrap()
                .set_len(self.length)
                .unwrap();
            Err(foks_desktop::AgentError::Transport(
                "upload length no longer matches".to_owned(),
            ))
        }
    }

    #[test]
    fn same_handle_length_changes_are_classified_as_source_changes() {
        const ORIGINAL: u64 = 1024 * 1024;
        let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
        });
        for changed in [ORIGINAL / 2, ORIGINAL * 2] {
            let source = tempfile::NamedTempFile::new().unwrap();
            source.as_file().set_len(ORIGINAL).unwrap();
            let header = file_create_header(&store, "/bundle.tar", 0, None, None).unwrap();
            let error = upload_file(
                &ResizeDuringUpload {
                    path: source.path().to_owned(),
                    length: changed,
                },
                header,
                source.path(),
                MutationKind::Create,
            )
            .unwrap_err();
            assert_eq!(error.code, "upload-source-changed");
        }
    }

    #[test]
    fn renderer_can_only_consume_paths_from_the_latest_native_drop_once() {
        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        let first = PathBuf::from("/tmp/first");
        let second = PathBuf::from("/tmp/second");
        assert_eq!(
            state.record_drop_paths(std::slice::from_ref(&first)),
            vec!["/tmp/first"]
        );
        assert_eq!(state.take_drop_path("/tmp/first").unwrap(), first);
        assert_eq!(
            state.take_drop_path("/tmp/first").unwrap_err().code,
            "drop-not-authorized"
        );
        state.record_drop_paths(&[second]);
        assert_eq!(
            state.take_drop_path("/tmp/first").unwrap_err().code,
            "drop-not-authorized"
        );
        let third = PathBuf::from("/tmp/third");
        let fourth = PathBuf::from("/tmp/fourth");
        assert_eq!(
            state.record_drop_paths(&[third, fourth]),
            vec!["/tmp/third", "/tmp/fourth"]
        );
        for path in ["/tmp/third", "/tmp/fourth"] {
            assert_eq!(
                state.take_drop_path(path).unwrap_err().code,
                "drop-not-authorized"
            );
        }
    }

    #[test]
    fn catalog_activity_and_profile_health_gate_group_writes() {
        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        let team = foks_agent_proto::TeamStoreRef {
            profile: "foks.example".to_owned(),
            account_alias: "personal".to_owned(),
            team_alias: "engineering".to_owned(),
            team_id: "03".to_owned(),
        };
        let id = store_id(&CatalogStoreRef::Team(team.clone()));
        *state.catalog.lock().unwrap() = Some(CatalogSnapshot {
            stores: vec![CatalogStoreSummary::Team {
                store: team.clone(),
                kind: "named".to_owned(),
                name: Some("Engineering".to_owned()),
                active: false,
            }],
            items: vec![CatalogItem {
                store: CatalogStoreRef::Team(team.clone()),
                metadata: foks_agent_proto::KvEntryMetadata {
                    path: "/shared".to_owned(),
                    node_type: "small-file".to_owned(),
                    version: 4,
                    size: Some(1),
                    read_role: KvRole::Owner,
                    write_role: KvRole::Owner,
                },
            }],
            ..Default::default()
        });
        assert_eq!(
            state.selected_create_store(&id).unwrap_err().code,
            "inactive-group"
        );
        assert_eq!(
            state
                .selected_mutation_item(&id, "/shared", 4)
                .unwrap_err()
                .code,
            "inactive-group"
        );
        *state.catalog.lock().unwrap() = Some(CatalogSnapshot {
            stores: vec![CatalogStoreSummary::Team {
                store: team.clone(),
                kind: "named".to_owned(),
                name: Some("Engineering".to_owned()),
                active: true,
            }],
            items: vec![CatalogItem {
                store: CatalogStoreRef::Team(team.clone()),
                metadata: foks_agent_proto::KvEntryMetadata {
                    path: "/shared".to_owned(),
                    node_type: "small-file".to_owned(),
                    version: 4,
                    size: Some(1),
                    read_role: KvRole::Owner,
                    write_role: KvRole::Owner,
                },
            }],
            ..Default::default()
        });
        assert_eq!(
            state.selected_create_store(&id).unwrap(),
            CatalogStoreRef::Team(team.clone())
        );
        assert_eq!(
            state
                .selected_mutation_item(&id, "/shared", 4)
                .unwrap()
                .store,
            CatalogStoreRef::Team(team)
        );
        state
            .catalog
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .blocked_profiles
            .push("foks.example".to_owned());
        assert_eq!(
            state.selected_create_store(&id).unwrap_err().code,
            "capability-unavailable"
        );
        assert_eq!(
            state
                .selected_mutation_item(&id, "/shared", 4)
                .unwrap_err()
                .code,
            "capability-unavailable"
        );
    }

    fn account_ref(profile: &str, alias: &str) -> foks_agent_proto::AccountStoreRef {
        foks_agent_proto::AccountStoreRef {
            profile: profile.to_owned(),
            account_alias: alias.to_owned(),
        }
    }

    fn team_ref(
        profile: &str,
        account_alias: &str,
        team_alias: &str,
    ) -> foks_agent_proto::TeamStoreRef {
        foks_agent_proto::TeamStoreRef {
            profile: profile.to_owned(),
            account_alias: account_alias.to_owned(),
            team_alias: team_alias.to_owned(),
            team_id: format!("03{team_alias}"),
        }
    }

    fn phase_four_catalog(blocked_profiles: Vec<String>) -> CatalogSnapshot {
        CatalogSnapshot {
            profiles: vec!["work.example".to_owned(), "home.example".to_owned()],
            stores: vec![
                CatalogStoreSummary::Account {
                    store: account_ref("work.example", "personal"),
                },
                CatalogStoreSummary::Team {
                    store: team_ref("work.example", "personal", "engineering"),
                    kind: "named".to_owned(),
                    name: Some("Engineering".to_owned()),
                    active: true,
                },
                CatalogStoreSummary::Team {
                    store: team_ref("home.example", "home", "homelab"),
                    kind: "named".to_owned(),
                    name: Some("Homelab".to_owned()),
                    active: true,
                },
            ],
            blocked_profiles,
            ..CatalogSnapshot::default()
        }
    }

    fn phase_four_state(blocked_profiles: Vec<String>) -> AppState {
        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        *state.catalog.lock().unwrap() = Some(phase_four_catalog(blocked_profiles));
        state
    }

    #[test]
    fn known_store_metadata_never_authorizes_an_account_operation() {
        let state = phase_four_state(vec![]);
        let account = account_ref("work.example", "personal");
        *state.catalog.lock().unwrap() = Some(CatalogSnapshot {
            profiles: vec!["work.example".to_owned()],
            known_stores: vec![CatalogStoreSummary::Account {
                store: account.clone(),
            }],
            ..CatalogSnapshot::default()
        });
        assert_eq!(
            state
                .selected_account(&store_id(&CatalogStoreRef::Account(account)))
                .unwrap_err()
                .code,
            "store-not-found"
        );
    }

    struct AccountsTransport {
        calls: Mutex<Vec<Operation>>,
        response: serde_json::Value,
    }

    impl foks_desktop::AgentTransport for AccountsTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation);
            Ok(self.response.clone())
        }
    }

    /// Two profiles, one alias — the collision the UI navigates by StoreRef to
    /// avoid.
    ///
    /// An account alias is profile-local, so this Mac may hold `personal` on
    /// two servers at once. `store_id` must give those two accounts different
    /// identities, `selected_account` must resolve each to its own profile
    /// without either shadowing the other, and neither must be reachable by the
    /// alias alone. The catalog order must not enter into it: the shell's
    /// Settings and Join screens carry these strings in their address bar, and
    /// a refresh that reorders the catalog cannot be allowed to change which
    /// account an address means.
    #[test]
    fn two_profiles_sharing_an_account_alias_get_distinct_store_refs() {
        let home = account_ref("home.example", "personal");
        let work = account_ref("work.example", "personal");
        let home_id = store_id(&CatalogStoreRef::Account(home.clone()));
        let work_id = store_id(&CatalogStoreRef::Account(work.clone()));
        assert_ne!(home_id, work_id);
        // Deterministic: the same account produces the same id every time, so
        // an address survives a catalog refresh.
        assert_eq!(
            home_id,
            store_id(&CatalogStoreRef::Account(account_ref(
                "home.example",
                "personal"
            )))
        );

        let state = AppState::new(Arc::new(AgentHandle::new(
            "/tmp/unused-foks-agent.sock".into(),
        )));
        let catalog = |stores: Vec<CatalogStoreSummary>| CatalogSnapshot {
            profiles: vec!["home.example".to_owned(), "work.example".to_owned()],
            stores,
            ..CatalogSnapshot::default()
        };
        *state.catalog.lock().unwrap() = Some(catalog(vec![
            CatalogStoreSummary::Account {
                store: home.clone(),
            },
            CatalogStoreSummary::Account {
                store: work.clone(),
            },
        ]));

        let resolved = state.selected_account(&home_id).unwrap();
        assert_eq!(resolved.profile, "home.example");
        assert_eq!(resolved.account_alias, "personal");
        let resolved = state.selected_account(&work_id).unwrap();
        assert_eq!(resolved.profile, "work.example");
        assert_eq!(resolved.account_alias, "personal");

        // The alias on its own names nothing.
        assert_eq!(
            state.selected_account("personal").unwrap_err().code,
            "store-not-found"
        );

        // The other catalog order answers identically.
        *state.catalog.lock().unwrap() = Some(catalog(vec![
            CatalogStoreSummary::Account {
                store: work.clone(),
            },
            CatalogStoreSummary::Account {
                store: home.clone(),
            },
        ]));
        assert_eq!(
            state.selected_account(&home_id).unwrap().profile,
            "home.example"
        );
        assert_eq!(
            state.selected_account(&work_id).unwrap().profile,
            "work.example"
        );

        // And an account that has left the catalog is reported gone rather than
        // resolved to the one that still shares its alias.
        *state.catalog.lock().unwrap() = Some(catalog(vec![CatalogStoreSummary::Account {
            store: home.clone(),
        }]));
        assert_eq!(
            state.selected_account(&work_id).unwrap_err().code,
            "store-not-found"
        );
    }

    #[test]
    fn account_projection_is_bound_to_catalog_identities() {
        let catalog = CatalogSnapshot {
            profiles: vec!["work.example".to_owned()],
            stores: vec![
                CatalogStoreSummary::Account {
                    store: account_ref("work.example", "personal"),
                },
                CatalogStoreSummary::Account {
                    store: account_ref("work.example", "automation"),
                },
            ],
            ..CatalogSnapshot::default()
        };
        let transport = AccountsTransport {
            calls: Mutex::new(Vec::new()),
            response: serde_json::json!([
                {"profile":"work.example","alias":"personal","username":"rae.chen"},
                {"profile":"work.example","alias":"automation","username":"deploy-bot"}
            ]),
        };
        let accounts = load_accounts(&transport, &catalog).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].alias, "automation");
        assert_eq!(accounts[1].username, "rae.chen");
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[Operation::ListAccounts {
                profile: "work.example".to_owned()
            }]
        );

        let unknown = AccountsTransport {
            calls: Mutex::new(Vec::new()),
            response: serde_json::json!([
                {"profile":"work.example","alias":"personal","username":"rae.chen"},
                {"profile":"work.example","alias":"outside","username":"mallory"}
            ]),
        };
        assert_eq!(
            load_accounts(&unknown, &catalog).unwrap_err().code,
            "invalid-response"
        );

        for malformed in [
            serde_json::json!([
                {"profile":"work.example","alias":"personal","username":"rae.chen","extra":true},
                {"profile":"work.example","alias":"automation","username":"deploy-bot"}
            ]),
            serde_json::json!([
                {"profile":"work.example","alias":"personal","username":"rae\nchen"},
                {"profile":"work.example","alias":"automation","username":"deploy-bot"}
            ]),
        ] {
            let transport = AccountsTransport {
                calls: Mutex::new(Vec::new()),
                response: malformed,
            };
            assert_eq!(
                load_accounts(&transport, &catalog).unwrap_err().code,
                "invalid-response"
            );
        }
    }

    struct MixedAccountsTransport {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for MixedAccountsTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ListAccounts { profile } if profile == "available" => Ok(
                    serde_json::json!([{"profile":"available","alias":"personal","username":"rae"}]),
                ),
                Operation::ListAccounts { profile } => {
                    panic!("blocked profile {profile} must not be read")
                }
                other => panic!("unexpected account operation {other:?}"),
            }
        }
    }

    #[test]
    fn account_projection_skips_blocked_profiles_and_keeps_available_accounts() {
        let catalog = CatalogSnapshot {
            profiles: vec!["available".to_owned(), "blocked".to_owned()],
            stores: vec![
                CatalogStoreSummary::Account {
                    store: account_ref("available", "personal"),
                },
                CatalogStoreSummary::Account {
                    store: account_ref("blocked", "work"),
                },
            ],
            blocked_profiles: vec!["blocked".to_owned()],
            ..CatalogSnapshot::default()
        };
        let transport = MixedAccountsTransport {
            calls: Mutex::new(Vec::new()),
        };

        let accounts = load_accounts(&transport, &catalog).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].profile, "available");
        assert_eq!(accounts[0].alias, "personal");
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[Operation::ListAccounts {
                profile: "available".to_owned(),
            }]
        );
    }

    #[test]
    fn roster_and_federation_responses_fail_closed() {
        let party_id = "01".repeat(33);
        let remote_host_id = "02".repeat(33);
        let remote_team_id = "03".repeat(33);
        let operation_id = "07".repeat(16);
        let good = MemberResponse {
            username: Some("dana.okafor".to_owned()),
            party_id_hex: party_id.clone(),
            scoped_host_id_hex: None,
            party_kind: "user".to_owned(),
            source_role: MemberRole::Member { visibility: 0 },
            destination_role: MemberRole::Member { visibility: 0 },
            generation: 5,
            locally_manageable: true,
        };
        assert_eq!(party_dtos("team", vec![good]).unwrap().len(), 1);
        let malformed_user = MemberResponse {
            username: Some("dana.okafor".to_owned()),
            party_id_hex: "01DANA".to_owned(),
            scoped_host_id_hex: None,
            party_kind: "user".to_owned(),
            source_role: MemberRole::Member { visibility: 0 },
            destination_role: MemberRole::Member { visibility: 0 },
            generation: 5,
            locally_manageable: true,
        };
        assert_eq!(
            party_dtos("team", vec![malformed_user]).unwrap_err().code,
            "invalid-response"
        );
        for (party_kind, party_id_hex, username, scoped_host_id_hex) in [
            (
                "user",
                "04".repeat(33),
                Some("dana.okafor".to_owned()),
                None,
            ),
            ("named-team", "14".repeat(33), None, None),
            ("ad-hoc-team", "03".repeat(33), None, None),
            ("named-team", "03".repeat(33), None, Some("09".repeat(33))),
        ] {
            let wrong_entity_type = MemberResponse {
                username,
                party_id_hex,
                scoped_host_id_hex,
                party_kind: party_kind.to_owned(),
                source_role: MemberRole::Member { visibility: 0 },
                destination_role: MemberRole::Member { visibility: 0 },
                generation: 5,
                locally_manageable: false,
            };
            assert_eq!(
                party_dtos("team", vec![wrong_entity_type])
                    .unwrap_err()
                    .code,
                "invalid-response"
            );
        }
        assert_eq!(
            party_dtos(
                "team",
                vec![MemberResponse {
                    username: None,
                    party_id_hex: "14".repeat(33),
                    scoped_host_id_hex: Some("02".repeat(33)),
                    party_kind: "ad-hoc-team".to_owned(),
                    source_role: MemberRole::Member { visibility: 0 },
                    destination_role: MemberRole::Member { visibility: 0 },
                    generation: 5,
                    locally_manageable: false,
                }]
            )
            .unwrap()
            .len(),
            1
        );
        let unsupported = MemberResponse {
            username: None,
            party_id_hex: "04".repeat(33),
            scoped_host_id_hex: None,
            party_kind: "device".to_owned(),
            source_role: MemberRole::Owner,
            destination_role: MemberRole::Owner,
            generation: 1,
            locally_manageable: false,
        };
        assert_eq!(
            party_dtos("team", vec![unsupported]).unwrap_err().code,
            "invalid-response"
        );
        let mislabeled_group = MemberResponse {
            username: Some("looks-like-a-person".to_owned()),
            party_id_hex: "03".repeat(33),
            scoped_host_id_hex: Some("09".repeat(33)),
            party_kind: "named-team".to_owned(),
            source_role: MemberRole::Owner,
            destination_role: MemberRole::Member { visibility: 0 },
            generation: 3,
            locally_manageable: false,
        };
        assert_eq!(
            party_dtos("team", vec![mislabeled_group]).unwrap_err().code,
            "invalid-response"
        );
        let unsupported_destination = FederationResponse {
            local_team_alias: "engineering".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: remote_host_id.clone(),
            remote_team_id_hex: remote_team_id.clone(),
            destination: MemberRole::Admin,
            operation_id_hex: Some(operation_id.clone()),
            active: true,
        };
        assert_eq!(
            federation_dtos(
                "team",
                "work.example",
                "engineering",
                vec![unsupported_destination],
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
        for (wrong_host, wrong_team) in [
            ("09".repeat(33), remote_team_id.clone()),
            (remote_host_id.clone(), "04".repeat(33)),
        ] {
            let wrong_entity_type = FederationResponse {
                local_team_alias: "engineering".to_owned(),
                remote_profile: "home.example".to_owned(),
                remote_team_alias: "homelab".to_owned(),
                remote_host_id_hex: wrong_host,
                remote_team_id_hex: wrong_team,
                destination: MemberRole::Member { visibility: 0 },
                operation_id_hex: None,
                active: true,
            };
            assert_eq!(
                federation_dtos(
                    "team",
                    "work.example",
                    "engineering",
                    vec![wrong_entity_type],
                )
                .unwrap_err()
                .code,
                "invalid-response"
            );
        }
        let uppercase_operation_id = FederationResponse {
            local_team_alias: "engineering".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: remote_host_id.clone(),
            remote_team_id_hex: remote_team_id.clone(),
            destination: MemberRole::Member { visibility: 0 },
            operation_id_hex: Some("AA".repeat(16)),
            active: false,
        };
        assert_eq!(
            federation_dtos(
                "team",
                "work.example",
                "engineering",
                vec![uppercase_operation_id],
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
        let wrong_local_team = FederationResponse {
            local_team_alias: "some-other-group".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: remote_host_id.clone(),
            remote_team_id_hex: remote_team_id.clone(),
            destination: MemberRole::Member { visibility: 0 },
            operation_id_hex: Some(operation_id),
            active: true,
        };
        assert_eq!(
            federation_dtos(
                "team",
                "work.example",
                "engineering",
                vec![wrong_local_team],
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
        let empty_optional_id = FederationResponse {
            local_team_alias: "engineering".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: remote_host_id.clone(),
            remote_team_id_hex: remote_team_id.clone(),
            destination: MemberRole::Member { visibility: 0 },
            operation_id_hex: Some(String::new()),
            active: false,
        };
        assert_eq!(
            federation_dtos(
                "team",
                "work.example",
                "engineering",
                vec![empty_optional_id],
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );

        let good_federation = FederationResponse {
            local_team_alias: "engineering".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: remote_host_id,
            remote_team_id_hex: remote_team_id,
            destination: MemberRole::Member { visibility: 0 },
            operation_id_hex: Some("0a".repeat(16)),
            active: true,
        };
        assert_eq!(
            federation_dtos("team", "work.example", "engineering", vec![good_federation])
                .unwrap()
                .len(),
            1
        );
        let good_ad_hoc_federation = FederationResponse {
            local_team_alias: "engineering".to_owned(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "friends".to_owned(),
            remote_host_id_hex: "02".repeat(33),
            remote_team_id_hex: "14".repeat(33),
            destination: MemberRole::Member { visibility: -2 },
            operation_id_hex: None,
            active: true,
        };
        assert_eq!(
            federation_dtos(
                "team",
                "work.example",
                "engineering",
                vec![good_ad_hoc_federation],
            )
            .unwrap()
            .len(),
            1
        );

        assert!(serde_json::from_value::<MemberResponse>(serde_json::json!({
            "username":"dana.okafor",
            "party_id_hex":party_id,
            "scoped_host_id_hex":null,
            "party_kind":"user",
            "source_role":{"member":{"visibility":0}},
            "destination_role":{"member":{"visibility":0}},
            "generation":5,
            "locally_manageable":true,
            "extra":true
        }))
        .is_err());
        assert!(serde_json::from_value::<MemberResponse>(serde_json::json!({
            "username":"dana.okafor",
            "party_id_hex":"01".repeat(33),
            "scoped_host_id_hex":null,
            "party_kind":"user",
            "source_role":{"member":{"visibility":0,"invented":true}},
            "destination_role":{"member":{"visibility":0}},
            "generation":5,
            "locally_manageable":true
        }))
        .is_err());
        assert!(
            serde_json::from_value::<FederationResponse>(serde_json::json!({
                "local_team_alias":"engineering",
                "remote_profile":"home.example",
                "remote_team_alias":"homelab",
                "remote_host_id_hex":"02".repeat(33),
                "remote_team_id_hex":"03".repeat(33),
                "destination":{"member":{"visibility":0}},
                "operation_id_hex":"07".repeat(16),
                "active":true,
                "extra":true
            }))
            .is_err()
        );

        assert_eq!(
            require_response_row_cap(
                &serde_json::Value::Array(
                    (0..=MAXIMUM_FIRST_RUN_ROWS)
                        .map(|_| serde_json::Value::Null)
                        .collect()
                ),
                "entries"
            )
            .unwrap_err()
            .code,
            "invalid-response"
        );
    }

    #[test]
    fn strict_demotion_orders_member_visibility_below_admin_and_owner() {
        assert!(MemberRole::Member { visibility: -1 }
            .is_strictly_lower_than(MemberRole::Member { visibility: 0 }));
        assert!(!MemberRole::Member { visibility: 0 }
            .is_strictly_lower_than(MemberRole::Member { visibility: 0 }));
        assert!(!MemberRole::Member { visibility: 1 }
            .is_strictly_lower_than(MemberRole::Member { visibility: 0 }));
        assert!(MemberRole::Member {
            visibility: i16::MAX
        }
        .is_strictly_lower_than(MemberRole::Admin));
        assert!(MemberRole::Admin.is_strictly_lower_than(MemberRole::Owner));
        assert!(!MemberRole::Owner.is_strictly_lower_than(MemberRole::Admin));
        assert_eq!(
            serde_json::from_value::<RoleInput>(serde_json::json!({
                "role": "Member",
                "visibility": -16384
            }))
            .unwrap(),
            RoleInput::Member { visibility: -16384 }
        );
        assert!(serde_json::from_value::<RoleInput>(serde_json::json!({
            "role": "Admin",
            "visibility": 0
        }))
        .is_err());
        assert_eq!(
            serde_json::from_value::<GroupKindInput>(serde_json::json!("adhoc")).unwrap(),
            GroupKindInput::Adhoc
        );
    }

    #[test]
    fn group_operation_transcripts_use_only_resolved_identities() {
        let account = account_ref("work.example", "personal");
        assert_eq!(
            create_group_operation(
                account.clone(),
                "design-systems",
                "Design Systems",
                GroupKindInput::Named,
            )
            .unwrap(),
            Operation::CreateTeam {
                profile: "work.example".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "design-systems".to_owned(),
                name: "Design Systems".to_owned(),
                kind: TeamKind::Named,
            }
        );
        assert_eq!(
            create_group_operation(
                account,
                "weekend-project",
                "renderer name is not sent",
                GroupKindInput::Adhoc,
            )
            .unwrap(),
            Operation::CreateTeam {
                profile: "work.example".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "weekend-project".to_owned(),
                name: String::new(),
                kind: TeamKind::AdHoc,
            }
        );

        let team = team_ref("work.example", "personal", "engineering");
        assert!(matches!(
            add_group_member_operation(team.clone(), "jules.park", RoleInput::Owner).unwrap(),
            Operation::AddTeamMember {
                profile,
                team_alias,
                username,
                role: TeamRole::Owner,
                visibility: 0,
            } if profile == "work.example" && team_alias == "engineering" && username == "jules.park"
        ));
        assert!(matches!(
            demote_group_member_operation(
                team.clone(),
                "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                MemberRole::Owner,
                RoleInput::Member { visibility: -2 },
            )
            .unwrap(),
            Operation::DemoteTeamMember {
                party_id_hex,
                role: TeamRole::Member,
                visibility: -2,
                ..
            } if party_id_hex == "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert_eq!(
            demote_group_member_operation(
                team.clone(),
                "dana.okafor",
                MemberRole::Member { visibility: 0 },
                RoleInput::Admin,
            )
            .unwrap_err()
            .code,
            "not-a-demotion"
        );
        assert!(matches!(
            remove_group_member_operation(
                team.clone(),
                "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            Operation::RemoveTeamMember { party_id_hex, .. }
                if party_id_hex == "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert_eq!(
            remove_group_member_operation(team.clone(), "sam.ortiz")
                .unwrap_err()
                .code,
            "invalid-request"
        );
        assert!(matches!(
            admit_group_operation(
                team,
                team_ref("home.example", "home", "homelab"),
                -3,
            ),
            Operation::AdmitFederatedTeam {
                local_profile,
                remote_profile,
                role: FederationRole::Member,
                visibility: -3,
                ..
            } if local_profile == "work.example" && remote_profile == "home.example"
        ));
    }

    #[test]
    fn member_targets_require_fresh_local_non_self_user_facts() {
        let state = phase_four_state(Vec::new());
        let local = team_ref("work.example", "personal", "engineering");
        let local_id = store_id(&CatalogStoreRef::Team(local));
        let account = account_ref("work.example", "personal");
        let account_id = store_id(&CatalogStoreRef::Account(account));
        state.accounts.lock().unwrap().insert(
            account_id.clone(),
            AccountDto {
                store: account_id,
                profile: "work.example".to_owned(),
                alias: "personal".to_owned(),
                username: "rae.chen".to_owned(),
            },
        );
        let party = |username: &str, manageable: bool| PartyDto {
            store: local_id.clone(),
            username: Some(username.to_owned()),
            party_kind: "user".to_owned(),
            generation: 4,
            locally_manageable: manageable,
            party_id_hex: format!("01{username}"),
            scoped_host_id_hex: None,
            source_role: KvRole::Member { visibility: 0 }.into(),
            destination_role: KvRole::Member { visibility: 0 }.into(),
        };
        state.rosters.lock().unwrap().insert(
            local_id.clone(),
            vec![
                party("rae.chen", true),
                party("dana.okafor", true),
                party("deploy-bot", false),
            ],
        );
        assert_eq!(
            state
                .selected_member_target(&local_id, "dana.okafor")
                .unwrap()
                .2,
            MemberRole::Member { visibility: 0 }
        );
        for username in ["rae.chen", "deploy-bot", "missing"] {
            assert_eq!(
                state
                    .selected_member_target(&local_id, username)
                    .unwrap_err()
                    .code,
                "member-not-actionable"
            );
        }
        state.rosters.lock().unwrap().clear();
        assert_eq!(
            state
                .selected_member_target(&local_id, "dana.okafor")
                .unwrap_err()
                .code,
            "roster-required"
        );
    }

    #[test]
    fn blocked_profiles_stop_group_reads_and_mutations_before_transport() {
        let state = phase_four_state(vec!["work.example".to_owned()]);
        let local_id = store_id(&CatalogStoreRef::Team(team_ref(
            "work.example",
            "personal",
            "engineering",
        )));
        assert_eq!(
            state.selected_team(&local_id).unwrap_err().code,
            "capability-unavailable"
        );
        assert_eq!(
            state
                .selected_active_team_for_mutation(&local_id)
                .unwrap_err()
                .code,
            "capability-unavailable"
        );
        let account_id = store_id(&CatalogStoreRef::Account(account_ref(
            "work.example",
            "personal",
        )));
        assert_eq!(
            state.selected_account(&account_id).unwrap_err().code,
            "capability-unavailable"
        );

        let state = phase_four_state(Vec::new());
        let household = team_ref("work.example", "personal", "household");
        let household_id = store_id(&CatalogStoreRef::Team(household.clone()));
        state
            .catalog
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .stores
            .push(CatalogStoreSummary::Team {
                store: household,
                kind: "ad-hoc".to_owned(),
                name: None,
                active: true,
            });
        assert_eq!(
            state
                .selected_active_team_for_mutation(&household_id)
                .unwrap_err()
                .code,
            "group-management-unavailable"
        );
    }

    #[test]
    fn admission_rerun_requires_one_retained_inactive_operation_and_fresh_remote() {
        let state = phase_four_state(Vec::new());
        let local_id = store_id(&CatalogStoreRef::Team(team_ref(
            "work.example",
            "personal",
            "engineering",
        )));
        let operation_id = "07".repeat(16);
        let entry = FederationEntryDto {
            store: local_id.clone(),
            remote_profile: "home.example".to_owned(),
            remote_team_alias: "homelab".to_owned(),
            remote_host_id_hex: "02".repeat(33),
            remote_team_id_hex: "03".repeat(33),
            destination: KvRole::Member { visibility: -2 }.into(),
            operation_id_hex: Some(operation_id.clone()),
            active: false,
        };
        state
            .federations
            .lock()
            .unwrap()
            .insert(local_id.clone(), vec![entry.clone()]);
        let (local, retained) = state
            .selected_inactive_admission(&local_id, &operation_id)
            .unwrap();
        assert!(matches!(
            rerun_group_admission_operation(local, retained).unwrap(),
            Operation::AdmitFederatedTeam {
                remote_profile,
                remote_team_alias,
                visibility: -2,
                ..
            } if remote_profile == "home.example" && remote_team_alias == "homelab"
        ));
        assert_eq!(
            state
                .selected_inactive_admission(&local_id, "renderer-chosen")
                .unwrap_err()
                .code,
            "invalid-request"
        );
        let mut no_operation = entry.clone();
        no_operation.operation_id_hex = None;
        state
            .federations
            .lock()
            .unwrap()
            .insert(local_id.clone(), vec![no_operation]);
        assert_eq!(
            state
                .selected_inactive_admission(&local_id, &operation_id)
                .unwrap_err()
                .code,
            "admission-not-resumable"
        );
        state
            .federations
            .lock()
            .unwrap()
            .insert(local_id.clone(), vec![entry.clone()]);

        *state.catalog.lock().unwrap() = Some(phase_four_catalog(vec!["home.example".to_owned()]));
        assert_eq!(
            state
                .selected_inactive_admission(&local_id, &operation_id)
                .unwrap_err()
                .code,
            "capability-unavailable"
        );

        *state.catalog.lock().unwrap() = Some(phase_four_catalog(Vec::new()));
        let mut duplicate = entry;
        duplicate.remote_team_alias = "other".to_owned();
        state
            .federations
            .lock()
            .unwrap()
            .insert(local_id.clone(), vec![duplicate.clone(), duplicate]);
        assert_eq!(
            state
                .selected_inactive_admission(&local_id, &operation_id)
                .unwrap_err()
                .code,
            "admission-not-resumable"
        );
    }

    #[test]
    fn stale_group_reads_cannot_repopulate_authorization_caches() {
        let state = phase_four_state(Vec::new());
        state.catalog_generation.store(2, Ordering::Release);
        let error = state
            .retain_roster(1, "old-store".to_owned(), &[])
            .unwrap_err();
        assert_eq!(error.code, "catalog-required");
        assert!(state.rosters.lock().unwrap().is_empty());

        state.catalog_generation.store(2, Ordering::Release);
        state.accounts.lock().unwrap().insert(
            "old-account".to_owned(),
            AccountDto {
                store: "old-account".to_owned(),
                profile: "work.example".to_owned(),
                alias: "personal".to_owned(),
                username: "rae.chen".to_owned(),
            },
        );
        state
            .federations
            .lock()
            .unwrap()
            .insert("old-store".to_owned(), Vec::new());
        state.invalidate_catalog();
        assert!(state.accounts.lock().unwrap().is_empty());
        assert!(state.federations.lock().unwrap().is_empty());
    }

    #[test]
    fn device_removal_preflight_refuses_current_unknown_and_duplicate_ids() {
        let state = phase_four_state(Vec::new());
        let account_id = store_id(&CatalogStoreRef::Account(account_ref(
            "work.example",
            "personal",
        )));
        let current = DeviceDto {
            id: "04".repeat(33),
            name: Some("This Mac".to_owned()),
            role: "owner",
            current: true,
        };
        let other = DeviceDto {
            id: format!("04{}", "06".repeat(32)),
            name: Some("Spare".to_owned()),
            role: "owner",
            current: false,
        };
        let yubi = DeviceDto {
            id: "08".repeat(34),
            name: None,
            role: "owner",
            current: false,
        };
        assert_eq!(
            state.selected_account_dto(&account_id).unwrap_err().code,
            "accounts-required"
        );
        state.accounts.lock().unwrap().insert(
            account_id.clone(),
            AccountDto {
                store: account_id.clone(),
                profile: "work.example".to_owned(),
                alias: "personal".to_owned(),
                username: "rae".to_owned(),
            },
        );
        assert_eq!(
            state.selected_account_dto(&account_id).unwrap().username,
            "rae"
        );
        state.devices.lock().unwrap().insert(
            account_id.clone(),
            vec![current.clone(), other.clone(), yubi.clone()],
        );
        assert_eq!(
            state
                .selected_device_target(&account_id, &current.id)
                .unwrap_err()
                .code,
            "current-device"
        );
        assert_eq!(
            state
                .selected_device_target(&account_id, &format!("04{}", "07".repeat(32)))
                .unwrap_err()
                .code,
            "device-not-found"
        );
        assert_eq!(
            state
                .selected_device_target(&account_id, &other.id)
                .unwrap(),
            account_ref("work.example", "personal")
        );
        assert_eq!(
            state
                .selected_device_target(&account_id, &yubi.id)
                .unwrap_err()
                .code,
            "device-not-removable"
        );
        state
            .devices
            .lock()
            .unwrap()
            .insert(account_id.clone(), vec![other.clone(), other]);
        assert_eq!(
            state
                .selected_device_target(&account_id, &format!("04{}", "06".repeat(32)))
                .unwrap_err()
                .code,
            "invalid-response"
        );
        state.devices.lock().unwrap().clear();
        assert_eq!(
            state
                .selected_device_target(&account_id, &format!("04{}", "06".repeat(32)))
                .unwrap_err()
                .code,
            "devices-required"
        );
    }

    struct PairingResumeTransport {
        calls: Mutex<Vec<Operation>>,
    }

    impl foks_desktop::AgentTransport for PairingResumeTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            self.calls.lock().unwrap().push(operation.clone());
            match operation {
                Operation::ListProfiles => {
                    Ok(serde_json::Value::Array(vec![test_profile_value("work")]))
                }
                Operation::ListPendingOperations { .. } => Ok(serde_json::json!([{
                    "kind":"pairing-acceptance",
                    "alias":"paired",
                    "target":null
                }])),
                Operation::ResumeDevicePairingAcceptance { .. } => Ok(serde_json::json!({
                    "alias":"paired",
                    "device_id_hex":"04".repeat(33),
                    "user_chain_sequence":7
                })),
                other => panic!("unexpected pairing resume operation {other:?}"),
            }
        }
    }

    #[test]
    fn pairing_resume_rechecks_one_authenticated_pending_identity() {
        let transport = PairingResumeTransport {
            calls: Mutex::new(Vec::new()),
        };
        let value = execute_pending_operation(
            &transport,
            "work",
            PendingOperationKind::PairingAcceptance,
            "paired",
            None,
            Operation::ResumeDevicePairingAcceptance {
                profile: "work".to_owned(),
                target_alias: "paired".to_owned(),
            },
        )
        .unwrap();
        assert!(device_provision_response(value, "paired").is_ok());
        assert_eq!(
            transport.calls.lock().unwrap().as_slice(),
            &[
                Operation::ListProfiles,
                Operation::ListPendingOperations {
                    profile: "work".to_owned()
                },
                Operation::ResumeDevicePairingAcceptance {
                    profile: "work".to_owned(),
                    target_alias: "paired".to_owned()
                }
            ]
        );
    }

    #[test]
    fn malformed_reset_success_is_post_mutation_ambiguity() {
        let state = phase_four_state(Vec::new());
        let value = serde_json::json!({"profile":"other","hard_state_reset":true});
        let error = reset_result_response(value, "work")
            .map_err(|error| ambiguous_mutation_response(&state, error.message))
            .unwrap_err();
        assert_eq!(error.code, "response-binding");
        assert!(error.ambiguous && error.fatal);
        assert_eq!(state.begin_mutation().unwrap_err().code, "ambiguous");
    }
}
