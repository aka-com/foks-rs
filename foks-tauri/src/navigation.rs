//! Where the FOKS window is allowed to go.
//!
//! The capability file (`PERMISSIONS.md`) bounds what a script inside the
//! window may *call*. This bounds where the window may *be*. They are separate
//! controls: a page that navigates itself to an attacker's origin keeps the
//! window, the title bar and the process, and every custom command in
//! `commands.rs` is then reachable from content FOKS did not build.
//!
//! The classification is a pure function of a URL so the table below is the
//! specification and the test at once. It is deliberately an allowlist of
//! whole origins — scheme, host and effective port, compared exactly — rather
//! than a prefix match, because `https://tauri.localhost.example/` and
//! `http://127.0.0.1:14210/` both pass a prefix match and neither is this app.
//!
//! CSP is the second line, not this one: `frame-src`/`form-action`/`base-uri`
//! in `tauri.conf.json` close the mechanisms CSP does cover, but CSP does not
//! reliably govern every top-level navigation, so the native policy is primary.

use url::Url;

// The host and port of `build.devUrl` in `tauri.conf.json`, which is also
// `foks-ui/vite.config.ts`'s `strictPort` server. Pinned to the configuration
// by a test below rather than remembered in two places.
const DEVELOPMENT_HOST: &str = "127.0.0.1";
const DEVELOPMENT_PORT: u16 = 1421;

// The custom scheme Tauri serves the embedded bundle from on macOS and Linux
// (`tauri://localhost`), and the host of the `http(s)://tauri.localhost`
// workaround form wry needs on Windows and Android. FOKS ships the first two
// platforms; the workaround form is allowed so that adding a target, or
// turning on `useHttpsScheme`, is not a silent blank window.
const ASSET_SCHEME: &str = "tauri";
const ASSET_HOST: &str = "localhost";
const ASSET_WORKAROUND_HOST: &str = "tauri.localhost";

/// Whether the main webview may navigate to `url`.
///
/// `development` is true only for a build that loads its frontend from the
/// Vite server — see [`policy`] for why that is `tauri::is_dev()` and not
/// `debug_assertions`.
pub(crate) fn allowed_navigation(url: &Url, development: bool) -> bool {
    // Tauri never emits either, and an origin is unchanged by them, but they
    // are a classic way to make a denied host read like an allowed one.
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        // The production origin on macOS and Linux. `port()` is `None` unless
        // the URL carries an explicit one, and `tauri://localhost:1/` is a
        // different origin than the app's.
        ASSET_SCHEME => host_is(url, ASSET_HOST) && url.port().is_none(),
        // `port()` is `None` for the scheme's own default port, so this admits
        // `http://tauri.localhost:80/` — the same origin — and refuses
        // `http://tauri.localhost:8080/`, which is not.
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
/// The window comes from the configuration, so there is no `WebviewWindow`
/// builder here to hang `on_navigation` off; replacing it with a manually
/// built window to obtain one would be a far larger lifecycle and packaging
/// change than this control is worth. Every plugin's hook is consulted and any
/// `false` cancels the navigation, so this composes rather than competes.
///
/// `tauri::is_dev()`, not `cfg!(debug_assertions)`, decides whether the
/// development origin is allowed: it is `!cfg!(feature = "custom-protocol")`,
/// the *same* switch `tauri`'s own `get_app_url` reads to choose between
/// `build.devUrl` and the embedded bundle. This repository does not declare a
/// `custom-protocol` feature on the package, so `cargo build --release`
/// produces a binary that still loads the Vite server (see
/// `pnpm run foks:build`) — `debug_assertions` would deny that binary its own
/// start page. The distributable artifacts turn the feature on (Bazel through
/// `MODULE.bazel`'s crate annotation, `tauri build` through the CLI), so they
/// are the builds where the development origin is refused.
pub(crate) fn policy<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("foks-navigation-policy")
        .on_navigation(|webview, url| {
            let allowed = allowed_navigation(url, tauri::is_dev());
            if !allowed {
                // Scheme, host and port only. A denied URL's path, query and
                // fragment are attacker-chosen and can carry a value that was
                // in the window; the origin is what a reader needs.
                tracing::warn!(
                    webview = webview.label(),
                    scheme = url.scheme(),
                    host = url.host_str().unwrap_or("<none>"),
                    port = ?url.port_or_known_default(),
                    "FOKS refused a navigation away from its own origin"
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
        // Neighbours of every allowed origin. None of these is this app.
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

    /// The one origin a packaged build must accept is the one it is served
    /// from, and the one a development build must accept is the one
    /// `tauri.conf.json` points it at. Stated separately from the table so a
    /// table edit cannot quietly delete the app's own start page.
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

    /// The CSP is defence in depth for the same boundary, so the directives it
    /// is depended on for are pinned here rather than left to a reading of the
    /// configuration.
    #[test]
    fn the_content_security_policy_closes_the_mechanisms_it_covers() {
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
            "a window that renders secrets does not evaluate strings: {csp}"
        );
    }
}
