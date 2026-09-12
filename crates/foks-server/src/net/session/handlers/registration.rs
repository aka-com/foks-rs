use std::sync::Arc;

use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::rpc::{RouteId, RoutedCall};

use super::super::ServerData;

pub(super) trait Operations {
    fn host_id(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn vhost_management_host(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;

    fn init_oauth2(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn sso_login(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn resolve_username(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn client_version_info(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn server_config(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn check_name_exists(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn join_waitlist(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn probe_key_exists(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn reserve_username(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn signup(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn check_invite_code(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn client_certificate_chain(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn uid_lookup_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn lookup_uid_by_device(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn subkey_box_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn load_subkey_box(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn login_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn passphrase_login(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn stretch_version(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn load_remote_user_chain(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
}

impl Operations for ServerData {
    fn host_id(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        foks_rpc::arguments::decode_void(argument).map_err(super::super::bad_arguments)?;
        foks_snowpack::encode(&foks_snowpack::Value::Binary(self.host()?.into_bytes()))
            .map_err(|_| RpcStatus::TransactionRetry)
    }
    fn vhost_management_host(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        foks_rpc::arguments::decode_void(argument).map_err(super::super::bad_arguments)?;
        foks_snowpack::encode(&foks_snowpack::Value::Text(
            self.vhost_management_host.as_bytes().to_vec(),
        ))
        .map_err(|_| RpcStatus::TransactionRetry)
    }

    fn init_oauth2(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let arg = foks_proto::InitOAuth2SessionArgument::decode(argument)
            .map_err(|_| RpcStatus::BadArguments("invalid OIDC request".into()))?;
        let peer = self.peer_ip.ok_or(RpcStatus::Unsupported)?.to_string();
        let url = self
            .sso
            .as_ref()
            .ok_or(RpcStatus::Unsupported)?
            .init(&arg, peer.as_bytes())
            .map_err(crate::sso::status)?;
        foks_snowpack::encode(&foks_snowpack::Value::Text(url.into_bytes()))
            .map_err(|_| RpcStatus::TransactionRetry)
    }
    fn sso_login(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        let arg = foks_proto::SsoLoginArgument::decode(argument)
            .map_err(|_| RpcStatus::BadArguments("invalid SSO login request".into()))?;
        self.sso
            .as_ref()
            .ok_or(RpcStatus::Unsupported)?
            .login(&arg)
            .map_err(crate::sso::status)
    }

    fn resolve_username(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::user::resolve_username(&snapshot, argument, None)
    }

    fn client_version_info(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::client_version_info(argument)
    }

    fn server_config(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        crate::services::registration::server_config(
            argument,
            &database,
            self.sso.as_ref().map(|s| s.public_config()),
        )
    }

    fn check_name_exists(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::registration::check_name_exists(argument, &database)
    }

    fn join_waitlist(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::peripheral::join_waitlist(
            argument,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            Arc::clone(&self.clock),
            self.entropy.as_ref(),
        )
    }

    fn probe_key_exists(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        let database = self.read_database()?;
        crate::services::registration::probe_key_exists(argument, &database)
    }

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

    fn subkey_box_challenge(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::issue_subkey_challenge(
            argument,
            &self.host()?,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
            self.entropy.as_ref(),
        )
    }

    fn load_subkey_box(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        crate::services::registration::load_subkey_box(
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

    fn load_remote_user_chain(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        let database = self.read_database()?;
        let snapshot = database
            .snapshot()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        let now = self
            .clock
            .now_micros()
            .map_err(|_| RpcStatus::TransactionRetry)?;
        crate::services::federation::load_remote_user_chain(&snapshot, &self.host()?, argument, now)
    }
}

pub(super) fn response(
    operations: &impl Operations,
    call: RoutedCall,
) -> Result<Vec<u8>, RpcStatus> {
    let sequence = call.call.sequence();
    match call.route.id {
        RouteId::RegGetHostID => {
            encode_success_response_at(&operations.host_id(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::TransactionRetry)
        }
        RouteId::RegGetVHostMgmtHost => encode_success_response_at(
            &operations.vhost_management_host(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::TransactionRetry),
        RouteId::RegInitOAuth2Session => {
            encode_success_response_at(&operations.init_oauth2(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::TransactionRetry)
        }
        RouteId::RegSsoLogin => {
            operations.sso_login(call.call.argument())?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::TransactionRetry)
        }

        RouteId::RegResolveUsername => encode_success_response_at(
            &operations.resolve_username(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::RegCheckNameExists => {
            operations.check_name_exists(call.call.argument())?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegJoinWaitList => {
            encode_success_response_at(&operations.join_waitlist(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegGetClientVersionInfo => encode_success_response_at(
            &operations.client_version_info(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::RegGetServerConfig => {
            encode_success_response_at(&operations.server_config(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegProbeKeyExists => {
            operations.probe_key_exists(call.call.argument())?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
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
        RouteId::RegGetSubkeyBoxChallenge => encode_success_response_at(
            &operations.subkey_box_challenge(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::RegLoadSubkeyBox => {
            encode_success_response_at(&operations.load_subkey_box(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegLoadUserChain => encode_success_response_at(
            &operations.load_remote_user_chain(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        _ => Err(RpcStatus::Unsupported),
    }
}
