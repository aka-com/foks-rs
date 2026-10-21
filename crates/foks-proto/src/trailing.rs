//! Forward-tolerant decoding for positional structs that upstream grows by
//! appending fields.
//!
//! go-foks adds fields to unsigned RPC structs only at the end and documents
//! that an older peer omits them, which go-codec decodes as the zero value.
//! go-codec also ignores array elements beyond the fields a struct declares.
//! The Rust decoders follow one rule, recorded with the per-type audit in
//! `book/23-compatibility.qmd`:
//!
//! - **Canonical objects decode exactly.** A struct that is signed, hashed,
//!   MAC'd, used as a key-derivation or nonce input, sealed as a box
//!   plaintext, or digested by a server keeps its exact field count. So does
//!   any struct nested inside one, and any Rust-only extension that carries
//!   its own version number and is specified to fail closed.
//! - **Unsigned RPC request and response structs accept a trailing tail.**
//!   They require their known fields and accept any number after them.
//! - **The tail is retained wherever the receiver re-encodes the value**:
//!   when it is stored, re-served to another party, forwarded, or compared
//!   as bytes. Retention uses the decoded bytes (types that keep an `exact`
//!   copy) or [`TrailingFields`]. A value that is only read, its fields
//!   copied into local state and never encoded again, may discard the tail;
//!   [`unsigned_rpc_fields`] marks those decoders.
//!
//! [`decode_grown`] accepts the retained shape: a fixed number of required
//! fields, then known optional fields that default when absent, then any
//! number of fields this implementation does not know. [`TrailingFields`]
//! retains how many optional fields were on the wire and the unknown
//! remainder, so a value that is decoded and re-encoded reproduces the
//! original bytes, and a value constructed locally with every optional field
//! at its default encodes in the older, shorter form.
//!
//! Encoders are unaffected: a value constructed by this implementation
//! encodes exactly the fields the pinned release defines, so bytes sent to a
//! v0.1.9 peer do not change.
//!
//! The hand-written server argument decoders in `foks-rpc` are still exact;
//! each is to be converted with the same stored-or-compared analysis.

use crate::{array_at_least, Result, Value};

/// Wire layout beyond the required fields of a grown positional struct.
///
/// Equality compares only the unknown fields: whether a known optional field
/// was written explicitly at its default changes the encoding, not the value.
/// Two equal values may therefore encode to different bytes; code that
/// decides whether to store or re-serve a value must compare encodings.
#[derive(Clone, Debug, Default)]
pub struct TrailingFields {
    optional_on_wire: usize,
    unknown: Vec<Value>,
}

impl PartialEq for TrailingFields {
    fn eq(&self, other: &Self) -> bool {
        self.unknown == other.unknown
    }
}
impl Eq for TrailingFields {}

impl TrailingFields {
    /// Fields after the known ones, in wire order.
    pub fn unknown(&self) -> &[Value] {
        &self.unknown
    }

    /// Drop fields whose meaning this implementation cannot judge, for
    /// example before projecting a stored value to a less privileged reader.
    pub fn clear_unknown(&mut self) {
        self.unknown.clear();
    }

    /// Number of optional fields to encode, given which of them hold a
    /// non-default value. The result never shortens what was decoded, always
    /// reaches the last non-default field, and covers every optional field
    /// when unknown fields follow them.
    pub fn optional_fields_to_encode(&self, non_default: &[bool]) -> usize {
        if !self.unknown.is_empty() {
            return non_default.len();
        }
        let needed = non_default
            .iter()
            .rposition(|set| *set)
            .map_or(0, |index| index + 1);
        needed.max(self.optional_on_wire.min(non_default.len()))
    }
}

/// The known fields of an unsigned RPC struct that is only read.
///
/// Requires at least `required` fields and returns exactly that many; any
/// fields after them are accepted and discarded. Use this only where the
/// decoded value is never re-encoded (see the module documentation); a value
/// that is stored, re-served, forwarded or compared retains the tail instead.
pub(crate) fn unsigned_rpc_fields(value: &Value, required: usize) -> Result<&[Value]> {
    Ok(&array_at_least(value, required)?[..required])
}

/// Fields of a decoded grown struct.
pub(crate) struct GrownFields<'a> {
    pub required: &'a [Value],
    /// One entry per known optional field; `None` when the peer omitted it.
    pub optional: Vec<Option<&'a Value>>,
    pub trailing: TrailingFields,
}

/// Split a positional struct into `required` fields, up to `optional` known
/// trailing fields, and an unknown remainder.
pub(crate) fn decode_grown(
    value: &Value,
    required: usize,
    optional: usize,
) -> Result<GrownFields<'_>> {
    let values = array_at_least(value, required)?;
    let (required_values, rest) = values.split_at(required);
    let optional_on_wire = rest.len().min(optional);
    let (present, unknown) = rest.split_at(optional_on_wire);
    Ok(GrownFields {
        required: required_values,
        optional: (0..optional).map(|index| present.get(index)).collect(),
        trailing: TrailingFields {
            optional_on_wire,
            unknown: unknown.to_vec(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    fn fields(n: u64) -> Value {
        Value::Array((0..n).map(Value::Unsigned).collect())
    }

    #[test]
    fn missing_optional_fields_decode_as_absent() {
        let value = fields(2);
        let grown = decode_grown(&value, 2, 1).unwrap();
        assert_eq!(grown.required.len(), 2);
        assert_eq!(grown.optional, vec![None]);
        assert!(grown.trailing.unknown().is_empty());
        assert_eq!(grown.trailing.optional_fields_to_encode(&[false]), 0);
        assert_eq!(grown.trailing.optional_fields_to_encode(&[true]), 1);
    }

    #[test]
    fn explicit_defaults_and_unknown_fields_are_retained() {
        let value = fields(3);
        let grown = decode_grown(&value, 2, 1).unwrap();
        assert_eq!(grown.optional, vec![Some(&Value::Unsigned(2))]);
        assert_eq!(grown.trailing.optional_fields_to_encode(&[false]), 1);
        let value = fields(5);
        let grown = decode_grown(&value, 2, 1).unwrap();
        assert_eq!(
            grown.trailing.unknown(),
            &[Value::Unsigned(3), Value::Unsigned(4)]
        );
        assert_eq!(grown.trailing.optional_fields_to_encode(&[false]), 1);
        let mut cleared = grown.trailing.clone();
        cleared.clear_unknown();
        assert_ne!(cleared, grown.trailing);
        assert_eq!(cleared.optional_fields_to_encode(&[false]), 1);
    }

    #[test]
    fn read_only_fields_accept_and_discard_a_tail() {
        assert_eq!(unsigned_rpc_fields(&fields(2), 2).unwrap().len(), 2);
        assert_eq!(
            unsigned_rpc_fields(&fields(4), 2).unwrap(),
            &[Value::Unsigned(0), Value::Unsigned(1)]
        );
        assert!(matches!(
            unsigned_rpc_fields(&fields(1), 2),
            Err(Error::FieldCount {
                expected: 2,
                found: 1
            })
        ));
        assert!(unsigned_rpc_fields(&Value::Bool(true), 1).is_err());
    }

    #[test]
    fn too_few_fields_and_non_arrays_are_rejected() {
        assert!(matches!(
            decode_grown(&fields(1), 2, 1),
            Err(Error::FieldCount {
                expected: 2,
                found: 1
            })
        ));
        assert!(decode_grown(&Value::Null, 2, 1).is_err());
    }
}
