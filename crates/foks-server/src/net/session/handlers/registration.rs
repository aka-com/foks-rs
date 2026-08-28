use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::rpc::{RouteId, RoutedCall};

use super::super::ServerData;

pub(super) trait Operations {
    fn reserve_username(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn signup(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn check_invite_code(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn client_certificate_chain(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn uid_lookup_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn lookup_uid_by_device(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn login_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn passphrase_login(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn stretch_version(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
}

impl Operations for ServerData {
    fn reserve_username(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::reserve_username(self, argument)
    }

    fn signup(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        ServerData::signup(self, argument)
    }

    fn check_invite_code(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::registration::check_invite_code(argument, &database, self.clock.as_ref())
    }

    fn client_certificate_chain(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::client_certificate_chain(self, argument)
    }

    fn uid_lookup_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::issue_uid_lookup_challenge(
            argument,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            self.entropy.as_ref(),
        )
    }

    fn lookup_uid_by_device(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::lookup_uid_by_device(
            argument,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
        )
    }

    fn login_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::issue_login_challenge(
            argument,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            self.entropy.as_ref(),
        )
    }

    fn passphrase_login(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::passphrase_login(
            argument,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
        )
    }

    fn stretch_version(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::stretch_version(argument)
    }
}

pub(super) fn response(
    operations: &impl Operations,
    call: RoutedCall,
) -> Result<Vec<u8>, RpcStatus> {
    let sequence = call.call.sequence();
    match call.route.id {
        RouteId::RegReserveUsername => encode_success_response_at(
            &operations.reserve_username(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::RegSignup => {
            operations.signup(call.call.argument())?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegGetLoginChallenge => {
            encode_success_response_at(&operations.login_challenge(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegLogin => encode_success_response_at(
            &operations.passphrase_login(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::RegStretchVersion => {
            encode_success_response_at(&operations.stretch_version(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegCheckInviteCode => {
            operations.check_invite_code(call.call.argument())?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegGetClientCertChain => encode_success_response_at(
            &operations.client_certificate_chain(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::RegGetUIDLookupChallege => encode_success_response_at(
            &operations.uid_lookup_challenge(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::RegLookupUIDByDevice => encode_success_response_at(
            &operations.lookup_uid_by_device(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        _ => Err(RpcStatus::Unsupported),
    }
}
