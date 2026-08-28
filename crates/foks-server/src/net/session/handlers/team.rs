use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::auth::Principal;
use crate::rpc::{RouteId, RoutedCall};

use super::super::{permission_denied, ServerData};

pub(super) trait Operations {
    fn loader_challenge(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn activate_loader(&self, argument: &[u8], principal: &Principal)
        -> Result<Vec<u8>, RpcStatus>;
    fn load_chain(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn reserve_name(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn create(&self, argument: &[u8], named: bool, principal: &Principal) -> Result<(), RpcStatus>;
    fn edit(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus>;
    fn make_inert_token(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn activate_token(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus>;
    fn load_removal_box(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
}

impl Operations for ServerData {
    fn loader_challenge(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        crate::services::team_loader::issue_challenge(
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

    fn activate_loader(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        crate::services::team_loader::activate(
            argument,
            principal,
            &self.host()?,
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
        )
    }

    fn load_chain(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::team_loader::load_chain(
            argument,
            principal,
            &self.host()?,
            &snapshot,
            self.clock.as_ref(),
        )
    }

    fn reserve_name(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        crate::services::team_admin::reserve_name(
            argument,
            principal,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            self.entropy.as_ref(),
        )
    }

    fn create(&self, argument: &[u8], named: bool, principal: &Principal) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::team_admin::create(
            argument,
            named,
            principal,
            &self.host()?,
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            &self.hostchain_tail,
        )
    }

    fn edit(&self, argument: &[u8], principal: &Principal) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        crate::services::team_admin::edit(
            argument,
            principal,
            &self.host()?,
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            &self.hostchain_tail,
        )
    }

    fn make_inert_token(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        crate::services::team_admin::make_inert_token(
            argument,
            principal,
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            self.entropy.as_ref(),
        )
    }

    fn activate_token(&self, argument: &[u8], principal: &Principal) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::team_admin::activate_token(
            argument,
            principal,
            &self.host()?,
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
        )
    }

    fn load_removal_box(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::team_admin::load_removal_box(
            argument,
            principal,
            &self.host()?,
            &snapshot,
            self.clock.as_ref(),
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
    let data = match call.route.id {
        RouteId::TeamLoaderGetTeamVOBearerTokenChallenge => {
            Some(operations.loader_challenge(call.call.argument(), principal)?)
        }
        RouteId::TeamLoaderActivateTeamVOBearerToken => {
            Some(operations.activate_loader(call.call.argument(), principal)?)
        }
        RouteId::TeamLoaderLoadTeamChain => {
            Some(operations.load_chain(call.call.argument(), principal)?)
        }
        RouteId::TeamAdminReserveTeamname => {
            Some(operations.reserve_name(call.call.argument(), principal)?)
        }
        RouteId::TeamAdminCreateTeam | RouteId::TeamAdminCreateTeamAdHoc => {
            operations.create(
                call.call.argument(),
                call.route.id == RouteId::TeamAdminCreateTeam,
                principal,
            )?;
            None
        }
        RouteId::TeamAdminEditTeam => Some(operations.edit(call.call.argument(), principal)?),
        RouteId::TeamAdminMakeInertTeamBearerToken => {
            Some(operations.make_inert_token(call.call.argument(), principal)?)
        }
        RouteId::TeamAdminActivateTeamBearerToken => {
            operations.activate_token(call.call.argument(), principal)?;
            None
        }
        RouteId::TeamAdminLoadRemovalKeyBoxForTeamAdmin => {
            Some(operations.load_removal_box(call.call.argument(), principal)?)
        }
        _ => return Err(RpcStatus::Unsupported),
    };
    match data {
        Some(data) => {
            encode_success_response_at(&data, sequence).map_err(|_| RpcStatus::Unsupported)
        }
        None => encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported),
    }
}
