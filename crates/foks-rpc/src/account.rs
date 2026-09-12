//! Go v0.1.9 account calls, including hosted helpers not served by our server.
use crate::{
    encode_call, encode_call_with_validated_argument, Result, REG_PROTOCOL_ID, USER_PROTOCOL_ID,
};
use foks_proto::ChangeUsernameArgument;
use foks_snowpack::{encode, Value};

pub fn encode_change_username_request_at(
    arg: &ChangeUsernameArgument,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(USER_PROTOCOL_ID, 11, &arg.encoded()?, sequence)
}
pub fn encode_reserve_username_for_change_request_at(
    name: &[u8],
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        12,
        &encode(&Value::Array(vec![Value::Text(name.to_vec())]))?,
        sequence,
    )
}
pub fn encode_get_tree_location_request_at(seqno: u64, sequence: u64) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        13,
        &encode(&Value::Array(vec![Value::Unsigned(seqno)]))?,
        sequence,
    )
}
pub fn encode_get_host_id_request_at(sequence: u64) -> Result<Vec<u8>> {
    encode_call_with_validated_argument(REG_PROTOCOL_ID, 3, &[0x90], sequence)
}
pub fn encode_get_vhost_mgmt_host_request_at(sequence: u64) -> Result<Vec<u8>> {
    encode_call_with_validated_argument(REG_PROTOCOL_ID, 22, &[0x90], sequence)
}
pub fn encode_new_web_admin_panel_url_request_at(sequence: u64) -> Result<Vec<u8>> {
    encode_call_with_validated_argument(USER_PROTOCOL_ID, 21, &[0x90], sequence)
}
pub fn encode_check_url_request_at(url: &str, sequence: u64) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        22,
        &encode(&Value::Array(vec![Value::Text(url.as_bytes().to_vec())]))?,
        sequence,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_frames_match_go_including_niladic_arrays() {
        let calls = [
            ("host-id", encode_get_host_id_request_at(7).unwrap()),
            (
                "vhost-mgmt",
                encode_get_vhost_mgmt_host_request_at(7).unwrap(),
            ),
            (
                "web-admin",
                encode_new_web_admin_panel_url_request_at(7).unwrap(),
            ),
            (
                "check-url",
                encode_check_url_request_at("https://admin.example/a?tok=x", 7).unwrap(),
            ),
            (
                "reserve",
                encode_reserve_username_for_change_request_at(b"alice_new", 7).unwrap(),
            ),
            (
                "location",
                encode_get_tree_location_request_at(2, 7).unwrap(),
            ),
        ];
        for (name, frame) in calls {
            assert_eq!(
                frame,
                std::fs::read(format!(
                    "../foks-snowpack/tests/fixtures/foks-v0.1.9/account/{name}.frame"
                ))
                .unwrap(),
                "{name}"
            );
        }
    }
    #[test]
    fn rename_wire_matches_go_and_rejects_partial_full() {
        for kind in ["full", "display"] {
            let prefix =
                format!("../foks-snowpack/tests/fixtures/foks-v0.1.9/account/rename-{kind}");
            let bytes = std::fs::read(format!("{prefix}.snowp")).unwrap();
            let arg = ChangeUsernameArgument::decode(&bytes).unwrap();
            assert_eq!(arg.encoded().unwrap(), bytes);
            assert_eq!(
                encode_change_username_request_at(&arg, 7).unwrap(),
                std::fs::read(format!("{prefix}.frame")).unwrap()
            );
        }
        for full in [Value::Unsigned(0), Value::Array(vec![Value::Null; 4])] {
            assert!(ChangeUsernameArgument::decode(
                &encode(&Value::Array(vec![Value::Text(b"alice".to_vec()), full])).unwrap()
            )
            .is_err());
        }
    }
}
