//! The login layout the desktop writes into a small file, and the one parser
//! that reads it.
//!
//! The desktop stores a login as UTF-8 lines of `key: value`, for example
//! `user: X`, `password: Y` and `url: Z`. FOKS v0.1.9 has no login type, so to
//! the protocol this is an ordinary small file. Metadata synchronization uses
//! [`small_file_shape`] to report whether a small file holds a login before it
//! discards the plaintext, and the desktop host uses [`login_field`] to copy a
//! single field. Both read lines the same way, so an item listed as a login
//! is one whose fields the host can find.
//!
//! A line is split at its first `:`; the key and the value are trimmed of
//! surrounding whitespace, which also drops a trailing `\r`. Keys are matched
//! exactly, with the aliases the desktop's new-item draft accepts:
//! `user`/`username` and `url`/`website`. The first line naming a field wins.

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

fn fields(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.split('\n').filter_map(|line| {
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        (!key.is_empty()).then(|| (key, value.trim()))
    })
}

/// Whether a small file's plaintext is a login: UTF-8 with a `password` key.
/// The password may be empty; the line alone is what the desktop writes for
/// every login.
pub fn small_file_shape(content: &[u8]) -> KvSmallFileShape {
    match std::str::from_utf8(content) {
        Ok(text) if fields(text).any(|(key, _)| LoginField::Password.accepts(key)) => {
            KvSmallFileShape::Login
        }
        _ => KvSmallFileShape::Other,
    }
}

/// The value of the first line naming `field`, or `None` when no line names
/// it or that line's value is empty. The result borrows from `text`, so a
/// caller holding the plaintext in a zeroizing buffer copies only this value.
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
            small_file_shape(b"ssid: Guest\r\n  password : x\r\n"),
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
    fn a_missing_or_empty_field_is_absent_and_the_first_line_wins() {
        assert_eq!(
            login_field("user: a\npassword: b\nurl: ", LoginField::Url),
            None
        );
        assert_eq!(login_field("password: b", LoginField::Username), None);
        assert_eq!(login_field("plain text", LoginField::Password), None);
        assert_eq!(
            login_field("user: first\nusername: second", LoginField::Username),
            Some("first")
        );
    }
}
