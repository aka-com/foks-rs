use super::*;

pub(super) struct Input {
    pub(super) post: bool,
    pub(super) path: String,
    pub(super) query: BTreeMap<String, String>,
    pub(super) fields: BTreeMap<String, String>,
    pub(super) session: Option<Zeroizing<[u8; 32]>>,
    pub(super) pending: Option<Zeroizing<[u8; 32]>>,
}
pub(super) fn parse_fields(raw: &[u8]) -> Result<BTreeMap<String, String>> {
    if raw.len() > 16 * 1024 {
        return Err(Error::Config("form too large"));
    }
    // Reject malformed percent escapes and invalid UTF-8 instead of lossy decoding.
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            if i + 2 >= raw.len()
                || !raw[i + 1].is_ascii_hexdigit()
                || !raw[i + 2].is_ascii_hexdigit()
            {
                return Err(Error::Config("invalid form encoding"));
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = BTreeMap::new();
    for (k, v) in url::form_urlencoded::parse(raw) {
        if k.contains('\u{fffd}')
            || v.contains('\u{fffd}')
            || k.chars().any(char::is_control)
            || v.chars().any(char::is_control)
            || out.len() >= 32
            || out.insert(k.into_owned(), v.into_owned()).is_some()
        {
            return Err(Error::Config("invalid form"));
        }
    }
    Ok(out)
}
pub(super) struct Cookies {
    pub(super) session: Option<Zeroizing<[u8; 32]>>,
    pub(super) pending: Option<Zeroizing<[u8; 32]>>,
}
pub(super) fn parse_cookies(headers: &hyper::HeaderMap) -> Result<Cookies> {
    let mut found = BTreeMap::new();
    let mut bytes = 0;
    for raw in headers.get_all("cookie") {
        bytes += raw.as_bytes().len();
        if bytes > 4096 {
            return Err(Error::Config("cookie too large"));
        }
        let raw = raw.to_str().map_err(|_| Error::Config("invalid cookie"))?;
        for pair in raw.split(';') {
            let (k, v) = pair
                .trim()
                .split_once('=')
                .ok_or(Error::Config("invalid cookie"))?;
            if found.insert(k, v).is_some() {
                return Err(Error::Config("duplicate cookie"));
            }
        }
    }
    Ok(Cookies {
        session: found
            .get(SESSION)
            .map(|v| unhex(v).map(Zeroizing::new))
            .transpose()?,
        pending: found
            .get(PENDING)
            .map(|v| unhex(v).map(Zeroizing::new))
            .transpose()?,
    })
}
pub(super) fn fields(input: &Input, allowed: &[&str]) -> Result<()> {
    if input.fields.len() != allowed.len() || allowed.iter().any(|k| !input.fields.contains_key(*k))
    {
        return Err(Error::Config("invalid form fields"));
    }
    Ok(())
}
pub(super) fn query(input: &Input, allowed: &[&str]) -> Result<()> {
    if input.query.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(Error::Config("invalid query fields"));
    }
    Ok(())
}
pub(super) fn field<'a>(input: &'a Input, name: &str) -> Result<&'a str> {
    input
        .fields
        .get(name)
        .map(String::as_str)
        .ok_or(Error::Config("missing form field"))
}
pub(super) fn revision(input: &Input) -> Result<u64> {
    let v = field(input, "revision")?;
    if v.is_empty() || v.starts_with('0') || !v.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Config("invalid revision"));
    }
    v.parse().map_err(|_| Error::Config("invalid revision"))
}
pub(super) fn validate_route(input: &Input) -> Result<()> {
    if !input.post {
        return match input.path.as_str() {
            "/" => query(input, &["session"]),
            "/admin/sessions" => query(input, &["uid", "after"]),
            "/admin/invites" | "/admin/audit" => query(input, &["after"]),
            "/assets/admin.css" | "/login/confirm" | "/admin" => query(input, &[]),
            _ => Err(Error::Config("unknown admin route")),
        };
    }
    query(input, &[])?;
    match input.path.as_str() {
        "/login/confirm" | "/logout" => fields(input, &["csrf"]),
        "/admin/invites" => fields(input, &["csrf", "nonce"]),
        "/admin/signup-policy" => fields(input, &["csrf", "revision", "regime", "confirm"]),
        "/admin/sessions/revoke-all" => fields(input, &["csrf", "uid", "confirm"]),
        path => {
            if let Some(id) = path
                .strip_prefix("/admin/sessions/")
                .and_then(|v| v.strip_suffix("/revoke"))
            {
                unhex::<16>(id)?;
                fields(input, &["csrf", "uid"])
            } else if let Some(id) = path
                .strip_prefix("/admin/invites/")
                .and_then(|v| v.strip_suffix("/disable"))
            {
                unhex::<16>(id)?;
                fields(input, &["csrf", "revision"])
            } else {
                Err(Error::Config("unknown admin route"))
            }
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for value in self.query.values_mut().chain(self.fields.values_mut()) {
            value.zeroize();
        }
    }
}
