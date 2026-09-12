//! Strict application-status decoding for named Go RPC and positional Snowpack.
use super::{
    decode, map_length, named_path_version_vector, positional_path_version_vector, text, unsigned,
    Cursor, Error, Result, StatusDetail, Value, ValueKind, STATUS_METHOD_NOT_FOUND_ERROR,
    STATUS_NOT_IMPLEMENTED,
};

pub(super) fn check_status(bytes: &[u8]) -> Result<()> {
    if bytes == [0xc0] {
        return Ok(());
    }
    if matches!(bytes.first(), Some(0x92)) {
        return check_positional_status(bytes);
    }
    // RPC status values are encoded by the Go RPC codec as named MessagePack
    // structs, not as canonical positional Snowpack values. For example a
    // stale-cache status is `{Sc: 8012, f11: {Root, Path}}`.
    let mut cursor = Cursor::new(bytes);
    let fields = map_length(&mut cursor)?;
    if !(1..=2).contains(&fields) {
        return Err(Error::Envelope {
            expected: "one- or two-field FOKS status",
            found: "unexpected map length",
        });
    }
    let mut code = None;
    let mut payload = None;
    for _ in 0..fields {
        let key = text(cursor.value()?)?;
        let value = cursor.value()?;
        if key == b"Sc" {
            if code.replace(unsigned(value)?).is_some() {
                return Err(Error::Envelope {
                    expected: "one FOKS status code",
                    found: "duplicate status code",
                });
            }
        } else if payload.replace((key, value)).is_some() {
            return Err(Error::Envelope {
                expected: "one FOKS status payload",
                found: "duplicate status payload",
            });
        }
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of FOKS status",
            found: "trailing data",
        });
    }
    let code = code.ok_or(Error::Envelope {
        expected: "FOKS status code",
        found: "status without Sc",
    })?;
    if code == 0 && payload.is_none() {
        return Ok(());
    }
    if code == 0 {
        return Err(Error::Envelope {
            expected: "payload-free successful FOKS status",
            found: "successful status with a payload",
        });
    }
    if code == STATUS_NOT_IMPLEMENTED && payload.is_some() {
        return Err(Error::Envelope {
            expected: "payload-free unsupported status",
            found: "unexpected payload",
        });
    }
    if code == STATUS_METHOD_NOT_FOUND_ERROR {
        let Some((_, bytes)) = payload.filter(|(tag, _)| tag == b"f3") else {
            return Err(Error::Envelope {
                expected: "method-not-found payload f3",
                found: "missing or incorrect payload",
            });
        };
        return Err(named_missing_method(bytes)?);
    }
    if code == 8012 {
        let Some((tag, value)) = payload else {
            return Err(Error::Envelope {
                expected: "KV stale-cache status payload",
                found: "missing status payload",
            });
        };
        if tag != b"f11" {
            return Err(Error::Envelope {
                expected: "KV stale-cache status field f11",
                found: "another status variant",
            });
        }
        return Err(Error::KvStaleCache(named_path_version_vector(value)?));
    }
    let detail = match payload {
        Some((_, value)) => match decode(value) {
            Ok(Value::Text(bytes)) => String::from_utf8(bytes).ok(),
            Ok(other) => Some(format!("{other:?}")),
            Err(_) => None,
        },
        None => None,
    };
    Err(Error::RemoteStatus {
        code,
        detail: StatusDetail(detail),
    })
}

