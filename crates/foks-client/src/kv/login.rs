//! The login layout the desktop writes into a small file, and the one parser
//! that reads it.
//!
//! The desktop stores a login as UTF-8 lines of `key: value`, for example
//! `user: X`, `password: Y` and `url: Z`. FOKS v0.1.9 has no login type, so to
//! the protocol this is an ordinary small file. Metadata synchronization uses
//! [`small_file_shape`] to report whether a small file holds a login before it
//! discards the plaintext, and the desktop host uses [`login_field`] to copy a
//! single field. Both read lines with [`field_line`].
//!
//! Lines are separated by `\n`, and one trailing `\r` is removed from each. A
//! field line is split at its first `:`. The key is the text before it,
//! untrimmed: it must be non-empty and contain no ASCII whitespace (space,
//! tab, line feed, form feed or carriage return), so an indented
//! `  password: x`, a spaced `password : x` and a sentence such as
//! `Reset it at https://example.com` are not field lines. A line whose `:` is
//! followed directly by `//`, such as a bare `https://example.com/reset`, is
//! a URL and not a field line either.
//! The value is the text after the `:` with exactly one leading space removed,
//! the separator the desktop writes; any other whitespace is part of the
//! value, so a password that begins or ends with a space is kept whole. Keys
//! are matched exactly, with the aliases the desktop's new-item draft
//! accepts: `user`/`username` and `url`/`website`. The first line naming a
//! field wins. The desktop renderer applies the same rules to text it has
//! read, in `apps/desktop/src/model/login.ts`.

use foks_client_db::KvSmallFileShape;

/// One field of a login small file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginField {
    Username,
    Password,
    Url,
}

impl LoginField {
    fn accepts(self, key: &str) -> bool {
        match self {
            Self::Username => key == "user" || key == "username",
            Self::Password => key == "password",
            Self::Url => key == "url" || key == "website",
        }
    }
}

/// The key and value of one line, or `None` when the line is not a field
/// line. `line` excludes its `\n`.
fn field_line(line: &str) -> Option<(&str, &str)> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (key, value) = line.split_once(':')?;
    let well_formed = !key.is_empty()
        && !key.contains(|c: char| c.is_ascii_whitespace())
        && !value.starts_with("//");
    well_formed.then(|| (key, value.strip_prefix(' ').unwrap_or(value)))
}

fn fields(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.split('\n').filter_map(field_line)
}

/// Whether a small file's plaintext is a login: UTF-8 in which every line
/// that is not blank is a field line, and one of them has the key `password`.
/// The password may be empty; the line alone is what the desktop writes for
/// every login. A document with any other line, such as indented YAML or an
/// INI section header, stays [`KvSmallFileShape::Other`].
pub fn small_file_shape(content: &[u8]) -> KvSmallFileShape {
    let Ok(text) = std::str::from_utf8(content) else {
        return KvSmallFileShape::Other;
    };
    let mut password = false;
    for line in text.split('\n') {
        match field_line(line) {
            Some((key, _)) => password |= LoginField::Password.accepts(key),
            None if line.bytes().all(|byte| byte.is_ascii_whitespace()) => {}
            None => return KvSmallFileShape::Other,
        }
    }
    if password {
        KvSmallFileShape::Login
    } else {
        KvSmallFileShape::Other
    }
}

