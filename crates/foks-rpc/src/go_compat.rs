//! Go named and positional version-vector compatibility decoding.

use crate::codec::{array_length, map_length, text, unsigned, Cursor, ValueKind};
use crate::{Error, Result};
use foks_proto::KvPathVersionVector;
use foks_snowpack::{decode, Value};

pub(super) fn positional_path_version_vector(value: &Value) -> Result<KvPathVersionVector> {
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "positional PathVersionVector",
            found: value.kind(),
        });
    };
    if fields.len() != 2 {
        return Err(Error::Envelope {
            expected: "two-field PathVersionVector",
            found: "another array length",
        });
    }
    let Value::Unsigned(root_version) = fields[0] else {
        return Err(Error::Envelope {
            expected: "unsigned PathVersionVector Root",
            found: fields[0].kind(),
        });
    };
    let directories = match &fields[1] {
        Value::Null => Vec::new(),
        Value::Array(entries) => {
            if entries.len() > foks_proto::MAXIMUM_KV_DIRECTORIES {
                return Err(Error::CollectionTooLarge {
                    kind: "cached directory",
                    received: entries.len(),
                    maximum: foks_proto::MAXIMUM_KV_DIRECTORIES,
                });
            }
            let mut total_dirents = 0usize;
            let mut output = Vec::with_capacity(entries.len());
            for entry in entries {
                output.push(positional_directory_version(entry, &mut total_dirents)?);
            }
            output
        }
        other => {
            return Err(Error::Envelope {
                expected: "PathVersionVector Path array or null",
                found: other.kind(),
            })
        }
    };
    Ok(KvPathVersionVector {
        root_version,
        directories,
    })
}

pub(super) fn positional_directory_version(
    value: &Value,
    total_dirents: &mut usize,
) -> Result<foks_proto::KvDirectoryVersion> {
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "positional DirVersion",
            found: value.kind(),
        });
    };
    if fields.len() != 3 {
        return Err(Error::Envelope {
            expected: "three-field DirVersion",
            found: "another array length",
        });
    }
    let id = positional_kv_id(&fields[0])?;
    let Value::Unsigned(version) = fields[1] else {
        return Err(Error::Envelope {
            expected: "unsigned DirVersion Vers",
            found: fields[1].kind(),
        });
    };
    let entries = match &fields[2] {
        Value::Null => Vec::new(),
        Value::Array(items) => {
            *total_dirents = total_dirents.saturating_add(items.len());
            if *total_dirents > foks_proto::MAXIMUM_KV_DIRENTS {
                return Err(Error::CollectionTooLarge {
                    kind: "cached dirent",
                    received: *total_dirents,
                    maximum: foks_proto::MAXIMUM_KV_DIRENTS,
                });
            }
            items
                .iter()
                .map(positional_dirent_version)
                .collect::<Result<Vec<_>>>()?
        }
        other => {
            return Err(Error::Envelope {
                expected: "DirVersion De array or null",
                found: other.kind(),
            })
        }
    };
    Ok(foks_proto::KvDirectoryVersion {
        id,
        version,
        entries,
    })
}

pub(super) fn positional_dirent_version(value: &Value) -> Result<foks_proto::KvDirentVersion> {
    let Value::Array(fields) = value else {
        return Err(Error::Envelope {
            expected: "positional DirentVersion",
            found: value.kind(),
        });
    };
    if fields.len() != 2 {
        return Err(Error::Envelope {
            expected: "two-field DirentVersion",
            found: "another array length",
        });
    }
    let id = positional_kv_id(&fields[0])?;
    let Value::Unsigned(version) = fields[1] else {
        return Err(Error::Envelope {
            expected: "unsigned DirentVersion Vers",
            found: fields[1].kind(),
        });
    };
    Ok(foks_proto::KvDirentVersion { id, version })
}

pub(super) fn positional_kv_id(value: &Value) -> Result<[u8; 16]> {
    let Value::Binary(bytes) = value else {
        return Err(Error::Envelope {
            expected: "16-byte KV id",
            found: value.kind(),
        });
    };
    <[u8; 16]>::try_from(bytes.as_slice()).map_err(|_| Error::Envelope {
        expected: "16-byte KV id",
        found: "another length",
    })
}

pub(super) fn named_path_version_vector(bytes: &[u8]) -> Result<KvPathVersionVector> {
    let mut cursor = Cursor::new(bytes);
    let fields = map_length(&mut cursor)?;
    if fields != 2 {
        return Err(Error::Envelope {
            expected: "two-field PathVersionVector",
            found: "another map length",
        });
    }
    let mut root_version = None;
    let mut directories = None;
    for _ in 0..fields {
        match text(cursor.value()?)?.as_slice() {
            b"Root" if root_version.is_none() => root_version = Some(unsigned(cursor.value()?)?),
            b"Path" if directories.is_none() => {
                directories = Some(named_directory_versions(cursor.value()?)?)
            }
            _ => {
                return Err(Error::Envelope {
                    expected: "PathVersionVector field",
                    found: "unknown field",
                });
            }
        }
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of PathVersionVector",
            found: "trailing data",
        });
    }
    Ok(KvPathVersionVector {
        root_version: root_version.ok_or(Error::Envelope {
            expected: "PathVersionVector Root",
            found: "missing field",
        })?,
        directories: directories.ok_or(Error::Envelope {
            expected: "PathVersionVector Path",
            found: "missing field",
        })?,
    })
}

