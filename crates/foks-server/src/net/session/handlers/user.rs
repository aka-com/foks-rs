use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::auth::Principal;
use crate::rpc::{RouteId, RoutedCall};

use super::super::{permission_denied, ServerData};

pub(super) trait Operations {
    fn load_user_chain(&self, argument: &[u8], principal: &Principal)
        -> Result<Vec<u8>, RpcStatus>;
    fn puk_for_role(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn provision_device(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn revoke_device(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn host_config(&self, principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
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
    fn load_user_chain(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::load_user_chain(&snapshot, &self.host()?, argument, principal)
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

    fn set_passphrase(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::user::set_passphrase(
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            argument,
            principal,
        )
    }

    fn change_passphrase(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::user::change_passphrase(
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            argument,
            principal,
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
