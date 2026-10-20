//! Frozen test-only schema59 oracle (3ef60ab7). Keep this independent of the
//! streaming implementation; it intentionally allocates old-style token vectors.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub fn tokens(sql: &str) -> Result<Vec<Vec<u8>>, &'static str> {
    let bytes = sql.as_bytes();
    let mut at = 0;
    let mut tokens = Vec::new();
    while at < bytes.len() {
        let first = bytes[at];
        if first.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"--") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at += 2;
            while at < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            if at == bytes.len() {
                return Err("unterminated schema comment");
            }
            at += 2;
            continue;
        }
        if matches!(first, b'\'' | b'"' | b'`' | b'[') {
            let start = at;
            let end = if first == b'[' { b']' } else { first };
            at += 1;
            let mut text = Vec::new();
            loop {
                let byte = *bytes.get(at).ok_or("unterminated schema quote")?;
                at += 1;
                if byte == end {
                    if end != b']' && bytes.get(at) == Some(&end) {
                        text.push(end);
                        at += 1;
                        continue;
                    }
                    break;
                }
                text.push(byte);
            }
            // SQLite ALTER TABLE historically quotes precisely these renamed
            // table/column identifiers. Qualify those spellings explicitly;
            // do not treat arbitrary double-quoted tokens as identifiers since
            // SQLite can also interpret them as string literals (DQS).
            text.make_ascii_lowercase();
            let migrated_identifier = matches!(
                text.as_slice(),
                b"chunk_objects"
                    | b"chunk_object_attempts"
                    | b"kv_file_chunks"
                    | b"blocked_reason"
                    | b"source_hash"
            );
            if first != b'\'' && migrated_identifier {
                tokens.push(text);
            } else {
                tokens.push(bytes[start..at].to_vec());
            }
        } else if first.is_ascii_alphanumeric() || first == b'_' {
            let start = at;
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            tokens.push(bytes[start..at].to_ascii_lowercase());
        } else {
            tokens.push(vec![first]);
            at += 1;
        }
    }
    // sqlite_schema normally omits the last semicolon; do not alter inner ones.
    if tokens.last().is_some_and(|t| t == b";") {
        tokens.pop();
    }
    Ok(tokens)
}

pub fn hash<'a>(
    objects: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str)>,
) -> Result<[u8; 32], &'static str> {
    let mut schema = BTreeMap::new();
    for (kind, name, table, sql) in objects {
        schema.insert((kind, name, table), tokens(sql)?);
    }
    let mut h = Sha256::new();
    h.update(b"foks-read-dependency-schema-v1\0");
    for ((kind, name, table), tokens) in schema {
        for text in [kind, name, table] {
            h.update((text.len() as u64).to_le_bytes());
            h.update(text.as_bytes());
        }
        h.update((tokens.len() as u64).to_le_bytes());
        for token in tokens {
            h.update((token.len() as u64).to_le_bytes());
            h.update(token);
        }
    }
    Ok(h.finalize().into())
}
