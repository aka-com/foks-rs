# Tauri native shell and desktop crate

Area key `desktop-native`. 15 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The Tauri shell (apps/desktop/src-tauri) is security-conscious in its core boundary. The renderer capability is three core permissions, and tests resolve them through the generated ACL manifests. Navigation goes through an exact-origin allowlist. Plaintext stays in Rust: clipboard writes, the native picker and dropped-file uploads never pass content through IPC. The app lock uses a generation counter. All 127 custom commands register in one flat generate_handler! list in lib.rs, and 125 of them check the main-window label. CI builds an unsigned and a notarized arm64 DMG and an x86_64 Debian package, and runs a packaged-WebView smoke test. Windows is not built: build-desktop.mjs throws on win32 and foks-agent exits on non-Unix. The gaps cluster in six areas. (1) Security behaviour does not match the documentation: locking is manual-only although the book describes an inactivity lock, the startup failure path offers a destructive state reset after a fixed 5-second wait, and a remote-origin admin window now shares the un-ACL'd command surface. (2) Logic is duplicated: DesktopModel is dead code, and the scripted backend and the Tauri commands build agent Operations separately and already disagree. 44 desktop structs mirror agent responses, and error codes are free-form strings. (3) The TypeScript and Rust contracts are kept in step by hand. Command names, argument keys and error codes are string literals; they match today, but no test checks them. (4) Platform coverage is uneven: Linux has no notifications, no clipboard-history hint and no name defaults, and only one architecture is built per OS. (5) There is no update channel. (6) context.rs (2458 lines) has grown into a catch-all. Several documents describe behaviour that has since changed.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [App lock is manual-only; add a native inactivity, sleep and screen-lock trigger](#desktop-native-auto-lock) | missing-feature | high | M |
| [Put the 127 custom commands behind Tauri's app ACL now that a remote-origin window exists](#desktop-native-app-acl-for-commands) | security | medium | M |
| [Split commands/context.rs and give store identities a typed key](#desktop-native-context-split) | maintainability | medium | M |
| [Replace free-form error-code strings with a registry shared by Rust and TypeScript](#desktop-native-error-code-registry) | code-quality | medium | M |
| [Linux chat notifications via org.freedesktop.Notifications, plus channel grouping and an unread badge](#desktop-native-linux-notifications-and-badges) | missing-feature | medium | M |
| [Agent Operations are built in three places; DesktopModel is dead and the real-agent CI test bypasses the shipping code](#desktop-native-operation-builders-triplicated) | maintainability | medium | L |
| [Widen the release matrix (universal macOS, rpm, aarch64 Linux) and record Windows as an explicit non-goal](#desktop-native-release-matrix) | missing-feature | medium | M |
| [A startup timeout can lead to a destructive state reset; add retry, failure classification and quarantine](#desktop-native-startup-reset-safety) | correctness-risk | medium | M |
| [Stop mirroring agent response types in the desktop; decode into foks-agent-proto types](#desktop-native-typed-agent-responses) | maintainability | medium | L |
| [No update channel: add signed in-app updates for macOS and an update notice for the .deb](#desktop-native-updater) | missing-feature | medium | L |
| [agent.log grows without bound and crash markers are never read](#desktop-native-agent-log-and-crash-markers) | code-quality | low | S |
| [Check command names and argument keys between generate_handler! and the TS bridge](#desktop-native-command-contract-check) | testing | low | S |
| [Correct specific stale statements in the desktop's security and architecture docs](#desktop-native-doc-drift) | docs | low | S |
| [Small Linux parity gaps: device and user name defaults, clipboard-history hint, link opening](#desktop-native-linux-parity) | feature-refinement | low | S |
| [Quit and window close always show a 'stop agent' modal, even when nothing is pending](#desktop-native-quit-confirmation) | feature-refinement | low | S |

### desktop-native-auto-lock

**App lock is manual-only; add a native inactivity, sleep and screen-lock trigger**

- Type: missing-feature
- Priority: high
- Effort: M
- Layers: desktop-native, desktop-ui, docs
- Verification: adjusted

The app lock arms once at process start and afterwards locks only when the renderer calls lock_app (from the shell's lock action). Nothing in Rust or TypeScript locks on inactivity, system sleep, screen lock or window hide. The book and the HTML intro both describe an inactivity lock that does not exist. A vault left open on an unattended machine therefore stays unlocked indefinitely. The renderer also has no handler for an 'app-locked' error, because today only the renderer can cause a lock.

**Evidence**

- [`apps/desktop/src-tauri/src/applock.rs:66`](../../../apps/desktop/src-tauri/src/applock.rs#L66): AppLock::lock() is the only arming path after startup. Its only caller is the lock_app command at line 146.
- [`apps/desktop/src/app/app-bootstrap.ts:475`](../../../apps/desktop/src/app/app-bootstrap.ts#L475): lockNow is the renderer trigger. Its only callers are the menu (vault-shell.tsx:460 lockFromMenu) and the Settings button (settings-screen.tsx:438). There is no timer or visibility caller.
- [`apps/desktop/src/use-conceal-on-inactive.ts:9`](../../../apps/desktop/src/use-conceal-on-inactive.ts#L9): Existing partial mitigation: it hides revealed secret values on blur or visibilitychange but does not lock the app
- [`book/20-desktop.qmd:92`](../../../book/20-desktop.qmd#L92): States that the app lock 'hides everything after a period of inactivity or on request'. The glossary (appendix-b-glossary.qmd:15) repeats this.
- [`docs/INTRO.html:364`](../../../docs/INTRO.html#L364): Describes an 'Auto-Lock / Timeout' state triggered by an inactivity timer or by closing the lid
- [`apps/desktop/src/bridge/errors.ts:34`](../../../apps/desktop/src/bridge/errors.ts#L34): commandRecovery has no case for 'app-locked', so a lock started natively would fall through to 'refresh'

**Recommendation**

Add src/applock/triggers.rs and run the policy in Rust, since the renderer is treated as untrusted. (1) Idle: read OS idle time on a 30 s tick. On macOS use CGEventSourceSecondsSinceLastEventType. On Linux use org.freedesktop.ScreenSaver.GetSessionIdleTime, falling back to the logind Session IdleHint over the existing zbus dependency. (2) Events: on macOS, observe NSWorkspace willSleep and screensDidSleep and the com.apple.screenIsLocked distributed notification. On Linux, observe logind Manager.PrepareForSleep(true) and Session.Lock or LockedHint. (3) Optionally lock when the main window is hidden or minimized, using WindowEvent. Every trigger calls one shared lock_now(app) that performs exactly what lock_app does today (AppLock::lock, invalidate_catalog, web_admin::close_all, chat_local::conceal), then emits foks://app-locked with a reason to the main window. core:event:allow-listen already permits the renderer to receive it. Store the policy (timeout minutes, on_sleep, on_screen_lock, on_hide) device-locally next to chat-local/notifications.json. Add commands get_lock_policy and set_lock_policy; changing the policy requires require_unlocked. In errors.ts, route both 'app-locked' and the event to the existing lock screen. Add a unit test that drives the trigger through a fake clock and checks that the generation advances.

**Mockup:** [Auto-lock and clipboard policy](../mockups/security-auto-lock.html)

<details><summary>Verifier note</summary>

The core claim holds. AppLock::lock() (applock.rs:66) is reached only from the lock_app command (applock.rs:146-159). The renderer calls lockNow (app-bootstrap.ts:475) only from the sidebar menu (vault-shell.tsx:460 lockFromMenu) and from the Settings lock button (settings-screen.tsx:438). Searches for idle, inactivity, sleep and screen-lock triggers in TS and Rust found nothing. The Rust WindowEvent handlers cover only drag-drop, window-state, close and admin-window destroy. book/20-desktop.qmd:92, appendix-b-glossary.qmd:15 and docs/INTRO.html:364-365 all describe an inactivity lock that does not exist. errors.ts commandRecovery has no 'app-locked' case. ISSUES.md does not track this. The capability default.json already grants core:event:allow-listen, and chat-local/notifications.json exists, so the recommendation is feasible. Two corrections. First, the line citations are slightly off: lock() is at 66 and lock_app at 146; line 142 is inside app_lock_state. Second, a partial renderer-side mitigation exists and should be acknowledged. useConcealOnInactive (use-conceal-on-inactive.ts) hides revealed secret values on blur or hide in the details panel, first-run, device sheets and write workflows. It does not arm the app lock.

</details>

### desktop-native-app-acl-for-commands

**Put the 127 custom commands behind Tauri's app ACL now that a remote-origin window exists**

- Type: security
- Priority: medium
- Effort: M
- Layers: desktop-native, docs
- Verification: confirmed

build.rs calls plain tauri_build::build() with no AppManifest, so the generate_handler! commands have no ACL check. Each command enforces the boundary itself through require_main_window. open_web_admin now creates host-admin-* WebviewWindows that load an external HTTPS origin, so remote content runs in a webview that shares that command surface. get_window_state and set_traffic_lights_visible do not call require_main_window. A remote admin page could therefore hide the main window's traffic-light buttons (TRAFFIC_VISIBLE is global and is re-applied when the main window gains focus). PERMISSIONS.md still says no additional windows exist.

**Evidence**

- [`apps/desktop/src-tauri/build.rs:2`](../../../apps/desktop/src-tauri/build.rs#L2): tauri_build::build() without Attributes::app_manifest, so app commands have no ACL entries
- [`apps/desktop/src-tauri/src/commands/web_admin.rs:189`](../../../apps/desktop/src-tauri/src/commands/web_admin.rs#L189): WebviewWindowBuilder::new(.., WebviewUrl::External(url)) opens a remote-origin webview labelled host-admin-<nonce>
- [`apps/desktop/src-tauri/src/window_state.rs:83`](../../../apps/desktop/src-tauri/src/window_state.rs#L83): set_traffic_lights_visible (and get_window_state at line 20) have no require_main_window and return Result<_, String>
- [`apps/desktop/src-tauri/PERMISSIONS.md:89`](../../../apps/desktop/src-tauri/PERMISSIONS.md#L89): 'generate_handler! commands are not ACL-gated', and line 172 'New windows are blocked … no additional windows are configured'
- [`apps/desktop/src-tauri/src/navigation.rs:71`](../../../apps/desktop/src-tauri/src/navigation.rs#L71): The navigation policy already special-cases host-admin- labels, which confirms a second window class

**Recommendation**

In build.rs, call tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(&[...127 names...]))). Grant the generated allow-<command> permissions in capabilities/default.json for windows:["main"] only, and add no capability matching host-admin-*. Extend the existing resolved-capability tests in lib.rs to assert that (a) the main capability resolves to exactly the generate_handler! set and (b) no capability resolves for a host-admin-* label or a remote URL. Add require_main_window to both window_state commands and switch them to AgentError. Keep the per-command label check as a second layer. The AppManifest list can also serve as the single command inventory for desktop-native-command-contract-check.

<details><summary>Verifier note</summary>

build.rs is plain tauri_build::build(). generate_handler! lists exactly 127 commands. web_admin.rs:189 opens a host-admin-<nonce> WebviewWindow on WebviewUrl::External, and navigation.rs:71 special-cases that label. A scripted check of all 127 command bodies found require_main_window in every one except get_window_state (window_state.rs:20) and set_traffic_lights_visible (window_state.rs:83); both return Result<_, String>. set_traffic_lights_visible stores the global TRAFFIC_VISIBLE, which observe() re-applies to the main window on Focused(true). The web_admin module doc ('no main-window IPC authority') shows the design already relies on per-command label checks against host-admin windows, because Tauri 2 skips the ACL for app commands when there is no app manifest. PERMISSIONS.md:89 says custom commands are not ACL-gated. PERMISSIONS.md:172 still says no additional windows are configured, and the file never mentions host-admin windows. Existing capability tests (lib.rs:456, 513) can be extended as proposed. AppManifest::commands and try_build exist in tauri-build 2, so the recommendation is feasible. The concrete exposure today is cosmetic (hiding the traffic-light buttons), which is consistent with medium priority for a defence-in-depth change.

</details>

### desktop-native-context-split

**Split commands/context.rs and give store identities a typed key**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: desktop-native
- Verification: adjusted

context.rs (2458 lines) holds an AppState with 24 shared fields and 62 methods. It mixes mutation scopes, catalog generations and epochs, chat-scope access, about 20 selected_* resolvers, cached account, device, roster and federation facts, and drop or picker path grants, with 92 lock().unwrap_or_else(PoisonError::into_inner) call sites. Store identities are JSON strings re-parsed through serde_json::Value at four sites. Lookups re-serialize every candidate catalog store (or item) to JSON to compare strings.

**Evidence**

- [`apps/desktop/src-tauri/src/commands/context.rs:48`](../../../apps/desktop/src-tauri/src/commands/context.rs#L48): AppState: 24 shared fields across catalog, chat, scopes, upload paths and facts caches
- [`apps/desktop/src-tauri/src/commands/context.rs:120`](../../../apps/desktop/src-tauri/src/commands/context.rs#L120): scoped() re-points six Arc fields to the per-scope MutationScopeState; they duplicate scope_state
- [`apps/desktop/src-tauri/src/commands/context.rs:180`](../../../apps/desktop/src-tauri/src/commands/context.rs#L180): for_store parses the store id JSON; the same ad-hoc parse recurs at 206, 1254 and 2409
- [`apps/desktop/src-tauri/src/commands/context.rs:1228`](../../../apps/desktop/src-tauri/src/commands/context.rs#L1228): selected_item serializes store_id(&item.store) for every catalog item on each lookup
- [`apps/desktop/src-tauri/src/commands/context.rs:1287`](../../../apps/desktop/src-tauri/src/commands/context.rs#L1287): selected_team clones and serializes every team store per lookup; same pattern at 1347, 1480, 1536, 1759, 1764
- [`apps/desktop/src-tauri/src/commands/vault.rs:209`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L209): store_id builds the JSON identity via serde_json::json! (sorted keys) that the renderer echoes back

**Recommendation**

(1) Add a StoreKey type, an enum of Account and Team with profile, aliases and team_id. Parse it once from the id string at command entry. Its Display must be byte-identical to the current serde_json::json! output, sorted keys included, because chat-contract.ts parses store ids. Build a HashMap<StoreKey, usize> for stores and items in accept_catalog_locked so lookups do no serialization. (2) Split context.rs into context/scopes.rs (MutationScope, MutationScopeState, scoped, begin_mutation, MutationGuard), context/catalog.rs (generations, epochs, retired profiles, publish and accept, freshness), context/lookup.rs (the selected_* resolvers and store-error helpers), context/chat.rs (selected_chat, require_chat_access, accept_chat_scope), context/facts.rs (accounts, devices, rosters, federations) and context/grants.rs (drop and picked paths). Replace the six per-view aliased Arc fields with accessors through scope_state so that scoped() only swaps scope and scope_state. (3) Add a private LockExt::lock_unpoisoned helper and use it at the 92 sites, and at the other 59 in the crate if desired. The tests in commands/tests/context.rs (1429 lines) cover a behaviour-preserving move.

<details><summary>Verifier note</summary>

The core claim holds. context.rs is 2458 lines. AppState has 24 Arc/handle fields spanning catalog generations and epochs, chat views, mutation scopes, upload path grants and fact caches (accounts, devices, rosters, federations). There are exactly 92 `PoisonError::into_inner` sites in context.rs (151 across the desktop crate), and no existing lock helper trait. The store id JSON is parsed ad hoc via serde_json::Value at lines 180 (for_store), 206 (clear_profile_facts), 1254 (retired_store_error) and 2409 (unrefreshed_store_error). Lookups re-serialize catalog entries through store_id() for every candidate at lines 843, 1228, 1287, 1347, 1480, 1536, 1759 and 1764. Line 1228 serializes once per catalog item, which is worse than the per-store case cited. tests/context.rs is 1429 lines. Not tracked in ISSUES.md. Corrections: impl AppState has 62 methods, not 49. The selected_team comparison is at line 1287, not 1279. The proposed four-module split leaves out the largest group: about 20 selected_* store and target resolvers (selected_item, selected_team, selected_account, selected_store, selected_member_target, ...) and the chat-scope methods (selected_chat, require_chat_access, accept_chat_scope). Also, scoped() (lines 120-166) re-points catalog_load, catalog_generation, chat_generation, catalog_load_generation, mutation_in_flight and mutation_requires_refresh at the per-scope MutationScopeState. Those fields duplicate scope_state and can be replaced by accessors, and a sub-struct split must preserve that per-view aliasing. serde_json::json! emits keys in sorted order without preserve_order, so StoreKey's Display must reproduce that order. chat-contract.ts:345 parses store ids in the renderer.

</details>

### desktop-native-error-code-registry

**Replace free-form error-code strings with a registry shared by Rust and TypeScript**

- Type: code-quality
- Priority: medium
- Effort: M
- Layers: desktop-native, desktop-ui, agent
- Verification: confirmed

AgentError.code is a String. Besides the exhaustive mapping from agent ErrorCode, the native layer makes up roughly 100 local codes inline via AgentError::new("..."). The renderer classifies recovery by comparing against hard-coded string arrays. Where no specific code exists, the UI falls back to regexes over human-readable messages, so rewording a Rust message changes UI behaviour. The TypeScript CommandError also decodes details.kind and details.operation, which the native layer never produces; only mock-bridge.ts fills them. The window_state commands return bare String errors, which the renderer normalizes to 'invalid-command-error'.

**Evidence**

- [`apps/desktop/src-tauri/src/agent/transport.rs:16`](../../../apps/desktop/src-tauri/src/agent/transport.rs#L16): AgentError::new(code: &str, ..) accepts any string; from_agent (line 64) is exhaustive, but local codes are not enumerated anywhere
- [`apps/desktop/src/bridge/errors.ts:35`](../../../apps/desktop/src/bridge/errors.ts#L35): commandRecovery classifies codes through string-literal arrays; a renamed Rust code silently falls through to 'refresh'
- [`apps/desktop/src/invitation-writes.ts:46`](../../../apps/desktop/src/invitation-writes.ts#L46): hardwareUnlockRequested regex-matches the message text 'unlock the account key|security key|hardware key'; 'hardware-required' is never emitted as a command error code. Line 51 matches /index range/ in messages.
- [`apps/desktop/src/shell/sync-popover.tsx:748`](../../../apps/desktop/src/shell/sync-popover.tsx#L748): Classifies a server failure as 'Cancelled' by regex over the message text
- [`apps/desktop/src-tauri/src/agent.rs:73`](../../../apps/desktop/src-tauri/src/agent.rs#L73): AgentErrorDetails has no kind or operation fields, but errors.ts:17-25 decodes both, and only mock-bridge.ts:94 sets kind

**Recommendation**

Introduce enum DesktopErrorCode (serde kebab-case) in src/agent.rs. Each variant carries const metadata (retryable, fatal, recovery class: ignore, retry, refresh, reconnect, quarantine(scope) or revalidate). AgentError::new takes the enum, and from_agent maps into it. Add a Rust test that serializes the full table to apps/desktop/src-tauri/wire-contract/error-codes.json, checked by assemble.mjs --check. Generate a TypeScript union from that file, or check it in a TS test, so that commandRecovery reads its classification from the table and every literal code compared in src/ is a known code. For the two message regexes, add agent ErrorCode variants or stable ErrorFields.reason tokens (for example reason='hardware-key-required' or 'team-nesting-refused'); the agent IPC is internal and pre-v1, so Go interop is unaffected. Remove details.kind and details.operation from the decoder and the mock.

**Already tracked:** wire-contract/inventory.json lists 'CommandError details and codes' under partialCoverage; this proposes the concrete registry

<details><summary>Verifier note</summary>

AgentError.code is a String, and AgentError::new takes &str (transport.rs:16). About 87 distinct local codes are created inline outside tests, alongside the exhaustive from_agent mapping. commandRecovery (errors.ts:34-74) classifies codes with string-literal arrays and falls back to retry or refresh. hardwareUnlockRequested (invitation-writes.ts:46) regex-matches message text. 'hardware-required' appears natively only as an account status in portability.rs, not as an error code. isNestingRefusal (line 51) matches /index range/. That pattern also catches unrelated messages such as 'team index range can be changed only by an administrator' (foks-client/src/team/metadata.rs:103). The hardware regex likewise matches 'wrong account hardware key' and 'FOKS hardware key operation failed', so misclassification is already possible. sync-popover.tsx:748 regex-matches 'cancelled'. AgentErrorDetails (agent.rs:73) has no kind or operation fields; errors.ts decodes both but nothing reads them, and only mock-bridge.ts:94 sets kind. The window_state commands return String errors, which normalizeCommandError turns into 'invalid-command-error'. wire-contract/inventory.json:208 lists 'CommandError details and codes' as partial coverage, and the finding adds a concrete registry design on top of that. foks-agent-proto is the versioned local protocol for foks-rs frontends, so adding ErrorCode variants or reason tokens does not touch Go FOKS wire compatibility.

</details>

### desktop-native-linux-notifications-and-badges

**Linux chat notifications via org.freedesktop.Notifications, plus channel grouping and an unread badge**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: desktop-native, desktop-ui, docs
- Verification: adjusted

On every non-macOS platform the notification platform module is a set of stubs: available() is false and display() returns an error, so Linux users get no chat alerts. The macOS path works but is minimal. The title is always 'FOKS', the body is generic unless previews are on, notifications are not grouped per channel, and the app icon shows no unread count. zbus is already a Linux dependency for polkit, so a native implementation needs no new crate. PERMISSIONS.md still says desktop notifications are unused.

**Evidence**

- [`apps/desktop/src-tauri/src/commands/chat_local/platform.rs:6`](../../../apps/desktop/src-tauri/src/commands/chat_local/platform.rs#L6): cfg(not(macos)) stubs: available() false; display() errors 'Desktop notifications are unavailable on this platform.'
- [`apps/desktop/src-tauri/Cargo.toml:52`](../../../apps/desktop/src-tauri/Cargo.toml#L52): zbus = "5" already linked on non-macOS Unix, used by applock/polkit.rs
- [`apps/desktop/src-tauri/src/commands/chat_local/platform/macos.rs:131`](../../../apps/desktop/src-tauri/src/commands/chat_local/platform/macos.rs#L131): Fixed title 'FOKS'; unique identifier per alert and no threadIdentifier, so no per-channel grouping
- [`apps/desktop/src-tauri/src/commands/chat_local.rs:560`](../../../apps/desktop/src-tauri/src/commands/chat_local.rs#L560): alert_text produces only 'New chat message.' / '<n> additional chat messages.'
- [`apps/desktop/src-tauri/src/commands/chat_local.rs:570`](../../../apps/desktop/src-tauri/src/commands/chat_local.rs#L570): route_current/activate/delivery_failed are cfg(macos)-only and must be un-gated for Linux
- [`apps/desktop/src/chat/notification-provider.tsx:251`](../../../apps/desktop/src/chat/notification-provider.tsx#L251): Existing enable, previews and per-channel inherit/all/none override controls
- [`apps/desktop/src-tauri/PERMISSIONS.md:74`](../../../apps/desktop/src-tauri/PERMISSIONS.md#L74): Stale: 'Desktop notifications are unused'

**Recommendation**

Add platform/linux.rs implementing the same interface over zbus. available() checks that org.freedesktop.Notifications.GetServerInformation succeeds. display() calls Notify with replaces_id set to the id last used for the route's chat_local::key, actions ['default','Open'], hints category im.received and the packaged desktop-entry name, and records the id against the token. A listener on ActionInvoked calls activate(app, token), and NotificationClosed drops the route. clear() calls CloseNotification for each recorded id. permission() returns service availability, and open_settings() returns the existing unavailable error. Un-gate route_current, activate and delivery_failed from cfg(macos). On macOS, set threadIdentifier to the channel key. When previews are on, take the title from a renderer-supplied channel label, bounded and validated like body. Add a set_unread_badge(count) command using WebviewWindow::set_badge_count, gated by require_main_window and cleared in chat_local::conceal. Update PERMISSIONS.md. The mockup should extend the existing Settings > Desktop alerts section and the per-channel override control, adding a platform status row, grouping, a badge toggle and a preview card.

**Already tracked:** ISSUES.md line 186: 'non-macOS native notifications remain deferred'; this adds a concrete zbus design and the grouping/badge refinements

**Mockup:** [Channel audience and alert settings](../mockups/chat-channel-info-and-alerts.html)

<details><summary>Verifier note</summary>

The core claim holds. platform.rs is cfg(not(macos)) stubs: available() returns false and display() returns 'Desktop notifications are unavailable on this platform.'. macos.rs:131 sets the fixed title 'FOKS' and sets no threadIdentifier. Every display uses a fresh token, so alerts are not grouped. alert_text (chat_local.rs:560) produces only the generic strings. zbus = "5" is at Cargo.toml:52 under cfg(all(unix, not(macos))) and is used in applock/polkit.rs. Nothing calls set_badge_count. PERMISSIONS.md:74 still says 'Desktop notifications are unused'. ISSUES.md:186 tracks the non-macOS deferral; the finding adds a concrete design. Adjustments: (1) Several controls the mockup brief describes already exist. Settings has a 'Desktop alerts' section (settings-screen.tsx:255, NotificationSettings) with enable and 'Include message previews' toggles. Per-channel overrides with inherit/all/none modes exist (Settings.overrides, notification-provider.tsx:251-308; chat_local.rs drops alerts for overridden channels). The mockup should extend these, not reintroduce them. (2) activate, route_current and delivery_failed in chat_local.rs are cfg(target_os = "macos") and must be un-gated for a Linux backend. (3) The renderer already handles available=false ('Desktop alerts are unavailable in this build.'), so the Linux availability probe connects to existing UI.

</details>

### desktop-native-operation-builders-triplicated

**Agent Operations are built in three places; DesktopModel is dead and the real-agent CI test bypasses the shipping code**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: desktop-native, tooling, ci, docs
- Verification: adjusted

foks-desktop's DesktopModel (lib.rs:1834-2835, plus private validators through about 2976), Screen, PassphraseAction and YubiAction are used only by their own unit tests. Neither foks-desktop-backend nor the Tauri app uses them, yet the README presents DesktopModel as the operation-building core. The scripted backend (main.rs, 101 Operation:: sites) and the Tauri command layer (19 *_operation builders) build Operations independently, and their rules already differ. The backend and DesktopModel reject a name on ad-hoc teams; the Tauri builder deliberately ignores it (tested as 'renderer name is not sent'). The real-agent CI transcript shares the KV mutation builders with the app, but its account, team, backup and passphrase steps exercise backend-only builders, not the shipping ones.

**Evidence**

- [`crates/foks-desktop/src/lib.rs:1834`](../../../crates/foks-desktop/src/lib.rs#L1834): The DesktopModel struct and impl end at line 2835. The private validators follow through about line 2976. None is referenced outside this file; required_verbatim_text is also used at lines 1510 and 1636.
- [`crates/foks-desktop/src/lib.rs:2411`](../../../crates/foks-desktop/src/lib.rs#L2411): DesktopModel rejects ad-hoc teams with a name: 'ad-hoc teams do not support team names'
- [`crates/foks-desktop/src/main.rs:660`](../../../crates/foks-desktop/src/main.rs#L660): The scripted backend separately rejects --name for ad-hoc teams and builds Operation::CreateTeam itself (101 Operation:: sites)
- [`apps/desktop/src-tauri/src/commands/groups.rs:557`](../../../apps/desktop/src-tauri/src/commands/groups.rs#L557): create_group_operation deliberately maps Adhoc to an empty name. commands/tests/groups.rs:546-559 asserts this, and the renderer sends '' for ad-hoc (create-team-sheet.tsx:101).
- [`crates/foks-desktop/src/main.rs:907`](../../../crates/foks-desktop/src/main.rs#L907): The backend reuses the shared foks_desktop KV builders that src-tauri also calls, so the transcript covers that part of the shipping code
- `github/workflows/foks-desktop.yml:64`: The real managed-agent transcript runs scripts/test-foks-desktop-real-agent.sh, which builds and drives foks-desktop-backend
- [`crates/foks-desktop/README.md:5`](../../../crates/foks-desktop/README.md#L5): Claims that 'DesktopModel keeps operation-building logic unit testable', although nothing uses DesktopModel

**Recommendation**

Step 1 (S): delete DesktopModel, Screen, PassphraseAction, YubiAction, the validators only they use, and their tests. Keep required_verbatim_text, which other code uses. Correct the README. Step 2: move the pure builders and response validators from src-tauri/src/commands into foks_desktop::ops::<domain>. Start with the 19 *_operation functions in groups.rs, execution.rs, yubikey.rs, portability.rs and vault.rs, and the servers.rs response validators. Return a typed input error that src-tauri maps to AgentError. Step 3: have foks-desktop-backend call the same functions, so the real-agent transcript covers the shipping validation for onboarding, team and backup operations as it already does for KV. Pilot with groups. Choose one ad-hoc-name rule in the shared builder, either the shipping rule (ignore) or rejection with a matching renderer change, and pin it with one test.

<details><summary>Verifier note</summary>

The core claim holds. DesktopModel (lib.rs:1834; impl through 2835, private validators through about 2976), Screen, PassphraseAction and YubiAction have no references outside crates/foks-desktop/src/lib.rs. The backend main.rs builds Operations inline (101 Operation:: sites). src-tauri has 19 *_operation builders. CI line 64 runs scripts/test-foks-desktop-real-agent.sh, which builds and drives foks-desktop-backend. The README still credits DesktopModel. ISSUES.md does not track any of this. Corrections: (a) The ad-hoc name difference is a deliberate, tested design in the shipping path, not a silent bug. groups.rs:557-558 explains the choice, commands/tests/groups.rs:546-559 asserts it ('renderer name is not sent'), and the renderer always sends name '' for ad-hoc (create-team-sheet.tsx:101). Recommending 'reject' overrides a tested decision; the point is that the rule should live in one place. (b) The real-agent transcript does exercise shipping library code: the backend calls the shared foks_desktop KV builders (create_kv_file_mutation, edit_kv_file_mutation, remove_kv_operation, create_kv_symlink_operation, the upload headers) that src-tauri also uses. Only the onboarding, team, backup and passphrase Operations in that transcript are backend-only. (c) required_verbatim_text is also used outside DesktopModel (lib.rs:1510, 1636), so it must survive the deletion. Priority should be medium: the deletion is small, and the rest is maintainability.

</details>

### desktop-native-release-matrix

**Widen the release matrix (universal macOS, rpm, aarch64 Linux) and record Windows as an explicit non-goal**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: ci, tooling, desktop-native, docs
- Verification: adjusted

Linux release blocker: the AppStream metainfo has no <description> and no homepage <url>. The existing tag-only `appstreamcli validate --no-net` step (foks-desktop.yml:207, AppStream 1.0.2 on ubuntu-24.04) treats these as an ERROR (app-description-required) and a WARNING (url-homepage-missing), and both fail the step. The first release tag will therefore fail the Linux job and block attest and publish. PR CI never runs this validation.

The matrix is also narrow. The release produces only arm64 DMGs, although minimumSystemVersion 13.0 includes Intel Macs, and only x86_64 .deb packages. There is no rpm, although the AppImage objection (no install step for the polkit policy) does not apply to rpm. There is no aarch64 Linux build. stage-foks-agent.sh stages only the host triple, so universal or cross builds are impossible.

productName 'foks-desktop' becomes the macOS app and Dock name. Windows is not built: build-desktop.mjs throws and foks-agent exits on non-Unix.

**Evidence**

- `github/workflows/foks-desktop.yml:207`: appstreamcli validate --no-net already runs, but only in the tag-only `linux` job (line 168 `if: startsWith(github.ref, 'refs/tags/')`). Missing description = ERROR and missing homepage = WARNING in AppStream 1.0.2, so the step fails.
- [`apps/desktop/src-tauri/linux/com.aka.foks.desktop.metainfo.xml:7`](../../../apps/desktop/src-tauri/linux/com.aka.foks.desktop.metainfo.xml#L7): No <description> or <url type="homepage">; unchanged since it was added in 8cb0d1a
- `github/workflows/foks-desktop.yml:26`: Validate macOS arm64 / Validate Linux x86_64 (68) / macOS arm64 release (116) / Linux x86_64 .deb (167); no other targets
- [`apps/desktop/src-tauri/tauri.bundle.linux.conf.json:4`](../../../apps/desktop/src-tauri/tauri.bundle.linux.conf.json#L4): targets ["deb"] only
- [`scripts/stage-foks-agent.sh:5`](../../../scripts/stage-foks-agent.sh#L5): Host triple only; copies target/release/foks-agent
- [`scripts/build-macos-release.sh:52`](../../../scripts/build-macos-release.sh#L52): Release DMG name and .app path hard-code arm64 and foks-desktop.app
- [`apps/desktop/src-tauri/tauri.conf.json:3`](../../../apps/desktop/src-tauri/tauri.conf.json#L3): productName "foks-desktop"; tauri-bundler 2.11.4 derives the deb Package name, the .desktop filename and /usr/lib/<name> from it
- [`crates/foks-agent/src/main.rs:197`](../../../crates/foks-agent/src/main.rs#L197): Non-Unix agent exits

**Recommendation**

(0) Add a <description>, a homepage <url> and one <releases> entry to the metainfo now. Move `appstreamcli validate --no-net` and `desktop-file-validate` into the validate-linux PR job so this failure appears before a tag is pushed.
(1) Let stage-foks-agent.sh accept --target and copy from target/<triple>/release. For macOS, build aarch64 and x86_64 agents, lipo them into binaries/foks-agent-universal-apple-darwin, and build with --target universal-apple-darwin. Update build-macos-release.sh (the .app path and the -macos-arm64 name) and the CI paths.
(2) Add a bundle.linux.rpm block with the same files and depends ["polkit", "pcsc-lite-libs"]. Tauri adds the WebKitGTK and GTK soname requires itself. Inspect the rpm in CI as the deb is inspected.
(3) Add an ubuntu-24.04-arm job for aarch64.
(4) Change the macOS product name only. Override productName to "FOKS" in tauri.bundle.macos.conf.json, or set CFBundleName/CFBundleDisplayName through a merged Info.plist. Keep the Linux productName "foks-desktop", because Tauri derives the deb and rpm Package name, the .desktop filename and /usr/lib/<name> from it.
(5) Add [profile.release] with strip = "debuginfo" and lto = "thin".
(6) State in RELEASE_POLICY.md that Windows is a non-goal, and list the prerequisites: named-pipe IPC with peer-SID checks, Windows Hello and a keystore backend. The tauri.localhost allowance is already documented and can stay.

**Already tracked:** linux/README.md 'Why there is no AppImage' (deliberate); crates/foks-desktop/README.md:187 'Windows is not a target'. rpm, aarch64 and universal macOS are not tracked.

<details><summary>Verifier note</summary>

The core claims hold. Both macOS jobs run on macos-15 (arm64), and build-macos-release.sh:50-53 hard-codes foks-desktop.app and the -macos-arm64.dmg name. tauri.bundle.linux.conf.json:4 has targets ["deb"] only. stage-foks-agent.sh:5 uses the rustc host triple and copies target/release/foks-agent. productName is "foks-desktop". There is no [profile.release] in Cargo.toml. build-desktop.mjs throws on any platform other than darwin and linux, and main.rs:197 is the non-Unix exit. rpm, aarch64 and universal builds are not tracked anywhere.

Three corrections are needed. (a) CI already runs `appstreamcli validate --no-net` on the metainfo, at foks-desktop.yml:207, but only in the tag-only `linux` release job. In AppStream 1.0.2 (ubuntu-24.04), a desktop-application with no <description> raises app-description-required at ERROR severity, and a missing homepage <url> raises url-homepage-missing at WARNING. appstreamcli fails by default on both errors and warnings, so the first foks-desktop-v* tag will fail the Linux release job, which also blocks attest and publish. No tag exists yet and the metainfo has never had a description, so the failure is latent. This is a concrete release blocker, so the priority rises to medium. <releases> is only PEDANTIC and <content_rating> only INFO, so they are not required as claimed. (b) Recommendation (4) is unsafe as written. In tauri-bundler 2.11.4, the deb and rpm Package name is kebab-case(productName), the .desktop file is `{productName}.desktop`, and the resource directory is /usr/lib/{productName}. Setting productName to "FOKS" globally would rename the Debian package to `foks`, which breaks upgrades and the CI check `Package = foks-desktop`. It would also rename the desktop file to FOKS.desktop, which breaks the metainfo <launchable>foks-desktop.desktop</launchable> and the desktop-file-validate path. (c) The tauri.localhost allowance is not an unlabelled remnant. PERMISSIONS.md:144-147 documents it as covering Windows, Android and useHttpsScheme. windows_subsystem has no effect on Unix.

</details>

### desktop-native-startup-reset-safety

**A startup timeout can lead to a destructive state reset; add retry, failure classification and quarantine**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: desktop-native, agent, desktop-ui
- Verification: adjusted

The desktop waits a fixed 5 s (50 x 100 ms) for the agent socket, while the agent reads the vault master key from the OS credential store before it binds. On failure, require_agent never consults error.code and never offers Try Again: with client-state.toml present the only choices are 'Review Reset…' or 'Quit'. A slow Keychain or Secret Service prompt cannot itself cause deletion, because the still-running agent holds ClientStateLease and reset_unreadable_state then fails with StateBusy. The user is still pushed to quit, or into a reset attempt that ends in a misleading 'Deletion may be incomplete' fatal dialog. When the agent exits because the credential store was denied or locked (a transient cause, reported as exit status 1 like every other failure), the lease is released. The destructive reset is then the only alternative to Quit, and it deletes keys, agent.log and crashes/ without checking that the state is actually unreadable.

**Evidence**

- [`apps/desktop/src-tauri/src/agent/process.rs:1090`](../../../apps/desktop/src-tauri/src/agent/process.rs#L1090): Fixed 50 x 100 ms socket wait. On timeout it returns the last probe error. The 'agent-start-failed' fallback at line 1115 applies only when no probe error was recorded.
- [`crates/foks-agent/src/main.rs:229`](../../../crates/foks-agent/src/main.rs#L229): The agent acquires ClientStateLease before vault_master_key(&open_credentials(..)) at line 234 and bind_private_agent_socket at line 257. A slow credential prompt therefore holds the lease.
- [`crates/foks-client-app/src/portability/lease.rs:393`](../../../crates/foks-client-app/src/portability/lease.rs#L393): reset_unreadable_state takes an exclusive ClientStateMaintenanceGuard, which returns StateBusy while an agent holds the lease. If no lease is held, it removes every entry except the lock files (remove_dir_all at line 427), including agent.log and crashes/.
- [`apps/desktop/src-tauri/src/startup.rs:226`](../../../apps/desktop/src-tauri/src/startup.rs#L226): Any startup error with a missing socket offers 'Review Reset…' or 'Quit'. error.code is ignored apart from takeover-declined, and there is no Try Again.
- [`apps/desktop/src-tauri/src/agent/maintenance.rs:516`](../../../apps/desktop/src-tauri/src/agent/maintenance.rs#L516): startup_reset_directory is Some whenever the socket is absent and client-state.toml exists, whatever the cause. reset_startup_state (line 528) re-checks this before deleting.
- [`apps/desktop/src-tauri/src/agent/process.rs:243`](../../../apps/desktop/src-tauri/src/agent/process.rs#L243): early_exit_error already appends up to 4 KiB of agent output to the message. The timeout path shows no log.
- [`crates/foks-agent/src/main.rs:189`](../../../crates/foks-agent/src/main.rs#L189): Every agent startup failure exits with status 1 (lines 183, 190), so the desktop cannot tell causes apart

**Recommendation**

(1) Replace the fixed 5 s loop. Keep polling while early_exit_error reports the child alive, up to a bound such as the 60 s already used for --request-timeout-seconds. After 5 s, show progress that mentions a possible keychain prompt. (2) Have the agent report the failure class, either with distinct exit codes (for example 65 for unreadable state, 75 for a temporary or credential-store failure, 73 when the socket or lock cannot be created) or with a small startup-failure.json beside agent.log. Map the class in ManagedAgentLaunch::early_exit_error. (3) In require_agent, always offer Try Again (loop back to ensure_started_with_confirmation) and Show Log. Offer reset only for the unreadable-state class, never for a credential-store denial or a timeout. (4) Make reset non-destructive. Move every entry except the lock files that reset_unreadable_state deliberately keeps in place into a sibling quarantine directory such as '<root>.reset-<unix-time>', rather than renaming the root. Native credential-store entries are already left alone, so the quarantined keys stay recoverable. Offer permanent deletion as a separate later action. Add a process test using the takeover-agent.c fixture pattern: a slow-binding agent, and an agent that exits with the credential-failure class, must both produce Try Again and no reset offer.

<details><summary>Verifier note</summary>

Most of the core claim holds. The socket wait is fixed at 50 x 100 ms (process.rs:1090). The agent reads the vault master key (main.rs:234) before it binds the socket (main.rs:257). require_agent (startup.rs:226-260) offers only 'Review Reset…' or 'Quit' whenever startup_reset_directory is Some, ignores error.code except for takeover-declined, and never offers Try Again. reset_unreadable_state deletes every entry except the lock files, including agent.log and crashes/. Every agent failure exits with status 1. The headline scenario, however, is blocked by existing locking: a startup timeout cannot lead to deletion while the slow agent is still alive. The agent acquires ClientStateLease (a shared path lock, main.rs:229) before it reads the credential store. ManagedAgentLaunch does not kill the child on timeout. reset_unreadable_state takes an exclusive ClientStateMaintenanceGuard (lease.rs:393) and therefore fails with StateBusy, which the existing test startup_reset_refuses_active_clients_and_legacy_agent_locks covers. reset_startup_state also re-checks socket absence (maintenance.rs:528). Several details are also wrong. The timeout returns the last probe error, not the 'agent-start-failed' fallback. Early-exit errors already append the last 4 KiB of agent output to the dialog message (process.rs:243-252), so the log is visible in that case. The function is at lease.rs:392, not 420. The real risks are these. (a) A slow keychain prompt leaves the user only Quit, or a reset attempt that fails with a misleading 'Deletion may be incomplete' fatal dialog. (b) If the user denies the keychain prompt, or the keyring is locked, the agent exits, the lease is released, and a destructive reset is offered as the only alternative to Quit, behind two strongly worded confirmations. Both deserve a fix, at medium priority.

</details>

### desktop-native-typed-agent-responses

**Stop mirroring agent response types in the desktop; decode into foks-agent-proto types**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: agent, desktop-native, client-lib
- Verification: adjusted

Agent success replies are untyped serde_json::Value. The command layer therefore declares 44 private Deserialize structs ending in *Response, each with deny_unknown_fields, to mirror shapes that the agent builds from typed proto structs. For example, servers.rs redeclares ServerStatusSnapshot and StoredHostStatus field for field and keeps compatibility as a raw Value. Both sides live in the same workspace, yet a field rename in the agent only shows up as a desktop runtime 'invalid-response' error.

**Evidence**

- [`crates/foks-agent-proto/src/message.rs:2324`](../../../crates/foks-agent-proto/src/message.rs#L2324): ResponseResult::Success { value: serde_json::Value }: no per-operation response type
- [`apps/desktop/src-tauri/src/commands/servers.rs:578`](../../../apps/desktop/src-tauri/src/commands/servers.rs#L578): ServerStatusResponse/StoredHostResponse duplicate proto ServerStatusSnapshot/StoredHostStatus (message.rs:281-296); compatibility kept as raw Value
- [`apps/desktop/src-tauri/src/commands/context.rs:1038`](../../../apps/desktop/src-tauri/src/commands/context.rs#L1038): The same DescribeServerStatus payload is decoded here as foks_agent_proto::ServerStatusSnapshot: two decoders for one reply
- [`crates/foks-desktop/src/lib.rs:1776`](../../../crates/foks-desktop/src/lib.rs#L1776): decode_agent_value<T> already decodes ProfileOverview, AccountSummary, TeamSummary, ServerStatusSnapshot (line 1111) into proto types
- [`apps/desktop/src-tauri/src/commands/accounts.rs:29`](../../../apps/desktop/src-tauri/src/commands/accounts.rs#L29): AccountResponse is field-for-field proto AccountSummary (message.rs:105)
- [`crates/foks-agent/src/main.rs:4013`](../../../crates/foks-agent/src/main.rs#L4013): Agent serializes wire_server_status(...) i.e. the typed proto ServerStatusSnapshot
- [`apps/desktop/src-tauri/src/commands/tests/contract.rs:392`](../../../apps/desktop/src-tauri/src/commands/tests/contract.rs#L392): Contract tests use hand-written JSON literals, so an agent-side field rename is not caught

**Recommendation**

In foks-agent-proto, add typed response structs per Operation family, reusing existing message.rs types (ServerStatusSnapshot, AccountSummary, TeamSummary, KvReadResult, ProfileOverview). Secret fields use SecretString/Zeroizing. Add a typed call, for example `call_typed<R: OperationResponse>`, on the Tauri AgentHandle and on foks-desktop's AgentTransport, building on the existing decode_agent_value. Have the agent build replies from those types. Migrate the desktop domain by domain, starting with servers.rs and accounts.rs, where exact proto duplicates already exist. Delete each mirror and keep binding and bounds validation as functions over the proto types. For strictness, either add deny_unknown_fields to the proto response types as a documented choice (it also affects foks-cli, foks-mcp and the testkit, which is acceptable because frames are PROTOCOL_VERSION-pinned) or keep it local through a desktop-side strict decode. Until migration completes, change the desktop contract tests to serialize proto values instead of JSON literals, so an agent rename fails at test time. This is internal agent IPC and does not touch the Go wire protocol.

<details><summary>Verifier note</summary>

The core claim holds. ResponseResult::Success carries serde_json::Value (message.rs:2324). The Tauri command layer has exactly 44 `*Response` structs, all marked #[serde(deny_unknown_fields)]; some are pub(super), so they are module-private rather than private. There are 80 non-test success_value/serde_json::from_value sites. servers.rs:578-598 mirrors proto ServerStatusSnapshot/StoredHostStatus (message.rs:281-296). The agent serializes the proto type through wire_server_status (main.rs:4013). accounts.rs:29 AccountResponse copies proto AccountSummary (message.rs:105) field for field. The desktop contract tests (tests/contract.rs) feed hand-written JSON literals, not serialized proto values, so an agent-side rename is not caught at compile or test time. Nothing in ISSUES.md tracks this. Evidence corrections: vault.rs:1078 is the list_catalog command, not a decode site; the nearest decode sites are vault.rs:921 and 1113. The finding also misses that the same DescribeServerStatus payload is already decoded as the proto type in two other places, commands/context.rs:1038 and crates/foks-desktop/src/lib.rs:1111. foks-desktop/src/lib.rs:1776 decode_agent_value already decodes ProfileOverview, AccountSummary, TeamSummary and KvReadResult into proto types. So the migration pattern exists and the mirrors are an inconsistency, not a missing capability. Recommendation caveat: proto types are also consumed by foks-cli, foks-mcp and foks-server-testkit. Putting deny_unknown_fields on them makes every consumer strict. That is tolerable only because frame.rs:91 rejects PROTOCOL_VERSION mismatches, and it should be stated as a deliberate choice. Secret-bearing replies (RecoveryPhraseResponse, ResetPreviewResponse token) need Zeroizing/SecretString fields in the proto types before their mirrors can be removed.

</details>

### desktop-native-updater

**No update channel: add signed in-app updates for macOS and an update notice for the .deb**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: desktop-native, desktop-ui, ci, docs
- Verification: adjusted

The app has no updater, no version check and no release feed. Users must download a new DMG or .deb by hand, although the UI tells them to 'Update FOKS' when a store is schema-incompatible, and a server can mark the client incompatible (protocol-mismatch or unknown-capability). The foks-desktop README says RELEASE_POLICY.md specifies updates, but it says nothing about updates. The managed foks-agent ships inside the bundle and must be replaced together with the app.

**Evidence**

- [`apps/desktop/src-tauri/Cargo.toml:35`](../../../apps/desktop/src-tauri/Cargo.toml#L35): Tauri plugins are clipboard-manager, dialog and single-instance (lines 35-37); no tauri-plugin-updater
- [`apps/desktop/src/screens/store-access.tsx:156`](../../../apps/desktop/src/screens/store-access.tsx#L156): schema-incompatible: 'Update FOKS or contact your administrator', no in-app mechanism
- [`crates/foks-desktop/README.md:186`](../../../crates/foks-desktop/README.md#L186): Claims updates are specified in packaging/foks-desktop/RELEASE_POLICY.md; that file has no update content
- `github/workflows/foks-desktop.yml:246`: Publish job uploads DMG, .deb and SHA256SUMS only; no updater manifest or signature
- [`crates/foks-agent-proto/src/message.rs:259`](../../../crates/foks-agent-proto/src/message.rs#L259): CompatibilityStatus::Incompatible{reason: Drift | ProtocolMismatch | UnknownCapability}: only the latter two suggest an update

**Recommendation**

macOS: add tauri-plugin-updater driven from Rust with no webview permission. Set bundle.createUpdaterArtifacts, sign with a minisign key held as a CI secret, and have the publish job attach latest.json, the .app.tar.gz and its .sig, covered by the existing attest step. Check at launch and every 24 h with a Settings opt-out, and document this as the desktop process's own network egress. Refuse downgrades. Install only through close_guard: stop the managed agent, install, relaunch. Block installation while unsent messages, an in-flight mutation or maintenance are pending. Linux .deb: read the same feed and show 'Version X available' with a link; do not attempt in-place install unless the pinned plugin supports .deb without ad-hoc privilege escalation. Later, publish a signed APT repository. Point the update prompt at the schema-incompatible and Incompatible{protocol-mismatch, unknown-capability} states, not at ordinary lease expiry. Add an 'Updates' section to RELEASE_POLICY.md covering key custody, feed URL and rollback, and fix the README claim.

<details><summary>Verifier note</summary>

The core claim holds. No tauri-plugin-updater dependency, updater config or update check exists anywhere. The Cargo plugins are clipboard-manager, dialog and single-instance (lines 35-37; line 34 is the tauri dependency). store-access.tsx:156 says 'Update FOKS or contact your administrator'. crates/foks-desktop/README.md:186 says updates are specified in RELEASE_POLICY.md, but that 25-line file contains no mention of updates, rollback or downgrade. The publish job uploads only *.dmg, *.deb and SHA256SUMS. close_guard.rs already tracks unsent messages and owns agent shutdown, so the sequencing proposal is grounded. Nothing in ISSUES.md tracks this. Correction: the summary's 'server compatibility leases can expire on older clients' conflates two things. Check-in and compatibility leases expire with time and are renewed by online verification. Only the Incompatible{protocol-mismatch | unknown-capability} status and the store-level 'schema-incompatible' state indicate that an update may help. So the mockup's 'banner when a compatibility lease expired, linking to the update panel' would send users to the updater for a non-version problem. Feasibility notes: the update check would be the desktop process's first direct network egress (today all network I/O goes through the agent), which should be stated and opt-out. Linux .deb in-place support depends on the pinned updater plugin version and needs privilege elevation, so a notice-only path is the conservative default.

</details>

### desktop-native-agent-log-and-crash-markers

**agent.log grows without bound and crash markers are never read**

- Type: code-quality
- Priority: low
- Effort: S
- Layers: desktop-native, agent
- Verification: adjusted

The desktop opens state_dir/agent.log in append mode for every launch and never rotates it. The agent writes untimestamped lines to stderr for recurring events such as scheduled refresh failures and connection errors. The panic hook writes crash-<ts>-<pid>.txt markers with no count limit, and nothing reads them: the UI, Copy diagnostics and startup ignore them. The agent's own abnormal exits, which the reaper thread observes, are not recorded. The README says crash markers are specified in RELEASE_POLICY.md, but they are not.

**Evidence**

- [`apps/desktop/src-tauri/src/agent/process.rs:298`](../../../apps/desktop/src-tauri/src/agent/process.rs#L298): OpenOptions create+append on agent.log for each launch; no size check or rotation
- [`crates/foks-desktop/src/lib.rs:2978`](../../../crates/foks-desktop/src/lib.rs#L2978): install_crash_reporter writes markers with no cap; no reader exists
- [`apps/desktop/src-tauri/src/agent/process.rs:349`](../../../apps/desktop/src-tauri/src/agent/process.rs#L349): Reaper sends ExitStatus to a channel read only during startup (try_recv at :244)
- [`crates/foks-agent/src/main.rs:535`](../../../crates/foks-agent/src/main.rs#L535): Recurring untimestamped eprintln!, written into agent.log
- [`crates/foks-client-app/src/portability/inventory.rs:476`](../../../crates/foks-client-app/src/portability/inventory.rs#L476): Exact-name root allowlist includes agent.log; any agent.log.1 would be rejected as an unknown root artifact
- [`crates/foks-desktop/README.md:186`](../../../crates/foks-desktop/README.md#L186): Says crash markers are specified in RELEASE_POLICY.md; they are not

**Recommendation**

In launch_agent, while holding the spawn lock, rename agent.log to agent.log.1 before opening it when it exceeds 4 MiB, keeping one generation. In the same change, add "agent.log.1" to the root-artifact allowlist in crates/foks-client-app/src/portability/inventory.rs, together with its export and reset handling, or the next portability operation fails on an unknown root artifact.

Prefix agent stderr lines with an RFC 3339 timestamp, or move foks-agent to tracing_subscriber with a timer. Cap crashes/ at the 20 newest markers.

Have the managed reaper write a marker of the same format, with a kind field, when the agent exits on a signal or with a non-zero status after it was ready.

Add a crash_reports command that returns timestamps and kinds only. Show 'FOKS quit unexpectedly on <date>' once in Settings, and include the markers in Copy diagnostics.

Fix the README sentence, or add a crash-marker section to RELEASE_POLICY.md.

<details><summary>Verifier note</summary>

The core holds.
- launch_agent (process.rs:298) opens state_dir/agent.log with create and append, with no size check or rotation. The agent's stdout and stderr both go to it.
- foks-agent writes untimestamped eprintln! lines for recurring events: scheduled refresh failed (main.rs:535) and connection failed (:351).
- install_crash_reporter (foks-desktop lib.rs:2978) writes crash-<ts>-<pid>.txt with no cap. A repository-wide search finds no reader of crashes/ apart from the portability inventory's private_directory check.
- The foks-agent-reaper thread (process.rs:349) sends the ExitStatus to a channel that is read only during startup (try_recv at :244), so exits after the agent is ready are not recorded.
- diagnostics.rs is in-memory timings only.
- crates/foks-desktop/README.md:185-187 says crash markers (and updates) are specified in RELEASE_POLICY.md, which mentions neither.

One feasibility gap: the portability inventory (crates/foks-client-app/src/portability/inventory.rs:471-479) allowlists root artifacts by exact name, including "agent.log", and fails with 'unknown root artifact blocks portability' for anything else. Rotating to agent.log.1 as recommended would break state portability unless that allowlist is extended in the same change.

</details>

### desktop-native-command-contract-check

**Check command names and argument keys between generate_handler! and the TS bridge**

- Type: testing
- Priority: low
- Effort: S
- Layers: desktop-native, desktop-ui, tooling
- Verification: adjusted

The 127 command names in lib.rs and their snake_case parameters must match the string literals and camelCase argument objects in src/bridge/*.ts. A one-off extraction found every Rust command referenced and 0 mismatches across 99 literal call sites. Nothing keeps it that way. UI tests run against mock-bridge.ts, Rust tests call handlers directly, and the packaged-WebView smoke test touches only a few commands. A renamed parameter would fail only at runtime with Tauri's 'missing required key' error. The wire contract deliberately defers DTO generation and lists request DTOs and the chat, invitation, SSO and bot shapes as uncovered.

**Evidence**

- [`apps/desktop/src-tauri/src/lib.rs:221`](../../../apps/desktop/src-tauri/src/lib.rs#L221): generate_handler! with 127 commands across 19 module prefixes
- [`apps/desktop/src/bridge/transport.ts:57`](../../../apps/desktop/src/bridge/transport.ts#L57): checked(command: string, args: Record<string, unknown> | undefined, ...): names and keys are untyped
- [`apps/desktop/src/bridge/commands-vault.ts:69`](../../../apps/desktop/src/bridge/commands-vault.ts#L69): checked('read_item', { storeId, path, version }, ..): literal name and camelCase keys
- [`apps/desktop/src/bridge/commands-yubikey.ts:17`](../../../apps/desktop/src/bridge/commands-yubikey.ts#L17): runYubi dispatches a variable command from the YubiCommand union; a literal-only walker would miss it
- [`apps/desktop/src-tauri/wire-contract/inventory.json:1`](../../../apps/desktop/src-tauri/wire-contract/inventory.json#L1): generation.status 'deferred'; uncoveredPublicShapes lists request DTOs and the chat/invitation/SSO/bot actions
- [`apps/desktop/src-tauri/wire-contract/commands.json:1`](../../../apps/desktop/src-tauri/wire-contract/commands.json#L1): Already exists as the 'commands' domain fixture file (commandError, commandAck, ...); the new manifest needs another name

**Recommendation**

Add a Rust test that scans src/**/*.rs for #[tauri::command] functions, using syn or a careful regex. It writes a manifest such as wire-contract/command-signatures.json, not commands.json, which already holds error, ack and mutation fixtures. The manifest maps each command to its camelCase argument keys, marks Option parameters optional and excludes State, AppHandle, Webview, WebviewWindow and Channel-injected types where appropriate. Cross-check the list against generate_handler! and verify it with assemble.mjs --check. Add a TS test that walks src/bridge with the TypeScript compiler API. It asserts that every checked, checkedMutation and invoke call with a literal name uses a listed command, passes no unknown keys and passes all required keys. It also enumerates the YubiCommand union in bridge/yubikey.ts, the dynamic runYubi path, against the manifest. If an app ACL manifest for commands lands, use it as the name source.

**Already tracked:** wire-contract/inventory.json defers DTO generation; this is a narrower first step limited to names and argument keys

<details><summary>Verifier note</summary>

The core claim holds. generate_handler! lists 127 commands across 19 module prefixes, at lib.rs:221 (not 205). Every Rust command name appears as a TS literal. The bridge's checked/checkedMutation (transport.ts:49-80) take `command: string` and `args: Record<string, unknown>`. No commands use rename_all, so Tauri's default camelCase argument mapping applies. UI tests use mock-bridge.ts. The real-agent transcript exercises foks-desktop-backend, not the Tauri command layer. No existing test checks command names or argument keys. inventory.json defers generation and lists request DTOs as uncovered. Corrections: (1) wire-contract/commands.json already exists as the domain file for command error, ack and mutation shapes, so the proposed output name would overwrite it; it needs another name. (2) There are about 107 literal call sites (checked, checkedMutation, invoke), not 99, plus dynamic dispatch. runYubi (commands-yubikey.ts:17-22) passes a variable command from the YubiCommand discriminated union (bridge/yubikey.ts:69+), so the TS check must enumerate that union's command/args pairs. (3) The existing ACL test in lib.rs reads tauri-build's generated gen/schemas JSON; it does not regex the source.

</details>

### desktop-native-doc-drift

**Correct specific stale statements in the desktop's security and architecture docs**

- Type: docs
- Priority: low
- Effort: S
- Layers: docs
- Verification: adjusted

Several documents that reviewers rely on for the security boundary describe an older design. PERMISSIONS.md says notifications are unused, no additional windows exist and every non-local origin is denied; native macOS notifications and the remote-origin host-admin window contradict all three. The src-tauri README module table lists 11 of the 22 command modules. The foks-desktop README says the agent 'survives window closure', whereas close_guard stops it on every close. The book claims an inactivity lock.

**Evidence**

- [`apps/desktop/src-tauri/PERMISSIONS.md:74`](../../../apps/desktop/src-tauri/PERMISSIONS.md#L74): 'Desktop notifications are unused'; line 142 'everything else deny'; line 172 'no additional windows are configured'
- [`apps/desktop/src-tauri/README.md:87`](../../../apps/desktop/src-tauri/README.md#L87): Command-domain table omits chat, chat_local, chat_migration, sso, invitations, bot, web_admin, portability, first_run, preparation and account_conveniences
- [`crates/foks-desktop/README.md:124`](../../../crates/foks-desktop/README.md#L124): 'the desktop launches a detached agent that survives window closure', contradicted by close_guard.rs (every close stops the agent)
- [`book/20-desktop.qmd:92`](../../../book/20-desktop.qmd#L92): Inactivity auto-lock is described but not implemented

**Recommendation**

Update PERMISSIONS.md with a 'Second window class: host-admin-*' section: incognito, new windows and downloads denied, origin-bound navigation through web_admin::navigation_allowed, 300 s expiry, and no capability. Add a 'Native notifications' section: UNUserNotificationCenter on macOS only, no Tauri plugin, no webview permission.

Regenerate the README module table from commands/mod.rs. Optionally add a test that each declared module appears in it.

Correct the foks-desktop README lifecycle sentence: the agent is stopped on every close or quit.

Change book chapter 20 to describe the lock as on-request only, or implement inactivity lock first. The ACL audit header already exists, so no change is needed there.

<details><summary>Verifier note</summary>

Each stale statement is confirmed:
- PERMISSIONS.md:74 says desktop notifications are unused. chat_local/platform/macos.rs posts through UNUserNotificationCenter, and the commands are registered at lib.rs:309-310 and 355.
- PERMISSIONS.md:137-142 and 172 describe one allowlist and no additional windows. web_admin.rs:189-193 builds incognito host-admin-* windows on remote External URLs with new windows and downloads denied and a 300 s expiry, and navigation.rs:71 routes them to a separate policy.
- The src-tauri README table (lines 85-97) lists 11 modules. commands/mod.rs declares 22, and the 11 missing names match the finding exactly.
- crates/foks-desktop/README.md:123-124 says the agent 'survives window closure', which contradicts close_guard.
- book/20-desktop.qmd:92 describes locking after inactivity. Only the menu-driven lockNow and lock_app exist; there is no idle timer.

One part of the recommendation is redundant. The PERMISSIONS.md header (lines 3-5) already names gen/schemas/acl-manifests.json and capabilities.json and says to re-verify after tauri updates, and lines 18-19 name the resolution test.

</details>

### desktop-native-linux-parity

**Small Linux parity gaps: device and user name defaults, clipboard-history hint, link opening**

- Type: feature-refinement
- Priority: low
- Effort: S
- Layers: desktop-native, docs
- Verification: confirmed

On Linux, app_info returns no computer or user name, so first-run falls back to a generic device name and an empty username. Clipboard copies carry no history-exclusion hint. PERMISSIONS.md says Linux has none, but x-kde-passwordManagerHint=secret is honoured by Klipper and other clipboard managers, and arboard (already in Cargo.lock via the clipboard plugin) can set it. Links and SSO open by running PATH-resolved xdg-open and waiting on .status(), which can block the worker when a terminal browser is configured and does not use the desktop portal.

**Evidence**

- [`apps/desktop/src-tauri/src/commands/application.rs:132`](../../../apps/desktop/src-tauri/src/commands/application.rs#L132): macos_computer_name and macos_user_name (line 160) return None on non-macOS
- [`apps/desktop/src-tauri/src/clipboard.rs:48`](../../../apps/desktop/src-tauri/src/clipboard.rs#L48): Linux path calls app.clipboard().write_text with no history-exclusion hint
- [`apps/desktop/src-tauri/PERMISSIONS.md:178`](../../../apps/desktop/src-tauri/PERMISSIONS.md#L178): 'Linux has no equivalent hint' (repeated in book/20-desktop.qmd:189)
- [`apps/desktop/src-tauri/src/commands/chat.rs:351`](../../../apps/desktop/src-tauri/src/commands/chat.rs#L351): Command::new("xdg-open") is PATH-resolved and waits for exit via .status(); open_sso_browser reuses this path (sso.rs:196)

**Recommendation**

(1) On Linux, fill computer_name from org.freedesktop.hostname1 PrettyHostname over zbus, falling back to libc::gethostname, and user_name from getpwuid_r(geteuid()).pw_name. Apply the existing length bounds. (2) Add a Linux-only arboard = "3" dependency and copy with Clipboard::set().exclude_from_history().text(..), which sets x-kde-passwordManagerHint=secret. Keep the 15 s clear, and update PERMISSIONS.md and the book. (3) Move URL opening into a small platform/opener.rs used by chat, SSO and notification settings. On Linux call org.freedesktop.portal.OpenURI.OpenURI over zbus, falling back to /usr/bin/xdg-open spawned without waiting. On macOS use /usr/bin/open.

<details><summary>Verifier note</summary>

Each claim was checked against the source.
- macos_computer_name (application.rs:132) and macos_user_name (:160) return None off macOS. In that case first-run-screen.tsx suggestedDeviceName() falls back to 'This computer' and the username field stays empty.
- The non-macOS clipboard write (clipboard.rs:49-52) uses plugin write_text with no hint.
- PERMISSIONS.md:178 and book/20-desktop.qmd:189 both state that Linux has no equivalent hint.
- arboard 3.6.1 is in Cargo.lock. Its Linux X11 and Wayland backends implement exclude_from_history by adding x-kde-passwordManagerHint.
- open_chat_link (chat.rs:351) runs the PATH-resolved "xdg-open" ("open" on macOS) and blocks on .status(). sso.rs:196 reuses it, and chat_local/platform/macos.rs:165 uses PATH-resolved "open" for notification settings.
- zbus 5 is already a Linux dependency for polkit, and libc is a dependency, so the recommendation is feasible.
- ISSUES.md does not track any of this.

Only the cited line 48 is off by two: it is the macOS branch, and the Linux write is at 49-52.

</details>

### desktop-native-quit-confirmation

**Quit and window close always show a 'stop agent' modal, even when nothing is pending**

- Type: feature-refinement
- Priority: low
- Effort: S
- Layers: desktop-native, desktop-ui
- Verification: adjusted

Every window close and every Cmd-Q moves the close guard into Decision and shows the renderer's 'Stop FOKS Agent and quit?' sheet, even when there are 0 unsent messages, no mutation in flight and no maintenance running. The native fallback dialog, used before the renderer is ready or on a repeated Cmd-Q, also prints '0 unsent messages will be discarded'. The decision has no memory. Background operation is deferred, so the realistic refinement is a quit flow that asks only when there is pending work or a shared or adopted agent.

**Evidence**

- [`apps/desktop/src-tauri/src/close_guard.rs:85`](../../../apps/desktop/src-tauri/src/close_guard.rs#L85): begin() always enters ExitState::Decision on the first request
- [`apps/desktop/src-tauri/src/close_guard.rs:215`](../../../apps/desktop/src-tauri/src/close_guard.rs#L215): Native fallback text always includes '{unsent} unsent messages will be discarded', including 0
- [`apps/desktop/src/app/exit-guard.tsx:57`](../../../apps/desktop/src/app/exit-guard.tsx#L57): Renderer decision sheet is shown on every close; unsentCopy() hides a 0 count; the copy warns that an already-running agent is also stopped
- [`apps/desktop/src-tauri/src/close_guard.rs:292`](../../../apps/desktop/src-tauri/src/close_guard.rs#L292): observe() routes every CloseRequested through the same gate

**Recommendation**

Extend ExitState::Decision with reasons: unsent count, mutation_in_flight (from the command context), active maintenance, and whether the agent was adopted rather than launched by this app (AgentProcessInfo.owned == false). AgentStatus does not report connected clients, so adding a client count would be a desktop-agent protocol extension; it does not touch the Go wire format. Add a device-local preference 'Ask before quitting: always / only when work is pending'. When it is set to the latter and no reason applies, go directly from Idle to Stopping. Keep the native fallback for the pending case and for a repeated Cmd-Q, and stop it printing a 0 count.

**Already tracked:** ISSUES.md lines 185-186: background, minimized and post-quit operation deferred; this refinement keeps quit equal to stopping the agent

<details><summary>Verifier note</summary>

The core holds. CloseGuard::begin (close_guard.rs:85) enters Decision on every first close or quit request, with no preference. observe (:292) routes CloseRequested through the same gate. exit-guard.tsx renders the 'Stop FOKS Agent and quit?' sheet on every close, even with 0 unsent messages and no mutation in flight. No quit preference exists anywhere. ISSUES.md:185-186 defers only background and post-quit operation, so this refinement does not conflict with it.

Corrections:
- The quoted 'Stop the FOKS agents … 0 unsent messages' text (line 215, not 212) is the native fallback. It appears only before the renderer subscribes or on a repeated Cmd-Q. The primary renderer sheet already omits the unsent line when the count is 0.
- The sheet also warns that quitting stops 'an agent already running when the app opened'. AgentStatus (foks-agent-proto message.rs:440) has no client count. Skipping the sheet could therefore stop an adopted agent that a CLI shares, unless AgentProcessInfo.owned == false counts as a reason to ask.

</details>
