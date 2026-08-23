//! Private, schema-oriented Snowpack decoding primitives.

use crate::{EntityId, Error, Result, Value, ENTITY_DEVICE, ENTITY_YUBI};

pub(crate) fn array(value: &Value, expected: usize) -> Result<&[Value]> {
    let Value::Array(values) = value else {
        return Err(type_error("array", value));
    };
    if values.len() != expected {
        return Err(Error::FieldCount {
            expected,
            found: values.len(),
        });
    }
    Ok(values)
}

pub(crate) fn list<T>(value: &Value, parser: fn(&Value) -> Result<T>) -> Result<Vec<T>> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(values) => values.iter().map(parser).collect(),
        _ => Err(type_error("list or null", value)),
    }
}

pub(crate) fn option<T>(
    value: &Value,
    parser: impl FnOnce(&Value) -> Result<T>,
) -> Result<Option<T>> {
    if value == &Value::Null {
        Ok(None)
    } else {
        parser(value).map(Some)
    }
}

pub(crate) fn variant<'a>(value: &'a Value, expected: &str) -> Result<&'a Value> {
    let Value::Variant(Some((tag, payload))) = value else {
        return Err(type_error("one-case variant", value));
    };
    if tag != expected.as_bytes() {
        return Err(Error::VariantTag {
            expected: expected.to_owned(),
            found: tag.clone(),
        });
    }
    Ok(payload)
}

pub(crate) fn unsigned(value: &Value) -> Result<u64> {
    match value {
        Value::Unsigned(value) => Ok(*value),
        _ => Err(type_error("unsigned integer", value)),
    }
}

pub(crate) fn boolean(value: &Value) -> Result<bool> {
    match value {
        Value::Bool(value) => Ok(*value),
        _ => Err(type_error("boolean", value)),
    }
}

pub(crate) fn integer(value: &Value) -> Result<i64> {
    match value {
        Value::Unsigned(value) => i64::try_from(*value).map_err(|_| Error::IntegerRange("i64")),
        Value::Negative(value) => Ok(*value),
        _ => Err(type_error("integer", value)),
    }
}

pub(crate) fn expect_unsigned(value: &Value, kind: &'static str, expected: u64) -> Result<()> {
    let found = unsigned(value)?;
    if found != expected {
        return Err(Error::UnknownEnum { kind, value: found });
    }
    Ok(())
}

pub(crate) fn binary(value: &Value) -> Result<&[u8]> {
    match value {
        Value::Binary(bytes) => Ok(bytes),
        _ => Err(type_error("binary", value)),
    }
}

pub(crate) fn fixed_blob<const N: usize>(value: &Value, kind: &'static str) -> Result<[u8; N]> {
    let bytes = binary(value)?;
    bytes.try_into().map_err(|_| Error::Length {
        kind,
        expected: N,
        found: bytes.len(),
    })
}

pub(crate) fn entity(value: &Value) -> Result<EntityId> {
    EntityId::from_bytes(binary(value)?.to_vec())
}

pub(crate) fn device_entity(value: &Value) -> Result<EntityId> {
    let entity = entity(value)?;
    match entity.entity_type() {
        ENTITY_DEVICE | ENTITY_YUBI => Ok(entity),
        found => Err(Error::WrongEntityType {
            expected: ENTITY_DEVICE,
            found,
        }),
    }
}

pub(crate) fn text(value: &Value) -> Result<String> {
    let bytes = text_bytes(value)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::Utf8)
}

pub(crate) fn text_bytes(value: &Value) -> Result<&[u8]> {
    let Value::Text(bytes) = value else {
        return Err(type_error("text", value));
    };
    Ok(bytes)
}

pub(crate) fn type_error(expected: &'static str, value: &Value) -> Error {
    Error::Type {
        expected,
        found: value_name(value),
    }
}

pub(crate) fn value_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Unsigned(_) => "unsigned integer",
        Value::Negative(_) => "negative integer",
        Value::Binary(_) => "binary",
        Value::Text(_) => "text",
        Value::Array(_) => "array",
        Value::Variant(_) => "variant",
    }
}
