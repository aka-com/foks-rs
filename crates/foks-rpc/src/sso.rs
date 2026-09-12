//! Client-only OIDC codecs; routes are advertised only when server enforcement lands.
use crate::{encode_call, Result, REG_PROTOCOL_ID};
use foks_proto::{InitOAuth2SessionArgument, PollOAuth2SessionArgument, SsoLoginArgument};
pub const REG_INIT_OAUTH2_SESSION_METHOD_POSITION: u64 = 18;
pub const REG_POLL_OAUTH2_SESSION_COMPLETION_METHOD_POSITION: u64 = 19;
pub const REG_SSO_LOGIN_METHOD_POSITION: u64 = 20;
pub fn encode_init_oauth2_session_request_at(
    arg: &InitOAuth2SessionArgument,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_INIT_OAUTH2_SESSION_METHOD_POSITION,
        &arg.encoded()?,
        sequence,
    )
}
pub fn encode_poll_oauth2_session_request_at(
    arg: &PollOAuth2SessionArgument,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_POLL_OAUTH2_SESSION_COMPLETION_METHOD_POSITION,
        &arg.encoded()?,
        sequence,
    )
}
pub fn encode_sso_login_request_at(arg: &SsoLoginArgument, sequence: u64) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_SSO_LOGIN_METHOD_POSITION,
        &arg.encoded()?,
        sequence,
    )
}

/// Add an exact SSO annex before the signup request enters its mutation journal.
pub fn attach_signup_sso(request: &[u8], sso: &foks_proto::RegSsoArgs) -> Result<Vec<u8>> {
    let mut reader = std::io::Cursor::new(request);
    let call = crate::read_call(&mut reader, crate::DEFAULT_MAX_FRAME_LENGTH)?;
    if reader.position() != request.len() as u64
        || call.protocol_id() != REG_PROTOCOL_ID
        || call.method_position() != crate::REG_SIGNUP_METHOD_POSITION
    {
        return Err(crate::Error::Envelope {
            expected: "one signup frame",
            found: "another request",
        });
    }
    foks_proto::DecodedSignupArgument::decode(call.argument())?;
    let foks_snowpack::Value::Array(mut fields) = foks_snowpack::decode(call.argument())? else {
        unreachable!("typed signup decoder checked array")
    };
    fields[15] = foks_snowpack::decode(&sso.encoded()?)?;
    encode_call(
        REG_PROTOCOL_ID,
        crate::REG_SIGNUP_METHOD_POSITION,
        &foks_snowpack::encode(&foks_snowpack::Value::Array(fields))?,
        call.sequence(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "../foks-snowpack/tests/fixtures/foks-v0.1.9/sso/{name}"
        ))
        .unwrap()
    }
    #[test]
    fn rpc_envelopes_match_go_positions_and_wrappers() {
        let init = InitOAuth2SessionArgument::decode(&fixture("init-login.snowp")).unwrap();
        assert_eq!(
            encode_init_oauth2_session_request_at(&init, 7).unwrap(),
            fixture("request-18.frame")
        );
        let poll = PollOAuth2SessionArgument::decode(&fixture("poll.snowp")).unwrap();
        assert_eq!(
            encode_poll_oauth2_session_request_at(&poll, 7).unwrap(),
            fixture("request-19.frame")
        );
        let login = SsoLoginArgument::decode(&fixture("login.snowp")).unwrap();
        assert_eq!(
            encode_sso_login_request_at(&login, 7).unwrap(),
            fixture("request-20.frame")
        );
    }
}
