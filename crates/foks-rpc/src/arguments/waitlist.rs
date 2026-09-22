use foks_snowpack::{decode, Value};

use crate::{Error, Result};

pub fn decode_join_waitlist(bytes: &[u8]) -> Result<Vec<u8>> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("join-waitlist argument wrapper"));
    };
    let [Value::Text(email)] = fields.as_slice() else {
        return Err(shape("join-waitlist argument"));
    };
    Ok(email.clone())
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}

#[cfg(test)]
mod tests {
    use foks_snowpack::encode;

    use super::*;

    #[test]
    fn decodes_the_exact_go_positional_argument() {
        let argument = encode(&Value::Array(vec![Value::Text(b"a@example.com".to_vec())])).unwrap();
        assert_eq!(decode_join_waitlist(&argument).unwrap(), b"a@example.com");
    }

    #[test]
    fn rejects_appended_fields() {
        let appended = encode(&Value::Array(vec![
            Value::Text(b"a@example.com".to_vec()),
            Value::Null,
        ]))
        .unwrap();
        assert!(decode_join_waitlist(&appended).is_err());
    }
}
