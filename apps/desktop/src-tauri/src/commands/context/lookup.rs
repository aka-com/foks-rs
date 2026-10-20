//! Resolution of renderer-selected stores, items, members and federation
//! targets against the accepted catalog and cached facts.

use super::facts::catalog_changed_during_group_read;
use super::{AppState, LockExt};
use crate::agent::AgentError;
use crate::commands::accounts::AccountDto;
use crate::commands::groups::{
    federated_membership_not_resumable, member_not_actionable, member_role_from_dto,
    FederationEntryDto, MemberRole,
};
use crate::commands::store_key::{store_id, StoreKey};
use crate::commands::validation::{
    invalid_request, invalid_response, require_profile_available, required_field,
    valid_device_member_id_hex, valid_operation_id_hex, valid_typed_entity_id_hex,
    AD_HOC_TEAM_ID_PREFIX, DEVICE_ID_PREFIX, HOST_ID_PREFIX, NAMED_TEAM_ID_PREFIX,
};
use foks_desktop::{CatalogItem, CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};

impl AppState {
    /// The listed version of the item at `path`. Commands resolve through
    /// the dirent-bound selections below; tests use this one directly.
    #[cfg(test)]
    pub(in crate::commands) fn selected_item(
        &self,
        store: &str,
        path: &str,
        version: u64,
    ) -> Result<CatalogItem, AgentError> {
        self.item_for_key(StoreKey::parse(store).as_ref(), path, version)
    }

