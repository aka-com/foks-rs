//! Navigation origin allowlist and policy for the FOKS webview window.
//!
//! Restricts top-level navigation to trusted local origins, preventing loaded
//! pages from navigating to external origins where IPC commands
//! would remain accessible.
//!
//! URLs are checked against an exact origin allowlist (scheme, host, and port)
//! rather than prefix matches to avoid accepting adjacent hostnames or ports.
//!
//! While `tauri.conf.json` defines CSP directives for embedded frames and forms,
//! this native navigation handler enforces top-level window navigation boundaries.

use url::Url;

// The host and port of `build.devUrl` in `tauri.conf.json`, which is also
// `apps/desktop/vite.config.ts`'s `strictPort` server. Verified against configuration
// by a test below to avoid maintaining redundant definitions.
const DEVELOPMENT_HOST: &str = "127.0.0.1";
const DEVELOPMENT_PORT: u16 = 1421;

// Custom scheme used in production bundles (`tauri://localhost`) and the
// fallback hostname used by wry on certain platforms (`tauri.localhost`).
const ASSET_SCHEME: &str = "tauri";
const ASSET_HOST: &str = "localhost";
const ASSET_WORKAROUND_HOST: &str = "tauri.localhost";

/// Whether the main webview may navigate to `url`.
///
/// `development` is true only for a build that loads its frontend from the
/// Vite server — see [`policy`] for why that is `tauri::is_dev()` and not
/// `debug_assertions`.
pub(crate) fn allowed_navigation(url: &Url, development: bool) -> bool {
    // Reject URLs with embedded credentials to prevent origin misparsing.
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        // The production origin on macOS and Linux. `port()` is `None` unless
        // the URL carries an explicit one, and `tauri://localhost:1/` is a
        // different origin than the app's.
        ASSET_SCHEME => host_is(url, ASSET_HOST) && url.port().is_none(),
        // Allow default port connections while rejecting explicit non-default ports.
        "http" | "https" if host_is(url, ASSET_WORKAROUND_HOST) => url.port().is_none(),
        "http" => {
            development && host_is(url, DEVELOPMENT_HOST) && url.port() == Some(DEVELOPMENT_PORT)
        }
        // Every remote origin, `file:`, `data:`, `blob:`, `javascript:`, a
        // hostless URL, and any scheme Tauri did not register.
        _ => false,
    }
}

/// Hosts are case-insensitive. The `url` crate lowercases them for special
/// schemes and leaves an opaque host — which is what `tauri://localhost` has —
/// as written, so compare rather than assume.
fn host_is(url: &Url, host: &str) -> bool {
    url.host_str()
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(host))
}

