use super::{permission_denied, ServerData};
use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::auth::Principal;
use crate::rpc::RoutedCall;

impl ServerData {
    pub(super) fn response(
        &self,
        call: RoutedCall,
        principal: Option<&Principal>,
    ) -> std::result::Result<Vec<u8>, RpcStatus> {
        let sequence = call.call.sequence();
        match (call.route.protocol, call.route.method) {
            ("Probe", "probe") => {
                self.validate_probe(call.call.argument())?;
                encode_success_response_at(&self.current_probe_response()?, sequence)
                    .map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg" | "MerkleQuery" | "KvStore", "selectVHost") => {
                self.validate_host_argument(call.call.argument())?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("MerkleQuery", "getCurrentRoot") => {
                self.validate_host_argument(call.call.argument())?;
                let root = self.current_root()?;
                encode_success_response_at(&root, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("MerkleQuery", "getHistoricalRoots") => {
                let response = self.historical_roots(call.call.argument())?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "reserveUsername") => {
                let response = self.reserve_username(call.call.argument())?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "signup") => {
                self.signup(call.call.argument())?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "getClientCertChain") => {
                let response = self.client_certificate_chain(call.call.argument())?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "getUIDLookupChallege") => {
                let response = crate::services::registration::issue_uid_lookup_challenge(
                    call.call.argument(),
                    &self.host()?,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg", "lookupUIDByDevice") => {
                let response = crate::services::registration::lookup_uid_by_device(
                    call.call.argument(),
                    &self.host()?,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "loadUserChain") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::user::load_user_chain(
                    &database,
                    &self.host()?,
                    call.call.argument(),
                    principal,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "getPukForRole") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::user::puk_for_role(
                    &database,
                    call.call.argument(),
                    principal,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "provisionDevice") => {
                let principal = principal.ok_or_else(permission_denied)?;
                self.user_mutation(call.call.argument(), principal, true)?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "revokeDevice") => {
                let principal = principal.ok_or_else(permission_denied)?;
                principal.require_ordinary_device()?;
                self.user_mutation(call.call.argument(), principal, false)?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("User", "getHostConfig") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::user::host_config(&database, principal)?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamLoader", "getTeamVOBearerTokenChallenge") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_loader::issue_challenge(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamLoader", "activateTeamVOBearerToken") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_loader::activate(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_deref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamLoader", "loadTeamChain") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_loader::load_chain(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "reserveTeamname") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let response = crate::services::team_admin::reserve_name(
                    call.call.argument(),
                    principal,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "createTeam" | "createTeamAdHoc") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                crate::services::team_admin::create(
                    call.call.argument(),
                    call.route.method == "createTeam",
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
                    &self.clock,
                    &self.hostchain_tail,
                )?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "editTeam") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_admin::edit(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.key_provider.as_ref().ok_or(RpcStatus::Unsupported)?,
                    &self.clock,
                    &self.hostchain_tail,
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "makeInertTeamBearerToken") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_admin::make_inert_token(
                    call.call.argument(),
                    principal,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                    self.entropy.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "activateTeamBearerToken") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                crate::services::team_admin::activate_token(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
                    self.clock.as_ref(),
                )?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("TeamAdmin", "loadRemovalKeyBoxForTeamAdmin") => {
                let principal = principal.ok_or_else(permission_denied)?;
                let database = self.read_database()?;
                let response = crate::services::team_admin::load_removal_box(
                    call.call.argument(),
                    principal,
                    &self.host()?,
                    &database,
                    self.clock.as_ref(),
                )?;
                encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("KvStore", method) => {
                let principal = principal.ok_or_else(permission_denied)?;
                principal.require_ordinary_device()?;
                let writer = self.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
                let database = self.read_database()?;
                match crate::services::kv::dispatch(
                    method,
                    call.call.argument(),
                    principal,
                    &database,
                    writer,
                    &self.clock,
                )? {
                    crate::services::kv::Response::Data(response) => {
                        encode_success_response_at(&response, sequence)
                            .map_err(|_| RpcStatus::Unsupported)
                    }
                    crate::services::kv::Response::Void => {
                        encode_void_success_response_at(sequence)
                            .map_err(|_| RpcStatus::Unsupported)
                    }
                }
            }
            _ => Err(RpcStatus::Unsupported),
        }
    }
}