    /// [`Self::selected_item`] for an identity the caller has already
    /// parsed; `None` is an identity that did not parse.
    fn item_for_key(
        &self,
        key: Option<&StoreKey>,
        path: &str,
        version: u64,
    ) -> Result<CatalogItem, AgentError> {
        let catalog = self.catalog.lock_unpoisoned();
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before reading an item.",
                true,
            ));
        };
        let item = key.and_then(|key| {
            catalog
                .items
                .iter()
                .find(|item| item.store == *key.store() && item.metadata.path == path)
        });
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
    fn retired_store_error(&self, key: &StoreKey) -> Option<AgentError> {
        let profile = key.profile();
        let retired = self.retired_profiles.lock_unpoisoned().contains(profile);
        retired.then(|| {
            AgentError::new(
                "catalog-required",
                format!("The vault for {profile} changed and has not been read again. Refresh and try again."),
                true,
            )
        })
    }

    /// The error for a selected store the catalog does not list: the
    /// retired or unrefreshed vault of the key's profile when that explains
    /// the absence, and otherwise `store-not-found` with `message`.
    fn absent_store_error(
        &self,
        catalog: &CatalogSnapshot,
        key: Option<&StoreKey>,
        message: &str,
    ) -> AgentError {
        key.and_then(|key| {
            self.retired_store_error(key)
                .or_else(|| unrefreshed_store_error(catalog, key))
        })
        .unwrap_or_else(|| AgentError::new("store-not-found", message, false))
    }

    pub(in crate::commands) fn selected_team(
        &self,
        store: &str,
    ) -> Result<(String, String), AgentError> {
        let catalog = self.catalog.lock_unpoisoned();
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault to view group details.",
                true,
            ));
        };
        let key = StoreKey::parse(store);
        let selected = selected_team_summary(catalog, key.as_ref())
            .map(|(team, _, _)| (team.profile, team.team_alias))
            .ok_or_else(|| {
                self.absent_store_error(
                    catalog,
                    key.as_ref(),
                    "This group is currently inaccessible. There may have been a server issue or you may have been removed.",
                )
            })?;
        require_profile_available(catalog, &selected.0)?;
        Ok(selected)
    }

    pub(in crate::commands) fn selected_account(
        &self,
        id: &str,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        self.selected_account_inner(id, true)
    }

    pub(in crate::commands) fn local_account(
        &self,
        id: &str,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        self.selected_account_inner(id, false).or_else(|error| {
            self.local_accounts
                .lock_unpoisoned()
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
        let catalog = self.catalog.lock_unpoisoned();
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault to access this account.",
                true,
            ));
        };
        let key = StoreKey::parse(id);
        let account = key.as_ref().and_then(StoreKey::account).and_then(|key| {
            catalog.stores.iter().find_map(|candidate| match candidate {
                CatalogStoreSummary::Account { store } if store == key => Some(store.clone()),
                _ => None,
            })
        });
        let Some(account) = account else {
            return Err(self.absent_store_error(
                catalog,
                key.as_ref(),
                "This account is no longer in the vault.",
            ));
        };
        if require_available {
            require_profile_available(catalog, &account.profile)?;
        }
        Ok(account)
    }

    pub(in crate::commands) fn selected_account_dto(
        &self,
        id: &str,
    ) -> Result<AccountDto, AgentError> {
        let account = self.selected_account(id)?;
        let accounts = self.accounts.lock_unpoisoned();
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

    pub(in crate::commands) fn selected_device_target(
        &self,
        account_store_id: &str,
        device_id: &str,
    ) -> Result<foks_agent_proto::AccountStoreRef, AgentError> {
        let account = self.selected_account(account_store_id)?;
        if !valid_device_member_id_hex(device_id) {
            return Err(invalid_request("Select a valid device ID."));
        }
        let devices = self.devices.lock_unpoisoned();
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

    pub(in crate::commands) fn ensure_profile_available(
        &self,
        profile: &str,
    ) -> Result<(), AgentError> {
        let catalog = self.catalog.lock_unpoisoned();
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

    pub(in crate::commands) fn selected_active_team_for_mutation(
        &self,
        id: &str,
    ) -> Result<foks_agent_proto::TeamStoreRef, AgentError> {
        let catalog = self.catalog.lock_unpoisoned();
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before modifying this group.",
                true,
            ));
        };
        let key = StoreKey::parse(id);
        let Some((team, kind, active)) = selected_team_summary(catalog, key.as_ref()) else {
            return Err(self.absent_store_error(
                catalog,
                key.as_ref(),
                "This group is currently inaccessible. There may have been a server issue or you may have been removed.",
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

    pub(in crate::commands) fn selected_remote_named_team(
        &self,
        local_profile: &str,
        id: &str,
    ) -> Result<foks_agent_proto::TeamStoreRef, AgentError> {
        let catalog = self.catalog.lock_unpoisoned();
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "Refresh the vault before federating this group.",
                true,
            ));
        };
        let key = StoreKey::parse(id);
        let Some((remote, kind, active)) = selected_team_summary(catalog, key.as_ref()) else {
            return Err(self.absent_store_error(
                catalog,
                key.as_ref(),
                "The group to admit is no longer in the vault.",
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

    pub(in crate::commands) fn selected_member_target(
        &self,
        selected_store_id: &str,
        username: &str,
    ) -> Result<(foks_agent_proto::TeamStoreRef, String, MemberRole), AgentError> {
        let team = self.selected_active_team_for_mutation(selected_store_id)?;
        let username = required_field(username, "Username is required.")?;
        let rosters = self.rosters.lock_unpoisoned();
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
        let accounts = self.accounts.lock_unpoisoned();
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

    pub(in crate::commands) fn selected_inactive_federated_membership(
        &self,
        store_id: &str,
        operation_id: &str,
    ) -> Result<(foks_agent_proto::TeamStoreRef, FederationEntryDto), AgentError> {
        let team = self.selected_active_team_for_mutation(store_id)?;
        if !valid_operation_id_hex(operation_id) {
            return Err(invalid_request(
                "Choose a valid pending federated membership to retry.",
            ));
        }
        let federations = self.federations.lock_unpoisoned();
        let Some(entries) = federations.get(store_id) else {
            return Err(AgentError::new(
                "federation-required",
                "Refresh this group's federation list before retrying a federated membership.",
                true,
            ));
        };
        let mut matches = entries.iter().filter(|entry| {
            !entry.active && entry.operation_id_hex.as_deref() == Some(operation_id)
        });
        let Some(entry) = matches.next() else {
            return Err(federated_membership_not_resumable());
        };
        if matches.next().is_some() {
            return Err(federated_membership_not_resumable());
        }
        let entry = entry.clone();
        drop(federations);
        let catalog = self.catalog.lock_unpoisoned();
        if entry.remote_profile == team.profile
            || !catalog
                .as_ref()
                .is_some_and(|catalog| catalog.profiles.contains(&entry.remote_profile))
        {
            return Err(federated_membership_not_resumable());
        }
        require_profile_available(
            catalog
                .as_ref()
                .ok_or_else(catalog_changed_during_group_read)?,
            &entry.remote_profile,
        )?;
        Ok((team, entry))
    }

    pub(in crate::commands) fn selected_active_federation_target(
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
        let federations = self.federations.lock_unpoisoned();
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
        let rosters = self.rosters.lock_unpoisoned();
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

    pub(in crate::commands) fn selected_store(
        &self,
        id: &str,
    ) -> Result<(CatalogStoreRef, Option<bool>), AgentError> {
        self.store_for_key(StoreKey::parse(id).as_ref())
    }

    /// [`Self::selected_store`] for an identity the caller has already
    /// parsed; `None` is an identity that did not parse.
    fn store_for_key(
        &self,
        key: Option<&StoreKey>,
    ) -> Result<(CatalogStoreRef, Option<bool>), AgentError> {
        let catalog = self.catalog.lock_unpoisoned();
        let Some(catalog) = catalog.as_ref() else {
            return Err(AgentError::new(
                "catalog-required",
                "The vault catalog is not loaded. Refresh to continue.",
                true,
            ));
        };
        let selected = key
            .and_then(|key| {
                catalog
                    .stores
                    .iter()
                    .find_map(|candidate| match candidate {
                        CatalogStoreSummary::Account { store } if key.account() == Some(store) => {
                            Some(None)
                        }
                        CatalogStoreSummary::Team { store, active, .. }
                            if key.team() == Some(store) =>
                        {
                            Some(Some(*active))
                        }
                        _ => None,
                    })
                    .map(|active| (key.store().clone(), active))
            })
            .ok_or_else(|| {
                self.absent_store_error(catalog, key, "This vault is no longer available.")
            })?;
        let profile = match &selected.0 {
            CatalogStoreRef::Account(store) => &store.profile,
            CatalogStoreRef::Team(store) => &store.profile,
        };
        require_profile_available(catalog, profile)?;
        Ok(selected)
    }

    pub(in crate::commands) fn selected_create_store(
        &self,
        id: &str,
    ) -> Result<CatalogStoreRef, AgentError> {
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

    pub(in crate::commands) fn selected_bound_mutation_item(
        &self,
        store: &str,
        path: &str,
        version: u64,
        dirent_id: &str,
    ) -> Result<CatalogItem, AgentError> {
        let item = self.selected_mutation_item(store, path, version)?;
        require_listed_dirent(&item, dirent_id, "modifying")?;
        Ok(item)
    }

    pub(in crate::commands) fn selected_mutation_item(
        &self,
        store: &str,
        path: &str,
        version: u64,
    ) -> Result<CatalogItem, AgentError> {
        let key = StoreKey::parse(store);
        let item = self.selected_current_item(key.as_ref(), path, version, "modifying")?;
        match self.store_for_key(key.as_ref())?.1 {
            Some(false) => Err(AgentError::new(
                "inactive-group",
                "This group is inactive. Complete its setup before modifying it.",
                false,
            )),
            Some(true) | None => Ok(item),
        }
    }

    /// The listed version of the listed dirent at `path`, for a command that
    /// reads the item without changing it, so an inactive group is not
    /// refused. `action` completes the refresh message of a `conflict`.
    pub(in crate::commands) fn selected_bound_item(
        &self,
        store: &str,
        path: &str,
        version: u64,
        dirent_id: &str,
        action: &str,
    ) -> Result<CatalogItem, AgentError> {
        let key = StoreKey::parse(store);
        let item = self.selected_current_item(key.as_ref(), path, version, action)?;
        require_listed_dirent(&item, dirent_id, action)?;
        Ok(item)
    }

    /// [`Self::selected_item`], with a changed or missing item reported as a
    /// `conflict` that asks for a refresh before `action` it.
    fn selected_current_item(
        &self,
        key: Option<&StoreKey>,
        path: &str,
        version: u64,
        action: &str,
    ) -> Result<CatalogItem, AgentError> {
        self.item_for_key(key, path, version)
            .map_err(|error| match error.code.as_str() {
                "item-version-mismatch" | "item-not-found" => AgentError::new(
                    "conflict",
                    format!(
                        "This item changed or was removed. Refresh the vault before {action} it."
                    ),
                    false,
                ),
                _ => error,
            })
    }
}

/// Refuses `item` with a `conflict` unless its dirent is `dirent_id`, the one
/// the renderer selected, so that an item recreated at the same path and
/// version is not used in its place.
fn require_listed_dirent(
    item: &CatalogItem,
    dirent_id: &str,
    action: &str,
) -> Result<(), AgentError> {
    let listed: String = item
        .metadata
        .dirent_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if dirent_id == listed {
        return Ok(());
    }
    Err(AgentError::new(
        "conflict",
        format!("This item was replaced. Refresh the vault before {action} it."),
        false,
    ))
}

/// The answer for a store that is absent from `catalog.stores` when its
/// profile's last read recorded its stores as incomplete, rather than
/// because the vault no longer holds it. The renderer keeps showing a profile's previous stores while
/// its read is incomplete, so a request for one of them has to say that the
/// vault has not been refreshed, and be retryable, instead of claiming the
/// store is gone. The store still authorizes nothing: only a complete read
/// of the profile does that.
fn unrefreshed_store_error(catalog: &CatalogSnapshot, key: &StoreKey) -> Option<AgentError> {
    let profile = key.profile();
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
    let complete = if key.account().is_some() {
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

/// The team store `key` names, with its kind and whether it is active, when
/// the catalog lists it.
fn selected_team_summary<'a>(
    catalog: &'a CatalogSnapshot,
    key: Option<&StoreKey>,
) -> Option<(foks_agent_proto::TeamStoreRef, &'a str, bool)> {
    let key = key?.team()?;
    catalog.stores.iter().find_map(|candidate| match candidate {
        CatalogStoreSummary::Team {
            store,
            kind,
            active,
            ..
        } if store == key => Some((store.clone(), kind.as_str(), *active)),
        _ => None,
    })
}
