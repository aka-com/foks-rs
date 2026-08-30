use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::auth::Principal;
use crate::rpc::{RouteId, RoutedCall};

use super::super::{permission_denied, ServerData};

pub(super) trait Operations {
    fn resolve_username(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn ping(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn device_nag(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn clear_device_nag(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn load_user_chain(&self, argument: &[u8], principal: &Principal)
        -> Result<Vec<u8>, RpcStatus>;
    fn puk_for_role(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn provision_device(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn revoke_device(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn host_config(&self, principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn post_generic_link(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn load_generic_chain(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn team_list(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn set_passphrase(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn change_passphrase(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn passphrase_salt(&self, argument: &[u8], principal: &Principal)
        -> Result<Vec<u8>, RpcStatus>;
    fn next_passphrase_generation(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn stretch_version(&self, argument: &[u8], principal: &Principal)
        -> Result<Vec<u8>, RpcStatus>;
    fn ppe_parcel(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn put_yubi_management_key(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<(), RpcStatus>;
    fn get_yubi_management_key(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn get_all_yubi_management_keys(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn grant_remote_user_view(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
}

impl Operations for ServerData {
    fn resolve_username(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::resolve_username(&snapshot, argument, Some(principal))
    }

    fn ping(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::ping(&snapshot, argument, principal)
    }

    fn device_nag(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::device_nag(&snapshot, argument, principal)
    }

    fn clear_device_nag(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::user::clear_device_nag(
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            argument,
            principal,
        )
    }

    fn load_user_chain(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let now = self
            .clock
            .now_micros()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::load_user_chain(&snapshot, &self.host()?, argument, principal, now)
    }

    fn puk_for_role(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::puk_for_role(&snapshot, argument, principal)
    }

    fn provision_device(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        self.user_mutation(argument, principal, true)
    }

    fn revoke_device(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        principal.require_ordinary_device()?;
        self.user_mutation(argument, principal, false)
    }

    fn host_config(&self, principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::host_config(&snapshot, principal)
    }

    fn post_generic_link(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        crate::services::generic::post(
            argument,
            principal,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            &self.hostchain_tail,
        )
    }

    fn load_generic_chain(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::generic::load(&snapshot, argument, principal)
    }

    fn team_list(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::generic::team_list(&snapshot, &self.host()?, argument, principal)
    }

    fn set_passphrase(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        let decoded = foks_rpc::arguments::decode_set_passphrase(argument)
            .map_err(|_| RpcStatus::BadArguments("invalid passphrase boxes".to_owned()))?;
        let link = decoded.user_settings_link.clone().ok_or_else(|| {
            RpcStatus::BadArguments("passphrase update omitted UserSettings link".to_owned())
        })?;
        let owned = super::super::OwnedPassphraseMutation::from_argument(&decoded)
            .map_err(|_| RpcStatus::BadArguments("invalid passphrase boxes".to_owned()))?;
        crate::services::generic::commit(
            link,
            principal,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            &self.hostchain_tail,
            Some(crate::services::generic::PassphraseCompanion::Set(owned)),
        )
    }

    fn change_passphrase(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        let current = database
            .passphrase(principal.uid())
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or(RpcStatus::PassphraseNotFound)?;
        let decoded = foks_rpc::arguments::decode_change_passphrase(argument, current.salt)
            .map_err(|_| RpcStatus::BadArguments("invalid passphrase boxes".to_owned()))?;
        let link = decoded.user_settings_link.clone().ok_or_else(|| {
            RpcStatus::BadArguments("passphrase update omitted UserSettings link".to_owned())
        })?;
        let owned = super::super::OwnedPassphraseMutation::from_argument(&decoded)
            .map_err(|_| RpcStatus::BadArguments("invalid passphrase boxes".to_owned()))?;
        crate::services::generic::commit(
            link,
            principal,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            &self.hostchain_tail,
            Some(crate::services::generic::PassphraseCompanion::Change(owned)),
        )
    }

    fn passphrase_salt(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::passphrase_salt(&snapshot, argument, principal)
    }

    fn next_passphrase_generation(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::next_passphrase_generation(&snapshot, argument, principal)
    }

    fn stretch_version(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::stretch_version(&snapshot, argument, principal)
    }

    fn ppe_parcel(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::ppe_parcel(&snapshot, argument, principal)
    }

    fn put_yubi_management_key(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<(), RpcStatus> {
        crate::services::user::put_yubi_management_key(
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            argument,
            principal,
        )
    }

    fn get_yubi_management_key(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::get_yubi_management_key(&snapshot, argument, principal)
    }

    fn get_all_yubi_management_keys(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::get_all_yubi_management_keys(&snapshot, argument, principal)
    }

    fn grant_remote_user_view(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        crate::services::federation::grant_remote_user_view(
            argument,
            principal,
            &self.host()?,
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            self.entropy.as_ref(),
        )
    }
}

pub(super) fn response(
    operations: &impl Operations,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    let principal = principal.ok_or_else(permission_denied)?;
    let sequence = call.call.sequence();
    match call.route.id {
        RouteId::UserResolveUsername => encode_success_response_at(
            &operations.resolve_username(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserPing => {
            encode_success_response_at(&operations.ping(call.call.argument(), principal)?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserGetDeviceNag => encode_success_response_at(
            &operations.device_nag(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserClearDeviceNag => {
            operations.clear_device_nag(call.call.argument(), principal)?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserSetPassphrase => {
            operations.set_passphrase(call.call.argument(), principal)?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserChangePassphrase => {
            operations.change_passphrase(call.call.argument(), principal)?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserGetSalt => encode_success_response_at(
            &operations.passphrase_salt(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserNextPassphraseGeneration => encode_success_response_at(
            &operations.next_passphrase_generation(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserStretchVersion => encode_success_response_at(
            &operations.stretch_version(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserGetPpeParcel => encode_success_response_at(
            &operations.ppe_parcel(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserLoadUserChain => encode_success_response_at(
            &operations.load_user_chain(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserGetPukForRole => encode_success_response_at(
            &operations.puk_for_role(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserProvisionDevice => {
            operations.provision_device(call.call.argument(), principal)?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserRevokeDevice => {
            operations.revoke_device(call.call.argument(), principal)?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserGetHostConfig => {
            encode_success_response_at(&operations.host_config(principal)?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserPostGenericLink => {
            operations.post_generic_link(call.call.argument(), principal)?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserLoadGenericChain => encode_success_response_at(
            &operations.load_generic_chain(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserGetTeamListServerTrust => encode_success_response_at(
            &operations.team_list(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserPutYubiManagementKey => {
            operations.put_yubi_management_key(call.call.argument(), principal)?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::UserGetYubiManagementKey => encode_success_response_at(
            &operations.get_yubi_management_key(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserGetAllYubiManagementKeys => encode_success_response_at(
            &operations.get_all_yubi_management_keys(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::UserGrantRemoteViewPermissionForUser => encode_success_response_at(
            &operations.grant_remote_user_view(call.call.argument(), principal)?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        _ => Err(RpcStatus::Unsupported),
    }
}
