use foks_proto::{EntityId, PostGenericLinkArgument};
use foks_snowpack::{decode, Value};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadGenericChainArgument {
    pub entity: EntityId,
    pub chain_type: u64,
    pub start: u64,
}

pub fn decode_post_generic_link(argument: &[u8]) -> Result<PostGenericLinkArgument> {
    PostGenericLinkArgument::decode(argument).map_err(Into::into)
}

pub fn decode_load_generic_chain(argument: &[u8]) -> Result<LoadGenericChainArgument> {
    let Value::Array(fields) = decode(argument)? else {
        return Err(shape("generic chain argument"));
    };
    let [Value::Binary(entity), Value::Unsigned(chain_type), Value::Unsigned(start)] =
        fields.as_slice()
    else {
        return Err(shape("generic chain argument"));
    };
    if *start == 0
        || !matches!(
            *chain_type,
            foks_proto::CHAIN_TYPE_USER_SETTINGS | foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
        )
    {
        return Err(shape("generic chain range"));
    }
    Ok(LoadGenericChainArgument {
        entity: EntityId::from_bytes(entity.clone())?,
        chain_type: *chain_type,
        start: *start,
    })
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another value",
    }
}
