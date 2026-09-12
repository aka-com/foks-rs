use super::*;

pub(super) fn dispatch(service: &WebAdminService, input: Input) -> Result<Response<Body>> {
    service.require_ready()?;
    if !input.post && input.path == "/assets/admin.css" {
        query(&input, &[])?;
        let mut r = response(StatusCode::OK, render::CSS.into());
        r.headers_mut().insert(
            "content-type",
            hyper::header::HeaderValue::from_static("text/css; charset=utf-8"),
        );
        return Ok(r);
    }
    if !input.post && input.path == "/" {
        query(&input, &["session"])?;
        let Some(ticket) = input.query.get("session") else {
            return Ok(response(StatusCode::OK, render::landing()));
        };
        let ticket =
            service.parse_ticket(&format!("{}/?session={ticket}", service.config.origin()))?;
        let pending = service.random::<32>()?;
        let binding = hash(PENDING_DOMAIN, &*pending.0);
        let csrf = hash(VERIFY_CSRF_DOMAIN, &hash(PENDING_CSRF_DOMAIN, &*pending.0));
        let existing = input.session.as_ref().map(|v| hash(SESSION_DOMAIN, &**v));
        service
            .write(move |db, now| db.web_stage(&ticket, &binding, &csrf, existing.as_ref(), now))?;
        let mut r = redirect("/login/confirm");
        cookie(&mut r, PENDING, Some(&pending.hex()), 60)?;
        return Ok(r);
    }
    if input.path == "/login/confirm" {
        query(&input, &[])?;
        let pending = input
            .pending
            .as_ref()
            .ok_or(Error::Database(foks_server_db::Error::ReceiptExpired))?;
        let binding = hash(PENDING_DOMAIN, &**pending);
        if !input.post {
            if let Some(existing) = &input.session {
                let h = hash(SESSION_DOMAIN, &**existing);
                if service.read(|db, now| db.web_context(&h, now)).is_ok() {
                    return Err(Error::Database(foks_server_db::Error::ReceiptConflict));
                }
            }
            let ctx = service.read(|db, now| db.web_confirmation(&binding, now))?;
            let csrf = Zeroizing::new(hex(&hash(PENDING_CSRF_DOMAIN, &**pending)));
            return Ok(response(StatusCode::OK, render::confirmation(&ctx, &csrf)));
        }
        fields(&input, &["csrf"])?;
        let csrf = hash(VERIFY_CSRF_DOMAIN, &unhex::<32>(field(&input, "csrf")?)?);
        let session = service.random::<32>()?;
        let session_hash = hash(SESSION_DOMAIN, &*session.0);
        let session_csrf = hash(VERIFY_CSRF_DOMAIN, &hash(SESSION_CSRF_DOMAIN, &*session.0));
        let id = *service.random::<16>()?.0;
        let existing = input.session.as_ref().map(|v| hash(SESSION_DOMAIN, &**v));
        service.write(move |db, now| {
            db.web_redeem(
                foks_server_db::WebRedemption {
                    binding: &binding,
                    csrf: &csrf,
                    session: &session_hash,
                    session_csrf: &session_csrf,
                    id: &id,
                    existing: existing.as_ref(),
                },
                now,
            )
        })?;
        let mut r = redirect("/admin");
        cookie(&mut r, SESSION, Some(&session.hex()), 300)?;
        cookie(&mut r, PENDING, None, 0)?;
        return Ok(r);
    }
    if input.post && input.path == "/logout" {
        query(&input, &[])?;
        fields(&input, &["csrf"])?;
        let mut r = redirect("/");
        if let Some(raw) = input.session.as_ref() {
            let hash = hash(SESSION_DOMAIN, &**raw);
            match service.read(|db, now| db.web_context(&hash, now)) {
                Ok(ctx) => {
                    let auth = WebAdminService::auth(raw, &unhex::<32>(field(&input, "csrf")?)?);
                    service.write(move |db, now| {
                        db.web_revoke(&auth, &ctx.credential.uid, Some(&ctx.record_id), now)
                    })?;
                }
                Err(Error::Database(
                    foks_server_db::Error::ReceiptExpired
                    | foks_server_db::Error::AuthorizationChanged,
                )) => {}
                Err(e) => return Err(e),
            }
        }
        cookie(&mut r, SESSION, None, 0)?;
        cookie(&mut r, PENDING, None, 0)?;
        return Ok(r);
    }
    let raw = input
        .session
        .as_ref()
        .ok_or(Error::Database(foks_server_db::Error::ReceiptExpired))?;
    let session_hash = hash(SESSION_DOMAIN, &**raw);
    let csrf = Zeroizing::new(hex(&hash(SESSION_CSRF_DOMAIN, &**raw)));
    if !input.post {
        return match input.path.as_str() {
            "/admin" => {
                query(&input, &[])?;
                let (ctx, counts) = service.read(|db, now| db.web_overview(&session_hash, now))?;
                Ok(response(
                    StatusCode::OK,
                    render::overview(&ctx, counts, &csrf),
                ))
            }
            "/admin/sessions" => {
                query(&input, &["uid", "after"])?;
                let target = input.query.get("uid").map(|v| unhex::<33>(v)).transpose()?;
                let after = input
                    .query
                    .get("after")
                    .map(|v| unhex::<16>(v))
                    .transpose()?;
                let (ctx, rows) = service.read(|db, now| {
                    db.web_sessions(
                        &session_hash,
                        target.as_ref(),
                        after.as_ref().map(|v| v.as_slice()).unwrap_or(&[]),
                        now,
                    )
                })?;
                Ok(response(
                    StatusCode::OK,
                    render::sessions(
                        &ctx,
                        &rows,
                        target.as_ref().unwrap_or(&ctx.credential.uid),
                        &csrf,
                    ),
                ))
            }
            "/admin/invites" => {
                query(&input, &["after"])?;
                let after = input
                    .query
                    .get("after")
                    .map(|v| unhex::<16>(v))
                    .transpose()?;
                let (ctx, policy, rows) = service.read(|db, now| {
                    db.web_invites(
                        &session_hash,
                        after.as_ref().map(|v| v.as_slice()).unwrap_or(&[]),
                        now,
                    )
                })?;
                let nonce = service.random::<32>()?;
                let nonce_hash = hash(NONCE_DOMAIN, &*nonce.0);
                let auth = WebAdminService::auth(raw, &hash(SESSION_CSRF_DOMAIN, &**raw));
                service.write(move |db, now| db.web_prepare_invite(&auth, &nonce_hash, now))?;
                Ok(response(
                    StatusCode::OK,
                    render::invites(&ctx, &policy, &rows, &nonce.hex(), &csrf),
                ))
            }
            "/admin/audit" => {
                query(&input, &["after"])?;
                let after = input
                    .query
                    .get("after")
                    .map(|v| {
                        v.parse::<u64>()
                            .map_err(|_| Error::Config("invalid audit cursor"))
                    })
                    .transpose()?
                    .unwrap_or(0);
                let (ctx, rows) =
                    service.read(|db, now| db.web_audit(&session_hash, after, now))?;
                Ok(response(StatusCode::OK, render::audit(&ctx, &rows, &csrf)))
            }
            _ => Ok(failure(StatusCode::NOT_FOUND)),
        };
    }
    query(&input, &[])?;
    let auth = WebAdminService::auth(raw, &unhex::<32>(field(&input, "csrf")?)?);
    match input.path.as_str() {
        "/admin/invites" => {
            fields(&input, &["csrf", "nonce"])?;
            let nonce = unhex::<32>(field(&input, "nonce")?)?;
            let raw = service.random::<10>()?;
            let code = foks_proto::InviteCode::Standard(raw.0.to_vec());
            let code_hash = crate::invites::invite_fingerprint(&code)?;
            let display = Zeroizing::new(code.to_user_string()?);
            let id = *service.random::<16>()?.0;
            let invite = foks_server_db::WebInvite {
                nonce_hash: hash(NONCE_DOMAIN, &nonce),
                id,
                code_hash,
            };
            let result = service.write(move |db, now| db.web_issue_invite(&auth, invite, now))?;
            Ok(response(
                StatusCode::OK,
                match result {
                    foks_server_db::WebInviteResult::Created(id) => {
                        render::invite_result(&id, Some(&display))
                    }
                    foks_server_db::WebInviteResult::AlreadyCreated(id) => {
                        render::invite_result(&id, None)
                    }
                },
            ))
        }
        "/admin/signup-policy" => {
            fields(&input, &["csrf", "revision", "regime", "confirm"])?;
            if field(&input, "confirm")? != "yes" {
                return Err(Error::Config("confirmation required"));
            }
            let revision = revision(&input)?;
            let regime = match field(&input, "regime")? {
                "required" => foks_server_db::InviteRegime::Required,
                "optional" => foks_server_db::InviteRegime::Optional,
                _ => return Err(Error::Config("invalid signup policy")),
            };
            service.write(move |db, now| db.web_set_policy(&auth, regime, revision, now))?;
            Ok(redirect("/admin/invites"))
        }
        "/admin/sessions/revoke-all" => {
            fields(&input, &["csrf", "uid", "confirm"])?;
            if field(&input, "confirm")? != "yes" {
                return Err(Error::Config("confirmation required"));
            }
            let uid = unhex::<33>(field(&input, "uid")?)?;
            service.write(move |db, now| db.web_revoke(&auth, &uid, None, now))?;
            Ok(redirect("/admin/sessions"))
        }
        path if path.starts_with("/admin/sessions/") && path.ends_with("/revoke") => {
            fields(&input, &["csrf", "uid"])?;
            let id = unhex::<16>(
                path.strip_prefix("/admin/sessions/")
                    .and_then(|v| v.strip_suffix("/revoke"))
                    .ok_or(Error::Config("invalid session route"))?,
            )?;
            let uid = unhex::<33>(field(&input, "uid")?)?;
            service.write(move |db, now| db.web_revoke(&auth, &uid, Some(&id), now))?;
            Ok(redirect("/admin/sessions"))
        }
        path if path.starts_with("/admin/invites/") && path.ends_with("/disable") => {
            fields(&input, &["csrf", "revision"])?;
            let id = unhex::<16>(
                path.strip_prefix("/admin/invites/")
                    .and_then(|v| v.strip_suffix("/disable"))
                    .ok_or(Error::Config("invalid invite route"))?,
            )?;
            let revision = revision(&input)?;
            service.write(move |db, now| db.web_disable_invite(&auth, &id, revision, now))?;
            Ok(redirect("/admin/invites"))
        }
        _ => Ok(failure(StatusCode::NOT_FOUND)),
    }
}
