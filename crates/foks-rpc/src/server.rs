use std::io::Read;

use super::{read_frame, text, unsigned, Cursor, Error, Result, METHOD_CALL_V2, RESPONSE_HEADER};

/// A validated v0.1.9 RPC call whose protocol argument remains byte-exact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedCall {
    sequence: u64,
    protocol_id: u64,
    method_position: u64,
    argument: Vec<u8>,
}

impl DecodedCall {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn protocol_id(&self) -> u64 {
        self.protocol_id
    }

    pub fn method_position(&self) -> u64 {
        self.method_position
    }

    pub fn argument(&self) -> &[u8] {
        &self.argument
    }
}

/// Reads and strictly decodes one bounded, length-prefixed RPC call.
pub fn read_call<R: Read>(reader: &mut R, maximum: usize) -> Result<DecodedCall> {
    decode_call(&read_frame(reader, maximum)?)
}

/// Strictly decodes one unframed v0.1.9 RPC call.
pub fn decode_call(content: &[u8]) -> Result<DecodedCall> {
    let mut cursor = Cursor::new(content);
    if cursor.byte()? != 0x95 {
        return Err(Error::Envelope {
            expected: "five-element RPC call array",
            found: "another MessagePack value",
        });
    }
    if unsigned(cursor.value()?)? != METHOD_CALL_V2 {
        return Err(Error::Envelope {
            expected: "RPC call method",
            found: "another RPC method",
        });
    }
    let sequence = unsigned(cursor.value()?)?;
    let protocol_id = unsigned(cursor.value()?)?;
    let method_position = unsigned(cursor.value()?)?;
    let wrapped = cursor.value()?;
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of RPC call",
            found: "trailing data",
        });
    }

    let mut wrapped = Cursor::new(wrapped);
    if wrapped.byte()? != 0x82 {
        return Err(Error::Envelope {
            expected: "two-field RPC data wrapper",
            found: "another MessagePack value",
        });
    }
    if text(wrapped.value()?)? != b"Data" {
        return Err(Error::Envelope {
            expected: "canonical Data field",
            found: "another field",
        });
    }
    let argument = wrapped.value()?.to_vec();
    if text(wrapped.value()?)? != b"Header" {
        return Err(Error::Envelope {
            expected: "canonical Header field",
            found: "another field",
        });
    }
    if wrapped.value()? != RESPONSE_HEADER {
        return Err(Error::Compatibility);
    }
    if !wrapped.done() {
        return Err(Error::Envelope {
            expected: "end of RPC data wrapper",
            found: "trailing data",
        });
    }
    foks_snowpack::validate(&argument).map_err(|source| Error::ArgumentSnowpack {
        protocol_id,
        method_position,
        source,
    })?;

    Ok(DecodedCall {
        sequence,
        protocol_id,
        method_position,
        argument,
    })
}
