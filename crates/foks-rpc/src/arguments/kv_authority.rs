//! Native KV authority syntax, split around the caller's existing clock read.
//! Neither syntax stage authenticates a credential or authorizes a namespace.
use crate::RpcStatus;
use foks_snowpack::{decode, Value};

/// Owns the original complete decode until authority resolution finishes.
/// No decoded interior or raw token is exposed through Debug/Clone accessors.
pub struct KvAuthorityArgument {
    root: Value,
    wrapped: bool,
}

/// Parsed syntax only. The caller must resolve the selected authority from SQL.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KvAuthoritySelection {
    Personal,
    Team([u8; 16]),
}

impl KvAuthorityArgument {
    /// Perform precisely the shape checks that precede the native clock read.
    pub fn decode(argument: &[u8]) -> Result<Self, RpcStatus> {
        let root = decode(argument).map_err(bad_arguments)?;
        let Value::Array(fields) = &root else {
            return Err(bad_arguments("KV argument is not a struct"));
        };
        let first = fields
            .first()
            .ok_or_else(|| bad_arguments("KV argument has no authentication"))?;
        let Value::Array(first_fields) = first else {
            return Err(bad_arguments("KV authentication is not a struct"));
        };
        let wrapped = !matches!(first_fields.first(), Some(Value::Unsigned(0 | 1)));
        let auth = if wrapped {
            first_fields
                .first()
                .ok_or_else(|| bad_arguments("KV request header has no authentication"))?
        } else {
            first
        };
        if !matches!(auth, Value::Array(_)) {
            return Err(bad_arguments("KV authentication is not a struct"));
        }
        Ok(Self { root, wrapped })
    }

    /// Classify after the caller's clock read, preserving clock-error precedence.
    /// Returns fixed data; the complete decoded argument remains owned by self.
    pub fn classify(&self) -> Result<KvAuthoritySelection, RpcStatus> {
        let Value::Array(fields) = &self.root else {
            unreachable!("private decoded argument invariant")
        };
        let Value::Array(first_fields) = &fields[0] else {
            unreachable!("private authentication shape invariant")
        };
        let auth = if self.wrapped {
            let Value::Array(auth) = &first_fields[0] else {
                unreachable!("private wrapped authentication invariant")
            };
            auth
        } else {
            first_fields
        };
        match auth.as_slice() {
            [Value::Unsigned(0), Value::Variant(None)] => Ok(KvAuthoritySelection::Personal),
            [Value::Unsigned(1), Value::Variant(Some((tag, token)))] if tag == b"1" => {
                let Value::Binary(token) = token.as_ref() else {
                    return Err(bad_arguments("team KV token is not binary"));
                };
                let token = token
                    .as_slice()
                    .try_into()
                    .map_err(|_| bad_arguments("team KV token has the wrong width"))?;
                Ok(KvAuthoritySelection::Team(token))
            }
            _ => Err(bad_arguments("unsupported KV authentication")),
        }
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

#[cfg(test)]
mod tests;
