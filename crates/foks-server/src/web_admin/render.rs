//! Escaped, packaged HTML. This module has no storage or key access.
use super::service::hex;
use foks_server_db::{
    AdminAuditEvent, InvitePolicy, InviteSnapshot, WebContext, WebSessionMetadata,
};
pub(crate) const CSS:&str="body{font:1rem system-ui,sans-serif;line-height:1.5;max-width:65rem;margin:2rem auto;padding:0 1rem;color:#18222d;background:#fff}a{color:#164ba0}nav{display:flex;gap:1rem;flex-wrap:wrap}table{border-collapse:collapse;width:100%;margin:1rem 0}th,td{text-align:left;padding:.5rem;border-bottom:1px solid #ccd3dc;overflow-wrap:anywhere}label{display:block;margin:.5rem 0}input,select,button{font:inherit;padding:.4rem}button{cursor:pointer;margin:.5rem 0}code{overflow-wrap:anywhere}a:focus-visible,button:focus-visible,input:focus-visible,select:focus-visible{outline:3px solid #a14d00;outline-offset:3px}.notice{border-left:4px solid #a14d00;padding:1rem}";
pub(crate) fn escape(input: &str) -> String {
    let mut out = String::new();
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
pub(crate) fn page(title: &str, body: &str) -> String {
    format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><link rel=\"stylesheet\" href=\"/assets/admin.css\"></head><body><main><h1>{}</h1>{body}</main></body></html>",escape(title),escape(title))
}
pub(crate) fn hidden(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        escape(name),
        escape(value)
    )
}
fn nav(ctx: &WebContext, csrf: &str) -> String {
    format!("<p>Host: <strong>{}</strong> · Account: <strong>{}</strong></p><nav aria-label=\"Administration\"><a href=\"/admin\">Overview</a><a href=\"/admin/sessions\">Browser sessions</a>{}</nav><form method=\"post\" action=\"/logout\">{}<button>Sign out</button></form>",escape(&ctx.host_name),escape(&ctx.username),if ctx.operator{"<a href=\"/admin/invites\">Signup access</a><a href=\"/admin/audit\">Audit log</a>"}else{""},hidden("csrf",csrf))
}
pub(crate) fn landing() -> String {
    page("Host administration","<p>Open administration from your unlocked FOKS account to sign in.</p><p>Expired access must be recovered in the native application.</p>")
}
pub(crate) fn confirmation(ctx: &WebContext, csrf: &str) -> String {
    page("Confirm account sign-in",&format!("<p>You are signing in to <strong>{}</strong> as <strong>{}</strong>.</p><p>Continue only if this is the host and account you intended to use.</p><form method=\"post\" action=\"/login/confirm\">{}<button>Confirm and sign in</button></form><a href=\"/\">Cancel</a>",escape(&ctx.host_name),escape(&ctx.username),hidden("csrf",csrf)))
}
pub(crate) fn overview(
    ctx: &WebContext,
    counts: Option<foks_server_db::WebOverview>,
    csrf: &str,
) -> String {
    let mut body = nav(ctx, csrf);
    body.push_str("<p>This browser session lasts at most five minutes. Native app lock closes its window; use sign out or revoke-all to revoke server access.</p>");
    if ctx.operator {
        body.push_str(&format!("<h2>Host operator</h2><p>Host ID: <code>{}</code></p><p>Service ready · Software {} · Database schema {}</p><p>Manage host signup access, browser sessions and the local audit log using the links above.</p>",hex(&ctx.credential.host),env!("CARGO_PKG_VERSION"),foks_server_db::SCHEMA_VERSION));
    }
    if let Some(c) = counts {
        body.push_str(&format!("<p>Accounts: {} · Host operators: {} · Signup invites: {} · Live browser sessions: {}</p>",c.accounts,c.operators,c.invites,c.sessions));
    }
    page("Host administration", &body)
}
pub(crate) fn sessions(
    ctx: &WebContext,
    rows: &[WebSessionMetadata],
    target: &[u8; 33],
    csrf: &str,
) -> String {
    let mut body = nav(ctx, csrf);
    if ctx.operator {
        body.push_str("<form method=\"get\" action=\"/admin/sessions\"><label>Local account UID <input name=\"uid\" required pattern=\"[0-9a-f]{66}\" maxlength=\"66\"></label><button>Inspect account</button></form>");
    }
    body.push_str("<h2>Browser sessions</h2><table><thead><tr><th>Session</th><th>Device</th><th>Created / expires (UTC microseconds)</th><th>Action</th></tr></thead><tbody>");
    for row in rows {
        body.push_str(&format!("<tr><td><code>{}</code>{}</td><td><code>{}</code></td><td>{} / {}</td><td><form method=\"post\" action=\"/admin/sessions/{}/revoke\">{}{}<button{}>Revoke</button></form></td></tr>",hex(&row.id),if row.id==ctx.record_id{" (current)"}else{""},hex(&row.credential),row.created_at_us,row.expires_at_us,hex(&row.id),hidden("csrf",csrf),hidden("uid",&hex(&row.uid)),if row.revoked{" disabled"}else{""}));
    }
    body.push_str("</tbody></table>");
    if rows.len() == 100 {
        body.push_str(&format!(
            "<a href=\"/admin/sessions?uid={}&amp;after={}\">Next page</a>",
            hex(target),
            hex(&rows[99].id)
        ));
    }
    body.push_str(&format!("<form method=\"post\" action=\"/admin/sessions/revoke-all\">{}{}<label><input type=\"checkbox\" name=\"confirm\" value=\"yes\" required> Revoke all browser sessions and pending login links for this account</label><button>Revoke all</button></form>",hidden("csrf",csrf),hidden("uid",&hex(target))));
    page("Browser sessions", &body)
}
pub(crate) fn invites(
    ctx: &WebContext,
    policy: &InvitePolicy,
    rows: &[InviteSnapshot],
    nonce: &str,
    csrf: &str,
) -> String {
    let mut body = nav(ctx, csrf);
    body.push_str(&format!("<h2>Signup policy</h2><p>Current policy: {:?}. Revision {}.</p><form method=\"post\" action=\"/admin/signup-policy\">{}{}<label>New policy <select name=\"regime\"><option value=\"required\">Invite required</option><option value=\"optional\">Invite optional</option></select></label><label><input type=\"checkbox\" name=\"confirm\" value=\"yes\" required> Apply this host signup policy</label><button>Update policy</button></form><h2>Issue a single-use invite</h2><form method=\"post\" action=\"/admin/invites\">{}{}<button>Create invite</button></form><h2>Invites</h2><table><thead><tr><th>ID</th><th>Kind / uses</th><th>State / revision</th><th>Action</th></tr></thead><tbody>",policy.regime,policy.revision,hidden("csrf",csrf),hidden("revision",&policy.revision.to_string()),hidden("csrf",csrf),hidden("nonce",nonce)));
    for row in rows {
        body.push_str(&format!("<tr><td><code>{}</code></td><td>{:?} / {}</td><td>{} / {}</td><td><form method=\"post\" action=\"/admin/invites/{}/disable\">{}{}<button{}>Disable</button></form></td></tr>",hex(&row.invite_id),row.kind,row.use_count,if row.active{"Active"}else{"Disabled"},row.configuration_revision,hex(&row.invite_id),hidden("csrf",csrf),hidden("revision",&row.configuration_revision.to_string()),if row.active{""}else{" disabled"}));
    }
    body.push_str("</tbody></table>");
    if rows.len() == 100 {
        body.push_str(&format!(
            "<a href=\"/admin/invites?after={}\">Next page</a>",
            hex(&rows[99].invite_id)
        ));
    }
    page("Host signup access", &body)
}
pub(crate) fn invite_result(id: &[u8; 16], code: Option<&str>) -> String {
    page("Signup invite",&format!("<p>Invite ID: <code>{}</code></p>{}<a href=\"/admin/invites\">Return to signup access</a>",hex(id),code.map(|v|format!("<p class=\"notice\">Copy this code now. It is shown only once.</p><p><code>{}</code></p>",escape(v))).unwrap_or_else(||"<p>This request already created the invite. Its secret code is no longer available. Disable it and create a new invite if needed.</p>".into())))
}
pub(crate) fn audit(ctx: &WebContext, rows: &[AdminAuditEvent], csrf: &str) -> String {
    let mut body = nav(ctx, csrf);
    body.push_str("<p>Local operational audit: at most 90 days and 100,000 events. The cap can shorten retention; this log is not tamper-proof.</p><table><thead><tr><th>Event</th><th>Actor</th><th>Action</th><th>Target</th><th>Time (UTC microseconds)</th></tr></thead><tbody>");
    for row in rows {
        let action = match row.action {
            1 => "Grant operator",
            2 => "Revoke operator",
            3 => "Issue login ticket",
            4 => "Browser sign-in",
            5 => "Revoke session",
            6 => "Revoke all sessions",
            7 => "Issue invite",
            8 => "Disable invite",
            9 => "Change signup policy",
            _ => "Unknown",
        };
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{action}</td><td><code>{}</code></td><td>{}</td></tr>",
            row.id,
            row.actor_uid
                .map(|v| hex(&v))
                .unwrap_or_else(|| "Offline operator".into()),
            hex(&row.target),
            row.occurred_at_us
        ));
    }
    body.push_str("</tbody></table>");
    if rows.len() == 100 {
        body.push_str(&format!(
            "<a href=\"/admin/audit?after={}\">Next page</a>",
            rows[99].id
        ));
    }
    page("Administration audit", &body)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_text_is_escaped() {
        assert_eq!(escape("<&\"'>"), "&lt;&amp;&quot;&#39;&gt;");
        assert!(!page("<script>", "safe").contains("<script>"));
    }
}
