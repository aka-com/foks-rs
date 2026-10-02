# Research note: desktop application, MCP server, and engineering practices in foks-rs

Repository: `/home/bnoland/projects/foks-rs` (HEAD `2ff0918`, 2026-09-22, 377 commits on the checked-out branch).
All paths below are relative to that root unless absolute. Line numbers are from the checkout as read on 2026-09-25.

---

## Part A. Repository overview (factual)

### A.1 What it is

foks-rs is a Rust reimplementation of FOKS, the Federated Open Key Store. The upstream project is the Go implementation at `github.com/foks-proj/go-foks`; the public host used as a live oracle is `foks.app:4430`. The pinned upstream version is **v0.1.9** (not "v0.19"): `tools/foks-v019-oracle/go.mod:6` declares `github.com/foks-proj/go-foks v0.1.9`, the crate descriptions in `crates/*/Cargo.toml` say "FOKS v0.1.9", the generated protocol baseline is `crates/foks-server/protocol/upstream-v0.1.9.json`, and CI fetches `go mod download github.com/foks-proj/go-foks@v0.1.9` (`.github/workflows/foks-standalone.yml`). The `README.md` states that "The FOKS project is pre-v1. Compatibility with the upstream Go protocol is tested explicitly; internal storage and application schemas may otherwise change without migration support."

The workspace version is `0.4.0` (`Cargo.toml` `[workspace.package]`), toolchain Rust 1.95.0 (`rust-toolchain.toml`), edition 2021. The desktop frontend uses React 19, Vite 8, TypeScript 6 (`package.json`), with `@tauri-apps/api ^2` and Tauri 2 (`apps/desktop/src-tauri/Cargo.toml`).

### A.2 Line counts (rough, `wc -l` over `*.rs` / `*.ts(x)`)

- Rust, all crates plus the Tauri shell: ~304,000 lines.
- Desktop TypeScript/TSX (`apps/desktop/src` + `apps/desktop/kit`): ~74,500 lines.
- Desktop tests (`apps/desktop/tests`): ~62,800 lines.
- Tauri shell (`apps/desktop/src-tauri`, Rust + JSON contracts): ~35,000 lines, of which `src/agent.rs` alone is 5,122 and `src/commands/context.rs` 2,434.

### A.3 Crate list (24 crates + the Tauri app package)

| Crate | Lines | One-line purpose (from Cargo `description` or README) | Depends on (workspace) |
|---|---|---|---|
| foks-snowpack | 1.9k | Canonical Snowpack (MessagePack-derived) codec byte-compatible with Go v0.1.9 | — |
| foks-proto | 12.7k | Exact FOKS v0.1.9 protocol types | snowpack |
| foks-crypto | 10.0k | FOKS v0.1.9 cryptographic primitives | proto, snowpack |
| foks-merkle-store | 1.0k | Deterministic storage-neutral Merkle tree | crypto, proto, snowpack |
| foks-verify | 8.1k | Stateless verification of host, identity and team chains | crypto, proto, snowpack |
| foks-rpc | 8.6k | Strict Snowpack RPC framing and client calls | crypto, proto, snowpack |
| foks-oidc | 0.8k | Bounded OIDC provider / identity policy | — |
| foks-keystore | 1.5k | Native credential stores (macOS Keychain, Linux Secret Service, private file) | — |
| foks-yubi | 1.8k | Isolated macOS/Linux YubiKey PIV provider | crypto, proto, snowpack |
| foks-client-db | 15.1k | Durable hard and soft SQLite state for a native client | crypto, proto, snowpack, verify |
| foks-client | 43.5k | Native identity, recovery, team and KV client | client-db, crypto, merkle-store, oidc, proto, rpc, snowpack, verify, yubi |
| foks-client-app | 47.7k | Application-level client (profiles, sessions, vaults, chat operations) | agent-proto, client, client-db, compat-artifact, crypto, keystore, oidc, proto, protocol-metadata, rpc, server-db, server-testkit, verify, yubi |
| foks-agent-proto | 6.0k | Local agent IPC protocol (JSON over Unix socket), incl. `chat-limits.json` | proto |
| foks-agent-client | 1.0k | Client library for the local agent socket | agent-proto |
| foks-agent | 16.5k | The resident `foks-agent` daemon | most client crates |
| foks-cli | 5.4k | `foks-rs` command-line tool, incl. `mcp kv` / `mcp team` | agent-client, agent-proto, client-app, compat-artifact, keystore, mcp, oidc, server-testkit, yubi |
| foks-mcp | 1.9k | Model Context Protocol server over stdio | agent-client, agent-proto, crypto, proto |
| foks-desktop | 8.3k | Renderer-independent desktop command library + scripted `foks-desktop-backend` | agent-client, agent-proto |
| foks-desktop-app (`apps/desktop/src-tauri`) | ~24k Rust | The shipping Tauri binary `foks-desktop` | client-app, keystore, yubi, agent-client, agent-proto, compat-artifact, crypto, desktop, verify, protocol-metadata |
| foks-server-db | 24.1k | Authoritative standalone SQLite storage for the server | crypto, merkle-store, proto |
| foks-server | 32.1k | Standalone single-host server | client-db, crypto, merkle-store, oidc, proto, rpc, server-db, snowpack, verify |
| foks-server-testkit | 21.3k | Sealed isolated-process test composition (spawns real server/agent) | nearly everything incl. desktop |
| foks-protocol-metadata | 1.5k | Validation and deterministic generation of pinned protocol metadata | — |
| foks-compat-artifact | 0.6k | Signed hosted-compatibility artifact ("lease") format | — |
| foks-go-interop | 1.2k | Reader for legacy Go client hard state | snowpack |

