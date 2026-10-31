pub(crate) mod federation;
pub(crate) mod generic;
pub(crate) mod kv;
pub(crate) mod log_upload;
pub(crate) mod merkle_query;
pub(crate) mod passphrase;
pub(crate) mod realtime;
pub(crate) mod registration;
pub(crate) mod signup;
pub(crate) mod team_admin;
pub(crate) mod team_invitations;
pub(crate) mod team_loader;
pub(crate) mod user;
pub(crate) mod user_mutation;
pub(crate) mod waitlist;

use std::sync::Arc;

const PROTOCOL_TIME_WINDOW_MILLISECONDS: u64 = 4 * 60 * 60 * 1_000;

pub(crate) fn protocol_time_is_nowish(time: u64, now_microseconds: u64) -> bool {
    time != 0 && time.abs_diff(now_microseconds / 1_000) <= PROTOCOL_TIME_WINDOW_MILLISECONDS
}

/// The server state that signup, username reservation, client certificate
/// issuance and user-chain mutations read. Each service checks the optional
/// ports at the point where it needs them, so a request that fails validation
/// reports that failure before a missing writer or key provider.
pub(crate) struct IdentityPorts<'a> {
    pub(crate) host_id: &'a [u8],
    pub(crate) canonical_name: &'a str,
    pub(crate) read_database: Option<&'a crate::read_pool::ReadPool>,
    pub(crate) metrics: &'a crate::ServerMetrics,
    pub(crate) writer: Option<&'a crate::WriterHandle>,
    pub(crate) key_provider: Option<&'a Arc<dyn crate::keys::HostKeyProvider>>,
    pub(crate) sso: Option<&'a Arc<crate::sso::SsoService>>,
    pub(crate) clock: &'a Arc<dyn foks_server_db::Clock>,
    pub(crate) entropy: &'a Arc<dyn crate::Entropy>,
    pub(crate) hostchain_tail: &'a foks_proto::HostchainTail,
}

impl IdentityPorts<'_> {
    fn read_database(&self) -> Result<crate::read_pool::ReadLease, foks_rpc::RpcStatus> {
        let pool = self.read_database.ok_or(foks_rpc::RpcStatus::Unsupported)?;
        pool.lease(self.metrics)
    }

    fn host(&self) -> Result<foks_proto::EntityId, foks_rpc::RpcStatus> {
        foks_proto::EntityId::from_bytes(self.host_id.to_vec()).map_err(user::bad_arguments)
    }
}

#[cfg(test)]
mod tests {
    use super::protocol_time_is_nowish;

    #[test]
    fn protocol_times_are_unix_milliseconds_against_internal_microseconds() {
        let now_millis = 1_700_000_000_000_u64;
        let now_micros = now_millis * 1_000;
        assert!(protocol_time_is_nowish(now_millis, now_micros));
        assert!(protocol_time_is_nowish(
            now_millis - 4 * 60 * 60 * 1_000,
            now_micros
        ));
        assert!(!protocol_time_is_nowish(
            now_millis - 4 * 60 * 60 * 1_000 - 1,
            now_micros
        ));
        assert!(!protocol_time_is_nowish(now_micros, now_micros));
        assert!(!protocol_time_is_nowish(0, now_micros));
    }

    #[test]
    fn every_write_mapper_reports_shared_failure_classes() {
        use crate::status::contract::{assert_shared_classes, wrapped_sso};
        type Mapper = fn(crate::Error) -> foks_rpc::RpcStatus;
        let mappers: [(&str, Mapper); 23] = [
            ("federation", super::federation::map_write_error),
            ("generic", super::generic::map_write_error),
            ("kv write", super::kv::map_write_error),
            ("kv upload", super::kv::map_upload_error),
            ("kv lock", super::kv::map_lock_error),
            ("log upload", super::log_upload::map_write_error),
            (
                "log upload identifier",
                super::log_upload::map_identifier_write_error,
            ),
            ("realtime", super::realtime::write_error),
            ("registration", super::registration::map_write_error),
            ("signup", super::signup::map_signup_error),
            ("username reservation", super::signup::map_reservation_error),
            ("certificate", super::signup::map_certificate_error),
            (
                "registration passphrase",
                super::registration::map_passphrase_write_error,
            ),
            ("team edit", super::team_admin::map_edit_error),
            ("team create", super::team_admin::map_create_error),
            ("team admin", super::team_admin::map_write_error),
            ("team invitations", super::team_invitations::write_error),
            ("team loader", super::team_loader::map_write_error),
            (
                "user mutation",
                super::user_mutation::map_user_mutation_error,
            ),
            ("yubi", super::user::map_yubi_write_error),
            ("device nag", super::user::map_device_nag_write_error),
            ("waitlist", super::waitlist::map_write_error),
            (
                "waitlist identifier",
                super::waitlist::map_identifier_write_error,
            ),
        ];
        for (route, map) in mappers {
            assert_shared_classes(route, wrapped_sso, map);
        }
    }
}