/// The policy, as a plugin, so it is registered before the webview
/// `tauri.conf.json` declares is created.
///
/// Enforces navigation policy via a Tauri plugin registered during initialization.
///
/// Uses `tauri::is_dev()` to determine if the development origin is permitted,
/// matching Tauri's protocol selection logic.
pub(crate) fn policy<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("foks-navigation-policy")
        .on_navigation(|webview, url| {
            let allowed = if webview.label().starts_with("host-admin-") {
                crate::commands::web_admin::navigation_allowed(webview.label(), url)
            } else {
                allowed_navigation(url, tauri::is_dev())
            };
            if !allowed {
                // Log origin components only (scheme, host, port) to avoid leaking
                // sensitive data potentially contained in path or query parameters.
                tracing::warn!(
                    webview = webview.label(),
                    scheme = url.scheme(),
                    host = url.host_str().unwrap_or("<none>"),
                    port = ?url.port_or_known_default(),
                    "Blocked navigation outside allowed origin"
                );
            }
            allowed
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::{allowed_navigation, DEVELOPMENT_HOST, DEVELOPMENT_PORT};
    use url::Url;

    /// `(url, allowed in a packaged build, allowed in a development build)`.
    const TABLE: &[(&str, bool, bool)] = &[
        // The production origin, and paths, queries and fragments on it.
        ("tauri://localhost/", true, true),
        ("tauri://localhost/index.html", true, true),
        (
            "tauri://localhost/index.html?state=settings&store=%7B%22kind%22%3A%22account%22%7D",
            true,
            true,
        ),
        ("tauri://localhost/index.html#anchor", true, true),
        ("tauri://LOCALHOST/", true, true),
        // wry's Windows/Android workaround form of that same origin, with and
        // without `useHttpsScheme`, including its default port written out.
        ("http://tauri.localhost/", true, true),
        ("https://tauri.localhost/", true, true),
        ("http://tauri.localhost:80/", true, true),
        ("https://tauri.localhost:443/", true, true),
        // The Vite server, which only a development build loads.
        ("http://127.0.0.1:1421/", false, true),
        ("http://127.0.0.1:1421/index.html", false, true),
        ("https://127.0.0.1:1421/", false, false),
        // Disallowed adjacent origins.
        ("http://127.0.0.1:14210/", false, false),
        ("http://127.0.0.1:1420/", false, false),
        ("http://127.0.0.1/", false, false),
        ("http://127.0.0.1.evil.example/", false, false),
        ("http://localhost:1421/", false, false),
        ("http://tauri.localhost.evil.example/", false, false),
        ("https://tauri.localhost.example/", false, false),
        ("http://tauri.localhost:8080/", false, false),
        ("http://eviltauri.localhost/", false, false),
        ("tauri://localhost:1421/", false, false),
        ("tauri://evil.example/", false, false),
        ("http://user:pass@tauri.localhost/", false, false),
        // Remote origins and non-navigable schemes.
        ("https://example.com/", false, false),
        ("http://example.com/", false, false),
        ("https://foks.app/download", false, false),
        ("file:///tmp/test.html", false, false),
        ("file:///etc/passwd", false, false),
        ("data:text/html,<script>alert(1)</script>", false, false),
        ("blob:tauri://localhost/8e1f2c3d", false, false),
        ("javascript:fetch('https://example.com')", false, false),
        ("about:blank", false, false),
        ("ipc://localhost/", false, false),
        ("foks://drop-paths", false, false),
    ];

    #[test]
    fn the_navigation_table_holds_in_both_builds() {
        for (address, packaged, development) in TABLE {
            let url = Url::parse(address).unwrap_or_else(|error| panic!("{address}: {error}"));
            assert_eq!(
                allowed_navigation(&url, false),
                *packaged,
                "{address} in a packaged build"
            );
            assert_eq!(
                allowed_navigation(&url, true),
                *development,
                "{address} in a development build"
            );
        }
    }

    /// Verifies that packaged and development builds admit their configured initial URLs.
    #[test]
    fn each_build_admits_the_page_it_actually_loads() {
        let start = Url::parse("tauri://localhost/index.html").unwrap();
        assert!(allowed_navigation(&start, false));
        assert!(allowed_navigation(&start, true));

        let configuration: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json"))
                .expect("tauri.conf.json is valid JSON");
        let dev_url = configuration["build"]["devUrl"]
            .as_str()
            .expect("tauri.conf.json names a devUrl");
        let dev_url = Url::parse(dev_url).expect("devUrl is a URL");
        assert_eq!(dev_url.host_str(), Some(DEVELOPMENT_HOST));
        assert_eq!(dev_url.port(), Some(DEVELOPMENT_PORT));
        assert!(allowed_navigation(
            &dev_url.join("index.html").unwrap(),
            true
        ));
        assert!(!allowed_navigation(
            &dev_url.join("index.html").unwrap(),
            false
        ));
    }

    /// Verifies required Content Security Policy directives configured in `tauri.conf.json`.
    #[test]
    fn the_content_security_policy_enforces_expected_directives() {
        let configuration: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json"))
                .expect("tauri.conf.json is valid JSON");
        let csp = configuration["app"]["security"]["csp"]
            .as_str()
            .expect("tauri.conf.json declares a csp");
        let directives: Vec<&str> = csp.split(';').map(str::trim).collect();
        for required in [
            "default-src 'self'",
            "script-src 'self'",
            "style-src 'self' 'unsafe-inline'",
            "img-src 'self' data: blob:",
            "form-action 'none'",
            "base-uri 'none'",
            "object-src 'none'",
            "frame-src 'none'",
            "frame-ancestors 'none'",
        ] {
            assert!(
                directives.contains(&required),
                "the CSP is missing {required:?}: {csp}"
            );
        }
        assert!(
            !csp.contains("unsafe-eval"),
            "CSP must not permit unsafe-eval: {csp}"
        );
    }
}