Dependency shape: a strict layering from `snowpack -> proto -> crypto -> {merkle-store, verify, rpc} -> client-db -> client -> client-app -> agent`, with `agent-proto`/`agent-client` as the thin IPC seam that `cli`, `mcp`, `desktop` and the Tauri app all sit on. The default `cargo test` members are the seven "core" crates (`client-db, crypto, proto, snowpack, verify, client, rpc`); the Tauri package is a workspace member but excluded from default members because it needs the platform webview toolchain (`Cargo.toml` `default-members`).

### A.4 Products built from one workspace

- `foks-server` (standalone single-host server, SQLite single-writer + WAL readers).
- `foks-agent` (resident daemon; owns keys, sessions, hard/soft SQLite state).
- `foks-rs` CLI (includes the MCP server entry point).
- `foks-desktop` (Tauri 2 + React 19 shell over the agent), macOS arm64 DMG and Linux x86_64 .deb.

---

## Part B. Desktop application techniques

### B.1 Three-tier trust architecture (webview / Tauri host / agent daemon)

**Problem.** A monolithic desktop credential manager puts web code and key material in one process; one XSS or supply-chain bug in the renderer exposes everything.

**How it works.** The app is decomposed into (1) an untrusted WebKit webview running React, (2) a privileged Rust "host" (`foks-desktop-app`) that owns the window, the OS integrations and every command, and (3) a separate resident `foks-agent` process that holds credentials and does all cryptography, reached over a Unix domain socket `foks-rs.sock`. The docs state plaintext file bytes, vault keys and private signing scalars "never cross the IPC boundary into the webview renderer" (`docs/DESKTOP_APP.html`, "Architecture Summary"; `docs/INTRO.html`, section 1). The agent survives window close; the CLI and MCP server connect to the same socket without the GUI (`docs/DESKTOP_APP.html` section 1, "Why Separate the Agent").

Socket and process hygiene, all in `apps/desktop/src-tauri/src/agent.rs`:
- `validate_agent_binary` (line 2319): the packaged agent must be a non-symlink regular file, owned by the same uid as the desktop executable, not group/world-writable (`mode & 0o022 == 0`) and executable.
- `prepare_state_directory` (~line 2347): state dir must be a directory owned by the euid, forced to `0700`.
- Socket metadata must not have group/world bits (`mode & 0o077 != 0` is refused, lines 1104, 2302, 2613) and must be owned by the euid.
- `unix_peer_pid` (line 3153) reads the peer PID via `LOCAL_PEERPID` on macOS and `SO_PEERCRED` on Linux so a takeover of an incompatible agent can name the exact process; `stop_incompatible_agent` (line 2687) rechecks device/inode before and after the confirmation dialog to defeat TOCTOU swaps (described in `docs/DESKTOP_APP.html` section 3).
- The managed agent is started with `--request-timeout-seconds` (line 2405); the frontend's queue derives its own wait budget from that 60 s figure (`apps/desktop/src/scheduling/profile-work.ts:63-70`).

**Why.** `apps/desktop/src-tauri/README.md` and `PERMISSIONS.md` say the whole design "keeps file bytes out of the renderer"; `crates/foks-desktop/README.md` says "Agent calls in that app run on blocking workers, never the webview thread."

