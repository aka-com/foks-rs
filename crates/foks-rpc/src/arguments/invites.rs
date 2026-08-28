use foks_proto::InviteCode;
use foks_snowpack::{decode, Value};

use crate::{Error, Result};

pub fn decode_check_invite_code(argument: &[u8]) -> Result<InviteCode> {
    let Value::Array(fields) = decode(argument)? else {
        return Err(Error::Envelope {
            expected: "one-field invite-code argument",
            found: "another Snowpack value",
        });
    };
    let [code] = fields.as_slice() else {
        return Err(Error::Envelope {
            expected: "one-field invite-code argument",
            found: "another field count",
        });
    };
    let code = InviteCode::from_value(code)?;
    code.validate()?;
    Ok(code)
}
