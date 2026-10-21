//! Forward-tolerant decoding for positional structs that upstream grows by
//! appending fields.
//!
//! go-foks adds fields to unsigned RPC structs only at the end and documents
//! that an older peer omits them, which go-codec decodes as the zero value.
//! [`decode_grown`] accepts that shape: a fixed number of required fields,
//! then known optional fields that default when absent, then any number of
//! fields this implementation does not know. [`TrailingFields`] retains how
//! many optional fields were on the wire and the unknown remainder, so a value
//! that is decoded and re-encoded reproduces the original bytes, and a value
//! constructed locally with every optional field at its default encodes in the
//! older, shorter form.
//!
//! Only unsigned request and response structs may use this. Signed, hashed or
//! MAC'd canonical objects keep exact field-count decoding.

use crate::{array_at_least, Result, Value};

/// Wire layout beyond the required fields of a grown positional struct.
///
/// Equality compares only the unknown fields: whether a known optional field
/// was written explicitly at its default changes the encoding, not the value.
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