fn check_positional_status(bytes: &[u8]) -> Result<()> {
    let value = decode(bytes)?;
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "two-field FOKS Status",
            found: value.kind(),
        });
    };
    if fields.len() != 2 {
        return Err(Error::Envelope {
            expected: "two-field FOKS Status",
            found: "another array length",
        });
    }
    let Value::Unsigned(code) = fields[0] else {
        return Err(Error::Envelope {
            expected: "unsigned FOKS status code",
            found: fields[0].kind(),
        });
    };
    if code == 0 && fields[1] == Value::Variant(None) {
        return Ok(());
    }
    if code == STATUS_NOT_IMPLEMENTED && fields[1] != Value::Variant(None) {
        return Err(Error::Envelope {
            expected: "payload-free unsupported status",
            found: "unexpected payload",
        });
    }
    if code == STATUS_METHOD_NOT_FOUND_ERROR {
        let Value::Variant(Some((tag, value))) = &fields[1] else {
            return Err(Error::Envelope {
                expected: "method-not-found variant",
                found: "missing payload",
            });
        };
        let Value::Array(method) = value.as_ref() else {
            return Err(Error::Envelope {
                expected: "method-not-found fields",
                found: "incorrect payload",
            });
        };
        if tag != b"3" || method.len() != 3 {
            return Err(Error::Envelope {
                expected: "method-not-found variant 3 with three fields",
                found: "incorrect payload",
            });
        }
        let (Value::Unsigned(protocol_id), Value::Unsigned(position), Value::Text(name)) =
            (&method[0], &method[1], &method[2])
        else {
            return Err(Error::Envelope {
                expected: "method-not-found identity",
                found: "incorrect fields",
            });
        };
        std::str::from_utf8(name).map_err(|_| Error::Envelope {
            expected: "UTF-8 protocol name",
            found: "invalid UTF-8",
        })?;
        return Err(Error::MethodNotFound {
            protocol_id: *protocol_id,
            position: *position,
        });
    }
    if code == 8012 {
        let Value::Variant(Some((tag, value))) = &fields[1] else {
            return Err(Error::Envelope {
                expected: "KV stale-cache status payload",
                found: "missing status payload",
            });
        };
        if tag.as_slice() != b"b" {
            return Err(Error::Envelope {
                expected: "KV stale-cache status variant b",
                found: "another status variant",
            });
        }
        return Err(Error::KvStaleCache(positional_path_version_vector(value)?));
    }
    let detail = match &fields[1] {
        Value::Variant(Some((_, value))) => match value.as_ref() {
            Value::Text(bytes) => String::from_utf8(bytes.clone()).ok(),
            other => Some(format!("{other:?}")),
        },
        _ => None,
    };
    Err(Error::RemoteStatus {
        code,
        detail: StatusDetail(detail),
    })
}

/// Decodes go-foks' positional `PathVersionVector` (`[Root, Path]`) carried in a
/// stale-cache status, applying the same directory/dirent bounds as the cached
/// projection so a hostile server cannot force an unbounded allocation.
fn named_missing_method(bytes: &[u8]) -> Result<Error> {
    let mut cursor = Cursor::new(bytes);
    if map_length(&mut cursor)? != 3 {
        return Err(Error::Envelope {
            expected: "three-field MethodV2",
            found: "incorrect map length",
        });
    }
    let (mut protocol_id, mut position, mut name) = (None, None, None);
    for _ in 0..3 {
        match text(cursor.value()?)?.as_slice() {
            b"Proto" if protocol_id.is_none() => protocol_id = Some(unsigned(cursor.value()?)?),
            b"Method" if position.is_none() => position = Some(unsigned(cursor.value()?)?),
            b"Name" if name.is_none() => {
                let bytes = text(cursor.value()?)?;
                std::str::from_utf8(&bytes).map_err(|_| Error::Envelope {
                    expected: "UTF-8 protocol name",
                    found: "invalid UTF-8",
                })?;
                name = Some(());
            }
            _ => {
                return Err(Error::Envelope {
                    expected: "unique MethodV2 field",
                    found: "unknown or duplicate field",
                })
            }
        }
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of MethodV2",
            found: "trailing data",
        });
    }
    match (protocol_id, position, name) {
        (Some(protocol_id), Some(position), Some(())) => Ok(Error::MethodNotFound {
            protocol_id,
            position,
        }),
        _ => Err(Error::Envelope {
            expected: "complete MethodV2",
            found: "missing fields",
        }),
    }
}
