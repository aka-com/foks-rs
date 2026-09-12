pub(crate) mod federation;
pub(crate) mod generic;
pub(crate) mod kv;
pub(crate) mod peripheral;
pub(crate) mod realtime;
pub(crate) mod registration;
pub(crate) mod team_admin;
pub(crate) mod team_invitations;
pub(crate) mod team_loader;
pub(crate) mod user;

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
}
