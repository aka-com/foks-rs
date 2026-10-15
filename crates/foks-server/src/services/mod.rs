pub(crate) mod federation;
pub(crate) mod generic;
pub(crate) mod kv;
pub(crate) mod log_upload;
pub(crate) mod realtime;
pub(crate) mod registration;
pub(crate) mod team_admin;
pub(crate) mod team_invitations;
pub(crate) mod team_loader;
pub(crate) mod user;
pub(crate) mod waitlist;

const PROTOCOL_TIME_WINDOW_MILLISECONDS: u64 = 4 * 60 * 60 * 1_000;

pub(crate) fn protocol_time_is_nowish(time: u64, now_microseconds: u64) -> bool {
    time != 0 && time.abs_diff(now_microseconds / 1_000) <= PROTOCOL_TIME_WINDOW_MILLISECONDS
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
        let mappers: [(&str, Mapper); 19] = [
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
            (
                "registration passphrase",
                super::registration::map_passphrase_write_error,
            ),
            ("team edit", super::team_admin::map_edit_error),
            ("team create", super::team_admin::map_create_error),
            ("team admin", super::team_admin::map_write_error),
            ("team invitations", super::team_invitations::write_error),
            ("team loader", super::team_loader::map_write_error),
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
