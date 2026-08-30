use foks_rpc::{encode_bare_success_response_at, encode_bare_void_success_response_at, RpcStatus};

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
    fn load_chain(
        &self,
        argument: &[u8],
        principal: Option<&Principal>,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn load_remote_view_tokens(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn load_team_membership_chain(
        &self,
        argument: &[u8],
        principal: Option<&Principal>,
    ) -> Result<Vec<u8>, RpcStatus>;
    fn grant_remote_view(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus>;
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
    fn post_team_membership_link(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<(), RpcStatus>;
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

    fn load_chain(
        &self,
        argument: &[u8],
        principal: Option<&Principal>,
    ) -> Result<Vec<u8>, RpcStatus> {
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

    fn load_remote_view_tokens(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::team_loader::load_remote_view_tokens(
            argument,
            principal,
            &self.host()?,
            &snapshot,
            self.clock.as_ref(),
        )
    }

    fn load_team_membership_chain(
        &self,
        argument: &[u8],
        principal: Option<&Principal>,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::team_loader::load_team_membership_chain(
            argument,
            principal,
            &self.host()?,
            &snapshot,
            self.clock.as_ref(),
        )
    }
    fn grant_remote_view(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        crate::services::federation::grant_remote_team_view(
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

    fn post_team_membership_link(
        &self,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::team_admin::post_team_membership_link(
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
}

pub(super) fn response(
    operations: &impl Operations,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    let sequence = call.call.sequence();
    let data = match call.route.id {
        RouteId::TeamLoaderGetTeamVOBearerTokenChallenge => Some(operations.loader_challenge(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamLoaderActivateTeamVOBearerToken => Some(operations.activate_loader(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamLoaderLoadTeamChain => {
            Some(operations.load_chain(call.call.argument(), principal)?)
        }
        RouteId::TeamLoaderLoadTeamMembershipChain => {
            Some(operations.load_team_membership_chain(call.call.argument(), principal)?)
        }
        RouteId::TeamLoaderLoadTeamRemoteViewTokens => Some(operations.load_remote_view_tokens(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamMemberGrantRemoteViewPermissionForTeam => Some(operations.grant_remote_view(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamAdminReserveTeamname => Some(operations.reserve_name(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamAdminCreateTeam | RouteId::TeamAdminCreateTeamAdHoc => {
            operations.create(
                call.call.argument(),
                call.route.id == RouteId::TeamAdminCreateTeam,
                principal.ok_or_else(permission_denied)?,
            )?;
            None
        }
        RouteId::TeamAdminEditTeam => Some(operations.edit(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamAdminMakeInertTeamBearerToken => Some(operations.make_inert_token(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamAdminActivateTeamBearerToken => {
            operations.activate_token(
                call.call.argument(),
                principal.ok_or_else(permission_denied)?,
            )?;
            None
        }
        RouteId::TeamAdminLoadRemovalKeyBoxForTeamAdmin => Some(operations.load_removal_box(
            call.call.argument(),
            principal.ok_or_else(permission_denied)?,
        )?),
        RouteId::TeamAdminPostTeamMembershipLink => {
            operations.post_team_membership_link(
                call.call.argument(),
                principal.ok_or_else(permission_denied)?,
            )?;
            None
        }
        _ => return Err(RpcStatus::Unsupported),
    };
    // Team protocols place their result bare on the wire with no DataWrap
    // envelope (go-foks proto/rem/team.go), matching how the client and
    // `foks_rpc::decode_call` treat every headerless protocol.
    match data {
        Some(data) => {
            encode_bare_success_response_at(&data, sequence).map_err(|_| RpcStatus::Unsupported)
        }
        None => encode_bare_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported),
    }
}