/// The value of the first field line naming `field`, or `None` when no line
/// names it or that line's value is empty, which a login with an empty
/// password line has. The result borrows from `text`, so a caller holding the
/// plaintext in a zeroizing buffer copies only this value.
pub fn login_field(text: &str, field: LoginField) -> Option<&str> {
    fields(text)
        .find(|(key, _)| field.accepts(key))
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRITTEN: &str = "user: satoshi\npassword: hx7-Qm2!vTe9\nurl: https://github.com/login";

    #[test]
    fn the_desktop_login_layout_is_a_login() {
        assert_eq!(
            small_file_shape(WRITTEN.as_bytes()),
            KvSmallFileShape::Login
        );
        // New logins always write a url line, even with no website.
        assert_eq!(
            small_file_shape(b"user: a\npassword: \nurl: "),
            KvSmallFileShape::Login
        );
        assert_eq!(
            small_file_shape(b"ssid: Guest\r\npassword:x\r\n\r\n"),
            KvSmallFileShape::Login
        );
    }

    #[test]
    fn other_small_files_are_not_logins() {
        for content in [
            &b"postgres://service:o4Kq@db.internal/prod"[..],
            b"sk-ant-api03-Rf2xQ7bTn41La9mZ4",
            b"passwords: plural is a different key",
            b"Password: keys are matched exactly",
            b"",
            &[0xff, b'p', b'a', b's', b's', b'w', b'o', b'r', b'd', b':'],
            // The key must start the line and end at the colon.
            b"ssid: Guest\n  password: x",
            b"password : x",
            b"database:\n  host: db.internal\n  password: s3cret\n",
            // Every line that is not blank must be a field line.
            b"[db]\npassword: x",
            b"password: x\nrotate this every quarter",
            // A key has no inner whitespace, and `://` starts a URL.
            b"user: alice\npassword: hunter2\nReset it at https://example.com/reset",
            b"user: alice\npassword: hunter2\nhttps://example.com/reset",
            b"API key: x\npassword: y",
        ] {
            assert_eq!(small_file_shape(content), KvSmallFileShape::Other);
        }
    }

    #[test]
    fn each_field_is_read_alone_with_its_aliases() {
        assert_eq!(login_field(WRITTEN, LoginField::Username), Some("satoshi"));
        assert_eq!(
            login_field(WRITTEN, LoginField::Password),
            Some("hx7-Qm2!vTe9")
        );
        assert_eq!(
            login_field(WRITTEN, LoginField::Url),
            Some("https://github.com/login")
        );
        let aliases = "username: s\r\nwebsite: https://a.example:8443/x\r\npassword: p:q";
        assert_eq!(login_field(aliases, LoginField::Username), Some("s"));
        assert_eq!(
            login_field(aliases, LoginField::Url),
            Some("https://a.example:8443/x")
        );
        // Only the first colon separates the key.
        assert_eq!(login_field(aliases, LoginField::Password), Some("p:q"));
    }

    #[test]
    fn only_the_separator_space_and_a_carriage_return_are_removed() {
        let padded = "user: a\npassword:  pad-me \r\nurl: ";
        assert_eq!(login_field(padded, LoginField::Password), Some(" pad-me "));
        assert_eq!(login_field("password:x", LoginField::Password), Some("x"));
        assert_eq!(
            login_field("password: \ttab\u{feff}", LoginField::Password),
            Some("\ttab\u{feff}")
        );
        // A key with a byte-order mark is not a login key.
        assert_eq!(login_field("\u{feff}user: a", LoginField::Username), None);
        // Only `//` directly after the `:` marks a URL; a value may start
        // with it after the separator space.
        assert_eq!(
            login_field("password: //x", LoginField::Password),
            Some("//x")
        );
        assert_eq!(login_field("password://x", LoginField::Password), None);
    }

    #[test]
    fn a_missing_or_empty_field_is_absent_and_the_first_line_wins() {
        assert_eq!(
            login_field("user: a\npassword: b\nurl: ", LoginField::Url),
            None
        );
        assert_eq!(
            login_field("user: a\npassword: \nurl: ", LoginField::Password),
            None
        );
        assert_eq!(login_field("password: b", LoginField::Username), None);
        assert_eq!(login_field("plain text", LoginField::Password), None);
        assert_eq!(login_field("  password: b", LoginField::Password), None);
        assert_eq!(
            login_field("user: first\nusername: second", LoginField::Username),
            Some("first")
        );
    }
}
