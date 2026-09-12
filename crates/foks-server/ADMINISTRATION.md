# Standalone host administration

The optional Rust-hosted application manages host signup invites/policy and browser
sessions. Explicit offline grants authorize host operators. Team roles, OIDC claims,
email addresses and localhost connections never grant host authority. Ordinary
accounts can inspect and revoke their own browser sessions.

Enable one dedicated HTTPS origin through a reverse proxy. Add to the installation
TOML (disabled when absent):

```toml
[web_admin]
origin = "https://admin.example.net"
listen = "127.0.0.1:8445"
```

The listener must be loopback-only with a nonzero port, distinct from all other
listeners. The installation needs at least three read connections and two writer
queue slots: two read connections and at most one pending writer job are reserved
for administration. Default installation limits satisfy these requirements.

An nginx example, with an operator-provisioned browser-trusted certificate:

```nginx
server {
    listen 443 ssl;
    server_name admin.example.net;
    ssl_certificate /etc/ssl/admin/fullchain.pem;
    ssl_certificate_key /etc/ssl/admin/privkey.pem;
    client_max_body_size 16k;
    client_header_buffer_size 16k;
    large_client_header_buffers 2 8k;
    client_header_timeout 5s;
    client_body_timeout 5s;
    keepalive_timeout 5s;
    access_log off;
    error_log /dev/null crit;
    location / {
        proxy_pass http://127.0.0.1:8445;
        proxy_set_header Host admin.example.net;
        proxy_set_header Connection close;
        proxy_set_header Forwarded "";
        proxy_set_header X-Forwarded-For "";
        proxy_set_header X-Forwarded-Host "";
        proxy_read_timeout 15s;
        proxy_send_timeout 5s;
        proxy_buffering off;
    }
}
```

Preserve the exact external authority, including a non-default port. Do not log
request queries, cookies, form bodies or response headers at the proxy. Disable
request inspection/analytics that retain these secrets. Forwarded headers carry no
authority in Rust. Keep this origin dedicated to administration. The probe's
self-signed certificate is not a browser certificate; never disable certificate
verification in the native webview.

## Bootstrap an operator

1. Initialize and run the installation normally; create an ordinary local account
   through the native application. On OIDC installations, link that account first.
   Migration eligibility alone does not admit browser administration.
2. Stop the server and grant its exact existing UID:

   ```sh
   foks-server admin grant --config /srv/foks/server.toml \
     --uid LOWERCASE_HEX_UID --reason "Initial host operator"
   foks-server admin list --config /srv/foks/server.toml
   ```

   Mutations hold the same `.writer-lock` as the live server and refuse a running
   writer. Output identifies host, UID, account and revision. List is read-only and
   paginated; use `--after LAST_UID` for the next page. No user is automatically
   promoted. To remove a grant, use `admin revoke` with the same arguments and a
   reason. A grant change and its audit event commit together.
3. Validate configuration with `foks-server config-check --config ...`, configure
   the proxy, and restart. `serve-config` enables the full listener and RPC product.
4. In the selected unlocked native account, explicitly configure the HTTPS admin
   destination, then open Host administration. Native code independently validates
   the destination and ticket ownership. Confirm the named host and account in the
   clean browser page. The CLI's `admin check` verifies support without printing a
   bearer URL; each successful check occupies a short-lived ticket slot.

## Sessions and signup access

Login URLs hold random 20-byte, Go-Base62 tickets lasting at most 60 seconds. GET
only stages an independent pending cookie and redirects away from the secret URL.
Confirmation POST atomically consumes the ticket and installs a separate random
32-byte browser cookie. A consumed link cannot be checked or redeemed again; this
is a deliberate difference from reusable Go-host sessions. The pinned Go native
issue/check order is tested before browser redemption.

Cookies use `__Host-`, Secure, HttpOnly, SameSite=Strict and Path=/. Browser sessions
last at most five minutes, capped by certificate expiry, without sliding renewal.
Every request resolves the current credential and active parent, local account,
SSO policy epoch/account generation and current operator grant. Routine native
refresh preserves the SSO stamp; explicit reauthentication requires a new browser
entry. Provider fencing or expired access denies requests and directs recovery to
the native application. This webview never redirects to an identity provider.

Deadline clocks include suspend time (Linux BOOTTIME; macOS MONOTONIC_RAW). A
backwards UTC step fences the epoch; after UTC catches up, fresh native entry uses
a new epoch. Restart and database restore also invalidate every old cookie, ticket,
confirmation and form nonce. Copying a cookie does not defeat the original deadline.
Native app lock closes its private window but does not revoke a copied cookie.
Use browser sign out or revoke-all for explicit server revocation. Revoke-all also
invalidates pending native login tickets.

Operators create generated single-use signup invites, shown exactly once. A retry
of the same form returns the committed invite ID without regenerating or storing
its plaintext code. Lost secret responses require disabling that invite and creating
another. Form admission lasts at most 60 seconds; committed receipts remain until
session expiry. Lists and disable support standard and multiuse invites; creating
an operator-chosen multiuse code remains an offline command. These are host signup
invites, separate from team invitations.

Policy and invite changes compare configuration/state revisions. Ordinary invite
redemption updates usage without changing configuration revision. A redemption
committed before disable may succeed; a disable committed first rejects redemption.
A stale form receives a conflict and must be reloaded.

## Operations and recovery

Requests have independent limits: 32 connections, 16 application jobs, 8 KiB URI,
16 KiB headers/body, 32 fields, 100 rows per page and 128 KiB output. Ticket,
confirmation and session caps are respectively 3/6/5 per account and
1,024/2,048/4,096 globally. Invite forms permit eight pending per session and 8,192
retained total, including consumed receipts. Capacity rejects new work without
silently evicting live credentials. Cookies and secrets are absent from metrics.

Maintenance deletes at most 128 rows per ephemeral family per pass, preserving
foreign-key ownership and committed nonce retention. Existing unrelated expiry
sweeps are bounded too. Metrics report `foks_admin_cleanup_attempts_total`,
`foks_admin_cleanup_failures_total` and `foks_admin_reclaimed_records_total` alongside
shared maintenance failure counters.

The local audit records successful typed actions, opaque targets and revisions,
atomically with mutations. It retains at most 90 days and 100,000 events; the cap
can shorten retention. It is operational history, not tamper-proof evidence.
Unauthenticated errors do not create unbounded audit rows.

Backups preserve durable operator grants and audit history. Restoring an older
backup can restore a previously revoked grant: inspect `admin list` before reopening
the listener. The new process epoch invalidates backed-up browser credentials.
Removing `[web_admin]` disables the listener and both handoff RPCs. Client state
relocation/import carries the encrypted account destination policy and requires
fresh native entry; it does not copy server-side browser sessions or proxy config.

Mutable installation/OIDC configuration, key operations, backups, account identity
changes, online grants, teams, billing and multitenant provisioning are outside this
browser release. Offline operator tools remain available if browser access is lost.