pub(super) fn named_directory_versions(
    bytes: &[u8],
) -> Result<Vec<foks_proto::KvDirectoryVersion>> {
    let mut cursor = Cursor::new(bytes);
    let length = array_length(&mut cursor)?;
    if length > foks_proto::MAXIMUM_KV_DIRECTORIES {
        return Err(Error::CollectionTooLarge {
            kind: "cached directory",
            received: length,
            maximum: foks_proto::MAXIMUM_KV_DIRECTORIES,
        });
    }
    if length > cursor.remaining() {
        return Err(Error::Truncated);
    }
    let mut total_dirents = 0usize;
    let mut output = Vec::with_capacity(length);
    for _ in 0..length {
        let value = cursor.value()?;
        let mut fields = Cursor::new(value);
        let count = map_length(&mut fields)?;
        if count != 3 {
            return Err(Error::Envelope {
                expected: "three-field DirVersion",
                found: "another map length",
            });
        }
        let mut id = None;
        let mut version = None;
        let mut entries = None;
        for _ in 0..count {
            match text(fields.value()?)?.as_slice() {
                b"Id" if id.is_none() => id = Some(fixed_16(fields.value()?)?),
                b"Vers" if version.is_none() => version = Some(unsigned(fields.value()?)?),
                b"De" if entries.is_none() => {
                    entries = Some(named_dirent_versions_counted(
                        fields.value()?,
                        &mut total_dirents,
                    )?)
                }
                _ => {
                    return Err(Error::Envelope {
                        expected: "DirVersion field",
                        found: "unknown field",
                    });
                }
            }
        }
        if !fields.done() {
            return Err(Error::Envelope {
                expected: "end of DirVersion",
                found: "trailing data",
            });
        }
        output.push(foks_proto::KvDirectoryVersion {
            id: id.ok_or(Error::Envelope {
                expected: "DirVersion Id",
                found: "missing field",
            })?,
            version: version.ok_or(Error::Envelope {
                expected: "DirVersion Vers",
                found: "missing field",
            })?,
            entries: entries.ok_or(Error::Envelope {
                expected: "DirVersion De",
                found: "missing field",
            })?,
        });
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of DirVersion list",
            found: "trailing data",
        });
    }
    Ok(output)
}

#[cfg(test)]
pub(super) fn named_dirent_versions(bytes: &[u8]) -> Result<Vec<foks_proto::KvDirentVersion>> {
    let mut total = 0;
    named_dirent_versions_counted(bytes, &mut total)
}

pub(super) fn named_dirent_versions_counted(
    bytes: &[u8],
    total: &mut usize,
) -> Result<Vec<foks_proto::KvDirentVersion>> {
    let mut cursor = Cursor::new(bytes);
    let length = array_length(&mut cursor)?;
    let next_total = total.checked_add(length).ok_or(Error::CollectionTooLarge {
        kind: "cached dirent",
        received: usize::MAX,
        maximum: foks_proto::MAXIMUM_KV_DIRENTS,
    })?;
    if next_total > foks_proto::MAXIMUM_KV_DIRENTS {
        return Err(Error::CollectionTooLarge {
            kind: "cached dirent",
            received: next_total,
            maximum: foks_proto::MAXIMUM_KV_DIRENTS,
        });
    }
    if length > cursor.remaining() {
        return Err(Error::Truncated);
    }
    *total = next_total;
    let mut output = Vec::with_capacity(length);
    for _ in 0..length {
        let value = cursor.value()?;
        let mut fields = Cursor::new(value);
        let count = map_length(&mut fields)?;
        if count != 2 {
            return Err(Error::Envelope {
                expected: "two-field DirentVersion",
                found: "another map length",
            });
        }
        let mut id = None;
        let mut version = None;
        for _ in 0..count {
            match text(fields.value()?)?.as_slice() {
                b"Id" if id.is_none() => id = Some(fixed_16(fields.value()?)?),
                b"Vers" if version.is_none() => version = Some(unsigned(fields.value()?)?),
                _ => {
                    return Err(Error::Envelope {
                        expected: "DirentVersion field",
                        found: "unknown field",
                    });
                }
            }
        }
        if !fields.done() {
            return Err(Error::Envelope {
                expected: "end of DirentVersion",
                found: "trailing data",
            });
        }
        output.push(foks_proto::KvDirentVersion {
            id: id.ok_or(Error::Envelope {
                expected: "DirentVersion Id",
                found: "missing field",
            })?,
            version: version.ok_or(Error::Envelope {
                expected: "DirentVersion Vers",
                found: "missing field",
            })?,
        });
    }
    if !cursor.done() {
        return Err(Error::Envelope {
            expected: "end of DirentVersion list",
            found: "trailing data",
        });
    }
    Ok(output)
}

pub(super) fn fixed_16(bytes: &[u8]) -> Result<[u8; 16]> {
    match decode(bytes)? {
        Value::Binary(bytes) => bytes.try_into().map_err(|_| Error::Envelope {
            expected: "16-byte identifier",
            found: "another binary length",
        }),
        other => Err(Error::Envelope {
            expected: "binary identifier",
            found: other.kind(),
        }),
    }
}
