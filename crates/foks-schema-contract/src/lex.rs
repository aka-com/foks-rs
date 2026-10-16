use crate::ContractError;
use sha2::{Digest, Sha256};

/// Canonical bytes borrow their original SQL; folding never allocates a token.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Token<'a> {
    pub(crate) bytes: &'a [u8],
    fold: bool,
}
impl<'a> Token<'a> {
    fn canonical(self) -> impl Iterator<Item = u8> + 'a {
        self.bytes.iter().map(move |b| {
            if self.fold {
                b.to_ascii_lowercase()
            } else {
                *b
            }
        })
    }
    pub(crate) fn equals(self, other: Self) -> bool {
        self.bytes.len() == other.bytes.len() && self.canonical().eq(other.canonical())
    }
    pub(crate) fn hash(self, hash: &mut Sha256) {
        hash.update((self.bytes.len() as u64).to_le_bytes());
        if !self.fold {
            hash.update(self.bytes);
            return;
        }
        // A fixed stack buffer bounds temporary folding memory even for long SQL.
        let mut scratch = [0u8; 128];
        for chunk in self.bytes.chunks(scratch.len()) {
            for (out, byte) in scratch.iter_mut().zip(chunk) {
                *out = byte.to_ascii_lowercase();
            }
            hash.update(&scratch[..chunk.len()]);
        }
    }
    #[cfg(test)]
    pub(crate) fn owned(self) -> Vec<u8> {
        self.canonical().collect()
    }
}

struct Raw<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Raw<'a> {
    fn next_token(&mut self) -> Result<Option<Token<'a>>, ContractError> {
        while self.at < self.bytes.len() {
            let start = self.at;
            let first = self.bytes[start];
            if first.is_ascii_whitespace() {
                self.at += 1;
                continue;
            }
            if self.bytes[start..].starts_with(b"--") {
                while self.at < self.bytes.len() && self.bytes[self.at] != b'\n' {
                    self.at += 1;
                }
                continue;
            }
            if self.bytes[start..].starts_with(b"/*") {
                self.at += 2;
                while self.at < self.bytes.len() && !self.bytes[self.at..].starts_with(b"*/") {
                    self.at += 1;
                }
                if self.at == self.bytes.len() {
                    return Err(ContractError::UnterminatedComment);
                }
                self.at += 2;
                continue;
            }
            if matches!(first, b'\'' | b'"' | b'`' | b'[') {
                let end = if first == b'[' { b']' } else { first };
                self.at += 1;
                loop {
                    let byte = *self
                        .bytes
                        .get(self.at)
                        .ok_or(ContractError::UnterminatedQuote)?;
                    self.at += 1;
                    if byte == end {
                        if end != b']' && self.bytes.get(self.at) == Some(&end) {
                            self.at += 1;
                            continue;
                        }
                        break;
                    }
                }
                let inner = &self.bytes[start + 1..self.at - 1];
                // None of the qualified ALTER names contains an escaped quote.
                let qualified = [
                    b"chunk_objects".as_slice(),
                    b"chunk_object_attempts",
                    b"kv_file_chunks",
                    b"blocked_reason",
                    b"source_hash",
                ]
                .iter()
                .any(|name| inner.eq_ignore_ascii_case(name));
                return Ok(Some(if first != b'\'' && qualified {
                    Token {
                        bytes: inner,
                        fold: true,
                    }
                } else {
                    Token {
                        bytes: &self.bytes[start..self.at],
                        fold: false,
                    }
                }));
            }
            if first.is_ascii_alphanumeric() || first == b'_' {
                self.at += 1;
                while self.at < self.bytes.len()
                    && (self.bytes[self.at].is_ascii_alphanumeric() || self.bytes[self.at] == b'_')
                {
                    self.at += 1;
                }
                return Ok(Some(Token {
                    bytes: &self.bytes[start..self.at],
                    fold: true,
                }));
            }
            self.at += 1;
            return Ok(Some(Token {
                bytes: &self.bytes[start..self.at],
                fold: false,
            }));
        }
        Ok(None)
    }
}

/// One-token lookahead removes only the last semicolon. Errors poison/fuse the
/// iterator, so malformed trailing comments are observed before successful EOF.
pub(crate) struct Tokens<'a> {
    raw: Raw<'a>,
    pending: Option<Token<'a>>,
    initialized: bool,
    done: bool,
}
impl<'a> Tokens<'a> {
    pub(crate) fn new(sql: &'a str) -> Self {
        Self {
            raw: Raw {
                bytes: sql.as_bytes(),
                at: 0,
            },
            pending: None,
            initialized: false,
            done: false,
        }
    }
}
impl<'a> Iterator for Tokens<'a> {
    type Item = Result<Token<'a>, ContractError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if !self.initialized {
            self.initialized = true;
            match self.raw.next_token() {
                Ok(token) => self.pending = token,
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
        let Some(current) = self.pending.take() else {
            self.done = true;
            return None;
        };
        match self.raw.next_token() {
            Ok(Some(next)) => {
                self.pending = Some(next);
                Some(Ok(current))
            }
            Ok(None) => {
                self.done = true;
                if current.bytes == b";" {
                    None
                } else {
                    Some(Ok(current))
                }
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}
impl std::iter::FusedIterator for Tokens<'_> {}

pub(crate) fn validate(sql: &str) -> Result<u64, ContractError> {
    let mut count = 0u64;
    for token in Tokens::new(sql) {
        token?;
        count = count.checked_add(1).ok_or(ContractError::LengthOverflow)?;
    }
    Ok(count)
}

pub fn sql_equal(left: &str, right: &str) -> Result<bool, ContractError> {
    let mut left = Tokens::new(left);
    let mut right = Tokens::new(right);
    let mut equal = true;
    loop {
        match (left.next().transpose()?, right.next().transpose()?) {
            (None, None) => return Ok(equal),
            (Some(a), Some(b)) if a.equals(b) => {}
            _ => equal = false,
        }
    }
}