**References.** Tauri v2 process model docs (https://v2.tauri.app/concept/process-model/); OWASP "Least privilege" and the Chromium sandbox rationale for untrusted renderers [VERIFY exact URLs].

### B.2 Capability allowlist of exactly three Tauri core permissions, pinned by test

**Problem.** Tauri's `core:default` permission set expands to ~92 commands including `core:image|from_path` (read any file the webview names) and `internal_toggle_devtools`.

**How it works.** `apps/desktop/src-tauri/capabilities/default.json` grants only `core:event:allow-listen`, `core:event:allow-unlisten`, `core:window:allow-start-dragging` for window `main`. A unit test in `src/lib.rs` (`the_resolved_capability_reaches_only_those_three_commands`, line 511; `RENDERER_COMMANDS` const at line 446) parses the `tauri-build` generated `gen/schemas/acl-manifests.json` and `capabilities.json`, resolves every permission to concrete `manifest|command` pairs, asserts the set equals exactly `{core:event|listen, core:event|unlisten, core:window|start_dragging}`, and additionally asserts a list of forbidden commands (`core:image|from_path`, `core:webview|internal_toggle_devtools`, `core:path|resolve`, ...) is unreachable (lines 543-551). `emit`/`emit_to` are deliberately not granted, so the renderer cannot synthesize drop events; drag-drop events flow one way from Rust (`src/dragdrop.rs:24-66`, using `emit_to` scoped to the window label).

Plugins (`tauri-plugin-dialog`, `tauri-plugin-clipboard-manager`) are linked but have zero webview permissions; they are driven only from Rust, so the webview "asks for the operation, but never receives a native-picked source path or file contents" (`PERMISSIONS.md`, "No plugin permissions at all").

A subtle point documented in `PERMISSIONS.md`: "Whether a command is compiled in is not the security boundary." Cargo feature unification across the workspace could re-enable tray/devtools commands in a sibling build; only the capability file is a control, which is why it is asserted by test.

**Custom commands** (`generate_handler!`) are not ACL-gated in Tauri, so every value-bearing custom command starts with `applock::require_unlocked` and `require_main_window` (`src/commands/vault.rs:1157-1250`).

**References.** Tauri v2 Capabilities and Permissions docs (https://v2.tauri.app/security/capabilities/, https://v2.tauri.app/security/permissions/); Tauri "Runtime Authority" concept [VERIFY page name].

### B.3 Webview sandboxing: `withGlobalTauri:false`, CSP, exact-origin navigation allowlist, static source lint

**Problem.** A window that renders secrets must not be able to reach remote origins or publish its IPC bridge on a global object.

**How it works.**
- `tauri.conf.json` sets `withGlobalTauri: false`, so `window.__TAURI__` never exists; only bundled modules that statically import `@tauri-apps/api/core` can invoke. `apps/desktop/tests/react-boundary.test.ts` is a static-analysis test that fails the build if `__TAURI__` appears in first-party source, if any raw-HTML sink (`innerHTML=`, `dangerouslySetInnerHTML`, `document.write`, ...) is used, and if any file other than three named bridge transport files imports the Tauri API.
- CSP (`tauri.conf.json` `app.security.csp`): `default-src 'self'; script-src 'self'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; frame-ancestors 'none'`.
- `src/navigation.rs` registers a plugin whose `on_navigation` hook allows only whole origins compared exactly (scheme, host, port): `tauri://localhost`, `http(s)://tauri.localhost`, and in development builds only `http://127.0.0.1:1421`. `file:`, `data:`, `blob:`, `javascript:` and remote origins are refused. Refusals log only scheme/host/port, never path or query, because "a window that renders secrets can put a value in one" (`PERMISSIONS.md`, "Allowed window locations"). Development is detected with `tauri::is_dev()` (the `custom-protocol` feature), not `debug_assertions`, because that is the same switch Tauri uses to choose `devUrl` vs the embedded bundle.
- Hosted-admin windows (`host-admin-*` labels) get their own per-window origin binding tied to the app-lock generation and an expiry (`src/commands/web_admin.rs:17-48`).

**References.** MDN CSP reference; Tauri v2 CSP docs (https://v2.tauri.app/security/csp/); OWASP XSS Prevention Cheat Sheet.

### B.4 App lock: OS-delegated authentication and a generation counter gate

**Problem.** An unattended unlocked laptop; also, background work that started before a lock must not deliver plaintext after the unlock.

**How it works** (`apps/desktop/src-tauri/src/applock.rs`):
- `AppLock { locked: AtomicBool, generation: AtomicU64, authenticating: AtomicBool, capability }` (lines 29-34). It starts **locked** iff the platform can authenticate (`locked: AtomicBool::new(capability.available)`, line 45); a platform without an authenticator starts unlocked so the UI can never become permanently inaccessible.
- `lock()` sets `locked` and increments `generation` (lines 76-77). `unlocked_generation(app)` captures the current generation and checks unlocked (lines 97-108); `require_unlocked_generation(app, g)` fails with `chat-interrupted` if the generation moved (lines 109-118). Long operations (file pickers, multi-chunk transfers, chat sessions) bracket themselves with these two calls, so a lock/unlock pair in the middle invalidates the work even though the app is unlocked again. A unit test `locking_invalidates_work_even_after_unlock` pins this (line ~300).
- `lock_app` command (lines 145-159) performs the scrub: `invalidate_catalog()`, `web_admin::close_all`, `chat_local::conceal` (on macOS this clears delivered/pending `UNUserNotificationCenter` notifications, `src/commands/chat_local/platform/macos.rs`).
- `unlock_app` (lines 161-191) uses `authenticating.swap(true)` to block a second prompt, and runs the OS prompt on `spawn_blocking`.
- macOS (lines 200-262): `LAContext::canEvaluatePolicy` probes `DeviceOwnerAuthenticationWithBiometrics` to label the mechanism "biometry" vs "password", then evaluates `DeviceOwnerAuthentication` (Touch ID / Apple Watch / account password) with a 60 s timeout; LAError codes -2, -4, -9 (user cancel, system cancel, app cancel) are "not authenticated" rather than errors.
- Linux (`src/applock/polkit.rs`): connects to the system D-Bus, checks that action `com.aka.foks.desktop.unlock` is registered via `EnumerateActions` (else reports "policy not installed"), and calls `CheckAuthorization` with subject `unix-process {pid, start-time}` where start-time is field 22 of `/proc/<pid>/stat` (parsed as the 20th field after the closing paren, line 80-84). Including start-time defeats PID reuse. The policy file `apps/desktop/src-tauri/linux/com.aka.foks.desktop.policy` sets `allow_active=auth_self`, `allow_inactive=no`, `allow_any=no`, so only the active console user can authenticate, without root. The .deb installs it to `/usr/share/polkit-1/actions/` (checked in CI, `foks-desktop.yml` "Verify Debian contents").
- No master password exists anywhere; authentication is fully delegated to the OS (`applock.rs:5-6`, `docs/INTRO.html` section 4).

**References.** Apple LocalAuthentication `LAContext.evaluatePolicy` docs; polkit D-Bus API reference (`org.freedesktop.PolicyKit1.Authority.CheckAuthorization`, "unix-process" subject with `start-time`); `proc(5)` man page for `/proc/[pid]/stat`.

### B.5 Keeping secrets out of the JavaScript heap: one value-returning command, conceal-on-blur, `Zeroizing`

**Problem.** The webview must show a password on request without ever holding file bytes or more than one secret.

**How it works.**
- `read_item` (`src/commands/vault.rs:1157`) is the sole content-returning command; it returns exactly one item at one exact version (`selected_item(&store_id, &path, version)` binds the request to the cached catalog). `copy_item_value` (line 1175) performs the same read but hands the `Zeroizing<String>` straight to the clipboard module; `download_file` (line 1232) opens a native save dialog from Rust and streams chunks to disk. Neither returns content through IPC (`PERMISSIONS.md`, "Controls for value-returning commands").
- Every secret argument is deserialized straight into `Zeroizing<String>` (`src/commands/validation.rs:401-405` `deserialize_secret`) and never echoed in responses or logs.
- In the webview, `useConcealOnInactive` (`apps/desktop/src/use-conceal-on-inactive.ts`) runs a concealment callback once per inactive transition on `blur` or `visibilitychange`; a `useLayoutEffect` guard (lines 48-52) handles the race where a secret arrives after the blur that started the read, masking it before first paint. The details panel additionally keeps a `concealEpoch` ref (`screens/details-panel.tsx:322, 399-422`) so a late-arriving read is discarded if a conceal happened meanwhile. Concealment also clears the in-memory tab history and the metadata repository (`apps/desktop/README.md` lines 303, 1068).
- `MAXIMUM_DESKTOP_REVEAL_BYTES = 16 MiB` bounds what a reveal will fetch (`crates/foks-desktop/src/lib.rs:31`).

**References.** `zeroize` crate docs; OWASP "Sensitive data in memory" guidance [VERIFY]; Page Visibility API (MDN).

### B.6 Clipboard hygiene: digest-only tracking, timed clear, concealed pasteboard type

**Problem.** Clipboard managers archive plaintext; a naive auto-clear can clobber something the user copied later.

**How it works** (`apps/desktop/src-tauri/src/clipboard.rs`):
- `copy_with_hygiene` (lines 39-64) computes `Sha256(value)` and a monotonically increasing generation, writes the value to the pasteboard, then drops the `Zeroizing<String>`; only the 32-byte digest and generation are retained in a static `Mutex<Option<PendingClipboard>>`.
- A thread sleeps `AUTO_CLEAR_SECONDS` (15 s on Linux, 30 s elsewhere, lines 17-20) then `clear_if_unchanged` (lines 66-93): it re-hashes the current clipboard text and clears only if the digest still matches, so an unrelated later copy is never destroyed. The generation ensures a repeated copy of the same value gets its own timer token (test `repeated_copies_have_distinct_timer_tokens`).
- macOS (lines 128-163) writes via `NSPasteboard` and also sets `org.nspasteboard.ConcealedType`, the de-facto convention asking clipboard-history tools not to record the item. Linux has no equivalent, hence the shorter timeout (`PERMISSIONS.md`, "Clipboard platform difference").
- `defer_exit_cleanup` (lines 106-126) calls `api.prevent_exit()` and performs one best-effort clear before the process exits, because `RunEvent::Exit` fires after plugin cleanup.

**References.** nspasteboard.org "ConcealedType" convention [VERIFY URL]; `tauri-plugin-clipboard-manager` docs.

### B.7 Native drag-and-drop with one-shot path authorization and streamed upload

**Problem.** Web-style file input loads bytes into JS `ArrayBuffer`s. FOKS needs uploads without bytes in the renderer, but also must not turn a path-taking command into a general file reader.

**How it works.**
- `tauri.conf.json` enables `dragDropEnabled`, so the OS drop event lands in Rust (`src/dragdrop.rs`). `record_drop_paths` (`src/commands/context.rs:1829-1845`) accepts exactly one UTF-8 path as *authorized*; multi-file drops are reported for display but authorize nothing. `upload_path` (line 1879) returns `drop-not-authorized` for any path not in `pending_upload_paths`; `release_upload_path` consumes it after use. Native picker paths go through `record_picked_path` and never cross IPC at all.
- The Rust side rejects symlinks and non-regular files, then streams the file to the agent via `put_kv_stream` (`src/commands/execution.rs:109`, `vault.rs:793`). Streaming starts above `MAXIMUM_INLINE_KV_BYTES = 128 KiB` (`crates/foks-desktop/src/lib.rs:27`); the frame size is the agent protocol's `MAXIMUM_KV_PAYLOAD_BYTES` (700 KiB per `docs/DESKTOP_APP.html` section 6). Chunk buffers are `Zeroizing<Vec<u8>>`.
- Download (`vault.rs:395-545`): native save dialog (Rust), `NamedTempFile::new_in(parent)`, per-chunk validation that store/path/version/offset match and the running total stays under `MAXIMUM_DOWNLOAD_BYTES`, each chunk zeroized after write, `sync_all`, then `persist(destination)` which is an atomic rename. The final length is checked against the authenticated size (`response-binding` error if not).
- Note: `docs/DESKTOP_APP.html` says `MAXIMUM_DOWNLOAD_BYTES = 512 MB`, but `src/commands/validation.rs:422` defines it as `1024 * 1024 * 1024` (1 GiB). The code is authoritative.

`crates/foks-desktop/tests/backend_transcript.rs` proves an 84 MiB sparse file is carried only as bounded stream frames.

**References.** Tauri v2 `dragDropEnabled` window option; POSIX `rename(2)` atomicity; `tempfile::NamedTempFile::persist`.

### B.8 Virtualized lists (windowing without a wrapping grid)

**Problem.** Keeping a long catalog responsive.

**How it works.** `apps/desktop/kit/virtual-list.ts` is a pure function `virtualListWindow({heights, listTop, scrollTop, viewport, overscan}) -> {start, end, padTop, padBottom}`. It prefix-sums row heights (measured or estimated), clamps `scrollTop` to `total - viewport` to avoid an empty flash after filtering (lines 61-68), finds the first row not fully scrolled out and the last row starting before the viewport bottom, expands by `overscan`, and returns spacer heights for unmounted rows. It is DOM-free so the arithmetic is unit-tested. Update (2026-10-01): the card grid uses `apps/desktop/src/screens/files-grid.ts` to window complete rows at the viewport width; the earlier 200-card cap is gone.

**References.** react-window / TanStack Virtual documentation for the windowing concept.

### B.9 Navigation state machine, guard engine, and deep links

**How it works.** `Location` is a discriminated union encoded to `?state=...&store=<StoreRef>` and mirrored with `history.replaceState` (no browser stack, no native swipe). Guards registered with `useNavigationGuard` return `null | prompt | refuse`; all guards are evaluated, any `refuse` halts, and the first `prompt` is shown only after every guard has run so "a confirmed prompt can never bypass another guard's refusal" (`apps/desktop/README.md:900-935`; `src/navigation/location-store.ts`). A custom trackpad swipe-back tracker requires 120 px travel and a 120 ms idle gap (`docs/DESKTOP_APP.html` section 9). Deep links require `?store=<StoreRef>` (a JSON string with profile, alias, team id) and reject bare `?account=` to prevent alias-collision attacks across servers (section 12).

### B.10 State coordination: CatalogCoordinator, QueryRepository, generation tokens

**How it works.** `CatalogCoordinator` (`src/app/catalog-runtime.ts`) coalesces concurrent snapshot loads into one promise, marks `dirty` on mutation during a load and schedules one trailing refresh, and publishes only the newest accepted result. `QueryRepository` / `MetadataRepository` (`src/metadata-repository.ts`) cache device/group metadata for 60 s, key it by profile and store, share one in-flight read per resource generation, and invalidate by advancing a generation so late reads cannot refill a cleared resource. Nothing secret is cached and nothing persists to disk (`apps/desktop/README.md` "Device metadata cache", "State coordination"). A failed refresh after a confirmed write is reported as a stale view, never as a failed write; ambiguous outcomes are never auto-replayed.

### B.11 Profile work scheduling: per-profile lanes, bounded queue, timings without values

**Problem.** The agent serializes requests per profile; the UI has many independent callers.

**How it works** (`apps/desktop/src/scheduling/profile-work.ts`): a `WeakMap<owner, Map<profile, Queue>>` where each queue has two lanes (`default`, `chat`), each with foreground and background FIFOs and one active job. Chat gets its own lane because the agent already admits one inbox poll per account, so waiting behind a catalog walk "buys nothing" (lines 42-48). `MAX_QUEUED = 256`; `MAX_QUEUE_WAIT = 60 s + 15 s` derived from the agent's request timeout. Background history work carries `preemptible: false` because "Native cancellation currently has no agent-lock-release acknowledgment" (line 25). Observers get `WorkTiming{profile, key?, priority, queueMilliseconds, executionMilliseconds, outcome, busy}`; "never request values or results" (line 89).

### B.12 Notification consumer: bounded rotation over channels with the "active thread invariant"

**Problem.** Discover new messages across many channels (benchmarked at 70 and 200) without flooding the agent, while the open conversation must never feel stale.

**How it works** (`apps/desktop/src/chat/notification-consumer.ts`):
- Limits: `NOTIFICATION_LIMITS = { queued: 64, baselines: 4096 }` (line 22); per-pass budget `PASS_ROWS = 400`, `PASS_BYTES = 4 MiB`; per-channel `CHANNEL_ROWS = 100`, `CHANNEL_BYTES = 1 MiB` (lines 53-56).
- `scan()` (lines 218-446) walks the immutable inbox snapshot in two segments starting at `this.cursor % size` (lines 283-360), so the walk is a **round-robin rotation** across all channels of all teams that resumes where the last pass stopped rather than always restarting at zero; it allocates no channel array. When `progress.size >= baselines` it evicts an idle baseline (`evictionCandidate`, lines 175-179) but caps evictions per 5 s window at `queued` (lines 316-335) so rotation "must not become an endless baseline RPC loop."
- Admission: before starting a job it reserves a whole channel read against the pass budget (`rows + (jobs+1)*CHANNEL_ROWS > PASS_ROWS`, lines 371-377) and acquires a per-profile admission permit (line 378); a budget hit re-arms after a 1 ms yield with leftovers first in Map order (lines 392-409).
- Timers: the consumer arms for the earliest computed deadline, or no timer at all if nothing is due; wake-ups otherwise come from inbox publications or admission releases (lines 419-445).
- Fallback backoff: a channel whose pass found nothing doubles its interval from `FALLBACK_MS = 5000` up to `FALLBACK_MAX_MS = 60_000`; a pass that observed a newer sequence resets to 5 s (`commit`, lines 492-508). **The channel whose thread is open never backs off** because `fallbackDue` uses `FALLBACK_MS` whenever `presented(p)` (lines 203-217). This is the "active thread invariant" the docs name (`docs/GUIDE.html` section 7).
- Baselines are seeded from the inbox projection's own head when the projection is not degraded, costing no RPC (lines 512-534). Replies are integrity-checked (`sameScope`, max 50 rows, max 256 chars per notification text, lines 469-479).
- The scale test `scale: 4096 baseline bound rotates with a bounded eviction rate` (`apps/desktop/tests/notification-scheduling.test.ts:719-722`, body at 659-712) builds 17 stores totalling 4097 channels and asserts exactly `queued` evictions, never more than `baselines` progress records, no content passes during pure rotation, and that the rotation resumes after 5 s. It runs only with `FOKS_TEST_SCALE=1` (`npm run test:foks-ui:scale`).

### B.13 Chat history paging by cursor, incremental tail, and bounded cache

**How it works.**
- The wire shape (`apps/desktop/src/chat-contract.ts:165-171`): a `history` result carries `messages`, `before: string | null` (the opaque cursor to fetch older rows), `missing_predecessors`, and optional `gap`. The inbox result carries `cursor`, `head` and `degraded`, and the decoder enforces `cursor <= head` and `degraded == (cursor !== head)` (lines 512-515).
- `useChatHistory` (`src/chat/use-chat-history.ts`): "Load older messages" issues `{action:'history', channel, before: older}`; an incremental refresh issues `after: <newest held sequence>` and, if the reply reports `gap !== false`, falls back to a full first page with `replace = true` (lines 117-133). A published-head gate skips the tail read when the inbox says nothing newer exists (lines 91-99), so revisions caused by non-arrival reasons cost no round trip.
- `ChatHistoryCache` (`src/chat/history-cache.ts`) is an LRU keyed by a `HistoryBinding{store, channel, scope, generation, authority}` with limits `{channels: 16, rows: CHAT_HISTORY_ROWS = 1000, bytes: CHAT_HISTORY_BYTES = 8 MiB}`; `get()` re-inserts to refresh recency (lines 71-78). A binding whose authority or generation changed is simply not current, so stale pages cannot be shown after a role change.
- Limits are shared with Rust through `crates/foks-agent-proto/chat-limits.json` (`CHAT_PAGE_ROWS: 50`, `CHAT_HISTORY_ROWS: 1000`, `CHAT_POLL_MILLISECONDS: 55000`, ...), imported directly by `apps/desktop/src/chat-limits.ts` and asserted equal in `crates/foks-agent/src/chat.rs`. The Rust agent's incremental-history test runs at page size 3 in the ordinary suite and at the production `CHAT_PAGE_ROWS` under `npm run test:rust:scale` (`crates/foks-agent/src/chat.rs:997-1006`, `scripts/test-rust-scale.sh`), covering "the same cursor boundaries quickly" (`README.md`).

### B.14 Durable operation receipts and submission IDs (resumability)

**How it works.** First-run account creation records an on-disk receipt (`account-operation-receipts-v1`) with `outcome: Unknown` before network dispatch, fsyncing file and parent directory, so a crash mid-mutation yields an explicit "Resume setup" rather than a duplicate account (`docs/DESKTOP_APP.html` section 11; `docs/INTRO.html` section 2). Chat channel creation and sends use durable submission IDs and a pending ledger; the UI polls the ledger rather than re-sending (`src/chat/send-service.ts`, `src/chat/channel-creation.ts`). Every mutation is compare-and-swap guarded by the catalog's exact version; ambiguous post-commit failures trigger a catalog refresh, never an automatic retry (`crates/foks-desktop/README.md`).

### B.15 Close guard: one-way teardown only after verified agent exit

`apps/desktop/src-tauri/src/close_guard.rs` models exit as a state machine (`Idle -> Decision{pid, unsent} -> Stopping -> Finalizing | Failed | ForceConfirmation`) and only lets the desktop exit once the agent's termination is verified; the last commit on the branch ("Wait for verified agent termination before desktop exit") landed this.

### B.16 Mock mode and the bridge boundary

`VITE_FOKS_MOCK=1` selects `src/mock-bridge.ts` + `src/fixture.ts` (no network, no timers) via a dynamic `import()` only for an explicit mock build or a non-native Vite dev page; a Tauri dev window still uses real commands, and "an ordinary production build outside Tauri fails closed" with the mock tree-shaken out (`apps/desktop/README.md:182-215`). `app-root.tsx` calls `selectBridge`, checks the Rust app lock, and calls `loadSnapshot` only after an armed lock has been authenticated. The Rust side pins response serialization with `wire-contract.json` (assembled from per-domain JSON in `src-tauri/wire-contract/` by `assemble.mjs`, which rejects duplicate keys) and `src/commands/tests/contract.rs`; `tests/bridge.test.ts` pins the decoder side, so drift fails on one side or the other.

### B.17 Bounded diagnostics with cursors

`src-tauri/src/diagnostics.rs:57-130`: a `TimingLog` ring buffer (`VecDeque`, `CAPACITY`) with a monotonically increasing sequence; `since(cursor)` returns events at or after the cursor plus `next`, so the frontend pages the native timing log incrementally. Agent loop timers are deduplicated by start timestamp. Text fields are truncated and the log never carries values.

---

## Part C. MCP server (`crates/foks-mcp`)

### C.1 What it exposes and how it maps to agent operations

**Problem.** Let an LLM agent read and write an encrypted vault without giving it credentials, database access or a shell.

**How it works.**
- Entry: `foks-rs --state-dir DIR mcp kv|team --profile P --account-alias A [--read-only]` (`crates/foks-cli/src/mcp.rs:10-49`). `ensure_agent` pings the socket, and if needed spawns the sibling `foks-agent` executable found next to `foks-rs` (`current_exe().with_file_name("foks-agent")`), "never a shell command or an executable named by a tool call" (line 58).
- Two tool sets (`contract.rs:165-183`): Team = `list`, `list-memberships`; KV read-only = `list, get, stat, usage`; KV normal adds `put, mkdir, rm, mv, foks_status, foks_pending`. The ten upstream names match go-foks v0.1.9's MCP (`crates/foks-mcp/README.md`); `foks_status`/`foks_pending` are local recovery additions. (Note: `docs/GUIDE.html` section 10 calls these `fennec_submission_id`/`fennec_status` and places `usage` under the Team set; the code says `submission_id`, `foks_status`, and `usage` is a KV tool. The code is authoritative.)
- Arguments deserialize with `deny_unknown_fields`; `PutArgs` zeroizes `content` on drop and the `Invocation` enum deliberately does not derive `Debug` "the put content can contain credentials" (`contract.rs:22`). Serde errors are never echoed because they can quote input (line 199).
- `AgentBackend` (`agent.rs`) binds one profile/account at connect and verifies the agent returned exactly that scope (lines 30-43); a `team` argument is resolved through `DataRead::ResolveTeam` and re-checked to be the same profile/account/host/user (lines 45-69). Reads are `Operation::ReadData`; writes go through `PrepareDataWrite` then either `put_kv_stream_cancellable` or a commit operation (`agent/writes.rs`). Path resolution follows symlinks inside the encrypted namespace with a 32-hop limit and cycle detection (lines 108-160).
- Writes carry a `SubmissionHandle` `v1-<16 hex unix secs>-<32 random hex>` (128 random bits); the client may supply one to keep intent across restarts, and reuse with different inputs is rejected. Structured results carry `submission_id`, `status`, `partial`, `node_id`. Recovery semantics (prepared / committed / rejected / unknown) and the rule "never repeat a mutation with a new ID after an unknown outcome" are spelled out in `crates/foks-mcp/README.md` ("Recovering a write").

### C.2 Authentication and scoping

There is no MCP-level auth. The trust model is: the MCP process runs as the local user, connects to the user's own `foks-rs.sock` (0600, uid-checked by the agent client), and the account "must already be usable by the agent; MCP never asks for passwords or hardware PINs on stdin" (`README.md`). Scope is fixed at process start to one profile + account + host; a tool "can select an accessible team, not another account or state directory."

### C.3 Bounds and backpressure

`contract.rs:9-14`: `MAX_INPUT_BYTES = MAX_OUTPUT_BYTES = 8 MiB`, `MAX_FILE_BYTES = 4 MiB`, `PAGE_ROWS = 1000`, `ACTIVE_CALLS = 4`, `QUEUED_CALLS = 16`. `session.rs` enforces `ACTIVE_CALLS` with a `Semaphore`, runs each backend call on `spawn_blocking`, links request cancellation to a child `CancellationToken`, and rejects results over `MAX_OUTPUT_BYTES - 1024` (lines 109-145). `transport.rs` wraps stdio with a max-length JSON-RPC codec, checks the *encoded* output size ("JSON escaping can expand sixfold", line 74), keeps a per-request-ID `Publishing` marker so ID reuse waits for flush, and applies a 30 s stall timeout so a blocked stdout cannot wedge the process (lines 66-115).

### C.4 Protocol versions and compat

`session.rs:44-64`: advertises MCP `2025-11-25` and negotiates `2025-06-18`, `2025-03-26`, `2024-11-05` through the pinned `rmcp = "=3.2.0"` SDK. `tools/foks-v019-oracle/run-mcp-compat.sh` runs the same stdio scenarios through an independent Go MCP SDK against both Rust and pinned Go services; the Go SDK unmarshals `stat` into upstream `lcl.KVStat`.

**References.** MCP specification (https://modelcontextprotocol.io/specification/2025-11-25 [VERIFY exact revision URL]); `rmcp` crate (official Rust SDK, https://github.com/modelcontextprotocol/rust-sdk); JSON-RPC 2.0 spec.

---

## Part D. Engineering practices

### D.1 Benchmark methodology (chat notification acceptance)

Files: `scripts/benchmarks/README.md`, `run-chat-notifications.py`, `chat-notifications.ts`, `summarize-chat-notifications.py`, `test_summary.py`, worker `crates/foks-server-testkit/examples/chat_notification_bench.rs`.

- **Real processes.** Worker uses `TestEnvironment::ProductionBenchmark`: a real isolated Rust server, a separate real agent process, private temporary client state, no OS keyring. The Node side runs the *production* TypeScript services (`ChatInboxService`, `NotificationConsumer`, `scheduleProfileWork`) against them, so the measurement covers service + agent IPC + server but excludes rendering.
- **Workloads.** `steady` (70 channels), `backlog` (70 channels + 120 sequentially confirmed sends injected at measurement start), `wide` (200 channels), `slow` (70 channels, 250 ms added after admission, one permanently failing channel). Arms: notifications `off` / `on`. Rates are explicit inputs: foreground history read every 500 ms, foreground write workflow every 2 s, a second account sending 6 incoming messages/s.
- **Phases.** Each trial: baselines established, then **15 s warmup, 60 s measurement, 10 s drain** (`chat-notifications.ts` defaults; `summarize` rejects anything else as "not an acceptance duration"). Three repetitions per arm, arm order alternated per repetition (`run-chat-notifications.py` loop), results appended as JSONL with `fsync` after each trial and refusal to overwrite.
- **Metric.** `foregroundHistory.p95` per trial (successful explicit history operations, submission to completion). Reported value = **median of the three trial p95s** per arm (`summarize`: `median(t['foregroundHistory']['p95'] ...)`). Review trigger: on-vs-off p95 increase exceeding **both 20% and 50 ms**. Discovery target: >= 99% of confirmed sends discovered by the drain deadline; `lateDisplays` must be 0; `foregroundSlots` must equal 120 with no missed slots; concurrency bounds `peakJobs <= 2`, `peakPerProfileJobs <= 1`, `peakBackgroundRPCs <= 1`.
- **Provenance.** The driver SHA-256 hashes the agent, worker, harness, every file under `apps/desktop/src`, `chat-limits.json`, and records source revision and patch digest in each trial. Event-loop delay p95 is sampled with `monitorEventLoopDelay` at 20 ms. Baseline mode extracts TypeScript from commit `6031904` and shares native binaries so only notification behavior differs.
- **Published numbers** (`README.md`): 43.7 ms steady/70, 38.4 ms backlog/70, 45.1 ms steady/200 on Linux x86_64.
- A second, CPU-only benchmark (`inbox-publication.ts`) measures publication at 10/100/1000 channels as the median of five batch means of 100 ops after 20 warmups. Server-side expiry benchmarks are ignored Rust tests with recorded results in `scripts/benchmarks/results/server-expiry-2026-09-20.json`.

**References.** Standard latency-percentile practice (e.g. Gil Tene, "How NOT to Measure Latency") [VERIFY]; Brendan Gregg's benchmarking checklist [VERIFY].

### D.2 CI structure (`.github/workflows`)

- `foks-standalone.yml`: on push to Rust/tools paths, daily cron, and dispatch. `client` job on ubuntu + macos-14 runs `tools/foks-client/check.sh` and ignored keystore tests against real macOS Keychain / GNOME Keyring under `dbus-run-session`. `server-fast` runs `generate-protocol.sh --check --offline`, `go test` on the oracle's offline fixture guards, and `tools/foks-server/check.sh` (which uses `cargo metadata` + `jq` to assert every `foks-*` package is defined in and reaches only FOKS-owned directories, a Cargo-graph boundary check). `server-full` (nightly) adds the live Docker/Postgres 17 gates: `run-live-compat.sh`, `run-sso-compat.sh`, `run-mcp-compat.sh`, `run-live-team-compat.sh`, `run-go-client-compat.sh`, `run-host-rotation-compat.sh`, plus release client/server lifecycle and small-team restart/capacity/backup tests.
- `foks-desktop.yml`: PR validation on macos-15 and ubuntu-24.04 (typecheck, eslint, `test:foks-ui` incl. scale, `test:rust:full --release`, clippy `-D warnings`, fmt, unsigned DMG assembly, packaged-startup smoke test that launches the bundled binary headless with `--smoke-test-packaged-startup`, bundle layout checks, and the real managed-agent desktop transcript). Tag `foks-desktop-v<version>` triggers signed builds; the tag must equal the Cargo workspace version.
- `foks-protocol-drift.yml`: weekly; `foks-protocol-sync` (a Go `go/ast` extractor that never executes upstream code) diffs the pinned v0.1.9 metadata against upstream mainline HEAD, classifies drift (wire-breaking / behavior-review / additive / outside slice / source-only), and opens or updates a `[protocol-drift]` issue.
- `foks-hosted-compat.yml`: daily canary against the real `foks.app:4430` host with a dedicated disposable account; builds tools, verifies their SHA256SUMS in a second job, runs a KV+onboarding lifecycle, signs a "capability lease" artifact with `foks-compat-artifact`, attests provenance, and publishes it at a stable release tag with compare-and-swap on the previous asset digest ("stable canary state changed after generation allocation"). A failed compatibility run still publishes a fail-closed revocation.
- All action versions are pinned by commit SHA.

### D.3 Differential / compatibility testing strategy

- **Fixture oracle** (`tools/foks-v019-oracle/README.md`): the unmodified Go v0.1.9 code captures the live probe, verifies host chain and Merkle root, and emits canonical Snowpack fixtures (`crates/foks-snowpack/tests/fixtures/foks-v0.1.9/{foks.app,user,signup,user-mutations,realtime,yubi}`). Rust tests decode, re-encode, and assert byte equality. Deterministic seeds make the generator produce byte-identical corpora twice. Known upstream quirks are preserved as fixtures rather than corrected: `TeamCreator` founding seqno 0, and the pinned Go client's reversed verifier order for rotated invitation certificates.
- **Live fleet**: Docker + Postgres 17 testcontainers run the unmodified Go integration environment; both directions are tested (Rust client -> Go server; Go client -> Rust server) plus chat, SSO, MCP, invitations and host-key rotation gates.
- **Hosted canary** as above, plus `foks-go-interop` for reading legacy Go client state.
- Chat limits are a single JSON source consumed by both Rust and TypeScript, with a Rust assertion that they agree.

**References.** Differential testing (McKeeman 1998, "Differential Testing for Software") [VERIFY]; testcontainers-go docs.

### D.4 Release signing and notarization

- `packaging/foks-desktop/RELEASE_POLICY.md`: fail-closed; unsigned macOS artifacts must never be published; checksums and GitHub build-provenance attestations for both platforms; placeholder icons flagged.
- `scripts/sign-macos-app.sh`: signs the helper `Contents/MacOS/foks-agent` first, then the outer app, both with `--options runtime --timestamp` (hardened runtime, secure timestamp) and an empty entitlements plist (`packaging/foks-desktop/macos/entitlements.plist`); accepts an unset identity only if exactly one Developer ID Application identity is installed; verifies `--deep --strict`.
- `scripts/notarize-macos-app.sh`: `ditto` app into a staging dir with an `/Applications` symlink, `hdiutil create` (UDZO), sign the DMG, `notarytool submit --wait`, `stapler staple`, `stapler validate`, and a Gatekeeper `spctl --assess` check.
- `scripts/build-macos-release.mjs` loads `.env` with Node's `process.loadEnvFile` (no shell expansion) without overriding already-exported values, then `build-macos-release.sh` validates all inputs before building anything; output lands under `target/release/foks-release/` only after validation.
- CI imports the `.p12` into a throwaway keychain and asserts exactly one identity (`foks-desktop.yml` "Import signing identity").
- Linux: `.deb` must contain `foks-desktop`, `foks-agent`, the polkit policy, desktop entry, AppStream metadata; validated with `desktop-file-validate` and `appstreamcli validate`.

**References.** Apple "Notarizing macOS software before distribution" and "Hardened Runtime" docs; GitHub `actions/attest-build-provenance` / SLSA provenance.

### D.5 Repository conventions

`AGENTS.md`: commit style (short description + paragraphs via multiple `-m`), Linux build deps (`libgtk-3-dev`, `libpcsclite-dev`, `libwebkit2gtk-4.1-dev`), the requirement to stage a `foks-agent` binary before the Tauri build script will run, serial execution of native desktop unit tests, and a note on forcing a relink when a stale standalone agent reports an IPC version mismatch. `CLAUDE.md` asks for technically precise prose without anthropomorphism. Workspace clippy lints warn on `todo!`, `unimplemented!`, `dbg!`. `ISSUES.md` is a prioritized, evidence-linked inventory of known gaps (chat retention quotas, terminal-row compaction, invitation lifecycle, realtime fanout cap of 1,024) and disclosed limitations (no Windows, no message deletion/reactions, 10,000-anchor equivocation window).

---

## Part E. Discrepancies and cautions for the book

1. `docs/GUIDE.html` section 10 uses `fennec_submission_id`, `fennec_status`, `fennec_pending` and lists `usage` under the Team toolset; code uses `submission_id`, `foks_status`, `foks_pending`, and `usage` is a KV tool (`crates/foks-mcp/src/contract.rs:32,44-47,165-183`).
2. `docs/DESKTOP_APP.html` says `MAXIMUM_DOWNLOAD_BYTES = 512 MB`; code says 1 GiB (`apps/desktop/src-tauri/src/commands/validation.rs:422`).
3. `docs/GUIDE.html` says the desktop is "React 18"; `package.json` pins React 19.
4. `docs/DESKTOP_APP.html` mentions `foks://agent-connection-loss` as an emitted event; the README says the shell *polls* `take_agent_connection_loss` (`apps/desktop/README.md` Phase 3 paragraph). Verify before quoting.
5. The `docs/*.html` files are polished explainers, likely generated; prefer the READMEs (`apps/desktop/README.md`, `src-tauri/PERMISSIONS.md`, `crates/foks-mcp/README.md`, `scripts/benchmarks/README.md`) and code for numbers.
6. `apps/desktop/README.md` links to `docs/state-consistency.md`, which does not exist in the checkout.
