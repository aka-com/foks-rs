# `foks-tauri` — the FOKS desktop application

The Tauri app for FOKS, beside `src-tauri`'s AKA: the native agent, catalog,
exact-version read, app-lock, clipboard, and download boundary behind the React
product shell, plus guarded writes and native streaming file ingress.

The two apps share the Cargo workspace, its single `Cargo.lock`, and the
pnpm/Vite toolchain. They share **nothing else** — not the bundle identifier,
the entitlements, the single-instance identity, the release train, the capability
set or the command surface.

- Rust crate: `foks-desktop-app`, binary `foks-desktop`. The crate named
  `foks-desktop` provides the free command/library surface plus the
  `foks-desktop-backend` transcript binary.
- Web app: `../foks-ui`, served on `http://127.0.0.1:1421` in dev and built to
  `../foks-ui/dist`.
- Bundle identifier: `org.foks.desktop` (matching packaging/foks-desktop/macos/Info.plist and the polkit action prefix; the AppStream component id stays `org.foks.Desktop`).

## Two decisions frozen in `tauri.conf.json`

- **`dragDropEnabled: true`.** The runtime intercepts OS drag-drop and hands
  Rust the dropped file **paths**; HTML5 drag and drop is suppressed inside the
  window in exchange. `src/dragdrop.rs` forwards them to the webview as
  `foks://drop-paths` (a `string[]`) and the hover state as `foks://drop-hover`
  (`{ hovering: boolean }`). This is a fork, not a flag: with it `false` the
  webview would get a pathless `File` and the only upload route would be reading
  the bytes into the renderer, which the secret-handling policy forbids.
- **`withGlobalTauri: false`.** The IPC surface is not published on
  `window.__TAURI__`. The FOKS bridge imports `@tauri-apps/api/core` instead.
  AKA sets this `true`; FOKS renders secrets and does not.

## Running it

The Tauri CLI picks its project directory by finding `tauri.conf.json` in the
current directory, so every command **runs from `foks-tauri/`**. Passing
`--config foks-tauri/tauri.conf.json` from the repository root does *not* work:
`-c` merges a config file with the one the CLI already discovered, and from the
root it discovers `src-tauri/` — you would build AKA's crate with FOKS's
settings. The pnpm scripts do the `cd` for you.

    pnpm run foks:dev          # local server + agent + Tauri development app
    pnpm run foks:dev:tauri    # Tauri only; runs `foks:dev:frontend` for the web side
    pnpm run foks:build        # production frontend plus release desktop executable
    pnpm run foks:bundle:deb   # staged-agent Linux .deb (see linux/README.md)

`beforeDevCommand` and `beforeBuildCommand` run from the repository root — the
nearest ancestor with a `package.json` — so they are plain root pnpm scripts.
The Tauri hooks use `foks:dev:frontend` and `foks:build:frontend`; this avoids
recursively invoking the aggregate native commands from Tauri.

Linux build dependencies: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libpcsclite-dev`, and `librsvg2-bin` for bundling. The managed-agent sidecar
must be staged under its target-triple-qualified name first; the Linux README
contains the exact command.

## Bazel and release packaging

`//foks-tauri:foks-desktop` embeds `//foks-ui:dir` and carries the macOS WebKit
link flag. `//foks-tauri:FOKS_app` assembles an unsigned development app with
the agent at `Contents/Helpers/foks-agent`. The release-only
`//foks-tauri:FOKS_app_release_signed` chain signs the helper and outer app and
fails closed when a Developer ID identity is absent. Release CI notarizes that
app as an Apple-metadata-preserving ZIP, staples it, and publishes the rebuilt
ZIP; FOKS does not ship a DMG. The private/manual
`FOKS_zip_notary_submission` target is only an input to notarization and is
deliberately not named or exposed as a release artifact: the downloadable ZIP
must be recreated from the stapled app.

Linux release packaging stays on Tauri's native `.deb` bundler because it owns
the control file, desktop entry, sidecar placement, and polkit mapping. The
release workflow also creates SHA-256 checksums and build-provenance
attestations. See
`../packaging/foks-desktop/RELEASE_POLICY.md` for supported architectures and
external release gates.

## Tests and checks

    cargo check -p foks-desktop-app
    cargo test -p foks-desktop-app
    cargo fmt -p foks-desktop-app -- --check
    cargo clippy -p foks-desktop-app --all-targets

The crate is a workspace **member but not a default member** (see the comment in
the root `Cargo.toml`): `cargo build`/`cargo test` with no `-p` skip it, because
building it needs the platform webview toolchain. Do not run
`cargo test --workspace` on a machine without WebKitGTK.

The unit tests also cover command wire goldens, catalog validation, the
single-flight mutation gate, exact CAS transcripts, one-use dropped paths,
private managed-agent state, clipboard digest tracking, and version-bound file
streaming in both directions. The upload test passes an 84 MiB sparse file
through the bounded reader and asserts that no inline operation contains it.

## Phase 2 command boundary

`list_catalog` returns the complete typed catalog snapshot and retains its
exact metadata in Rust. `read_item`, `copy_item_value`, `copy_item_path`, and
`download_file` accept the opaque store id returned by that snapshot plus path
and version; Rust refuses anything not present at that exact version. Only
`read_item` returns a value to the webview. Copy writes directly to the system
clipboard, while Download writes version-bound chunks to a temporary file next
to the chosen destination and atomically places the completed file.

On a platform with an available OS authenticator, the managed app lock starts
armed before the webview can load agent or catalog state. The renderer decodes
`app_lock_state` first and boots only after `unlock_app` reports unlocked. If
the packaged authenticator is unavailable, Rust leaves the lock unarmed and
reports the reason rather than stranding the application behind it.

Catalog/store loads cancel the preceding load and use a generation check so an
older completion cannot replace newer state. Call `list_catalog` once and
derive its stores; do not issue `list_stores` and `list_catalog` concurrently.
`wire-contract.json` is the shared serialization golden.

`list_servers` is deliberately passive. The current agent's `ListProfiles`
response contains only a profile name; it has no read-only last-probe or lease
status operation. FOKS therefore returns null host/chain/epoch/lease facts and
does not silently call `Probe`, because Probe can insert or advance trust.
Catalog failures and `blockedProfiles` are the honest typed Alerts source.

## Phase 3 mutation boundary

Account-vault and active-group creates use `KvPrecondition::Create`; edits,
file replacements, and removes are built from the catalog item and use
`KvPrecondition::ExactVersion`. Every mutation is single-flight, clears the
cached catalog before touching the agent, and requires a fresh catalog after an
ambiguous outcome. A create conflict is `already-exists`; a guarded conflict is
`conflict`. The protocol's typed `CapabilityDenied(kv)` becomes the honest
`capability-unavailable`, not “lease lapsed,” because no current read operation
proves the cause. Catalog-blocked profiles and inactive groups fail before the
transport is touched; inactive groups return `inactive-group` and expose the
explicit resumable creation operation.

The active-group create commands require explicit native `readRole` and
`writeRole` strings: `Owner`, `Admin`, or `Member:<signed-i16>`. Account creates
omit both and remain Owner/Owner. `create_text_item`, `create_link` (the
protocol's symlink node), `create_folder`, `import_dropped_file`, and
`pick_and_import_file` all use that contract. Supplying only one role or trying
to override an account role fails closed. Edit and replacement commands accept
no role arguments: they preserve the authenticated roles retained with the
exact catalog version.

Native drops send path strings, never bytes. Rust authorizes one UTF-8 path only
when the native event contains exactly one file; multi-drop paths are emitted
so the UI can explain its refusal but none can be consumed by a direct invoke.
It opens only a regular non-symlink file and streams the opened handle through
`put_kv_stream` on a blocking worker. The native
picker follows the same path without disclosing its source path to the
webview. File replacement retains the selected item's roles and exact version.
The v0.1.9 client encodes files of at most 2,040 bytes as `small-file` nodes;
native replacement accepts both that inline encoding and chunked `file` nodes.
Folder creation is live, while folder removal remains deliberately absent
because the renderer boundary never authorizes recursive deletion. Link
creation is live; link editing remains unavailable because the desktop
builder requires links to be removed and recreated rather than presenting
that pair as an atomic update.

`take_agent_connection_loss` is the passive observation used for the
full-window stop. `retry_agent_connection` validates/restarts the managed agent
on a blocking worker only when the current connection is unhealthy; it does
not restart a healthy agent. It returns the normal agent-status shape; the web
app reloads the catalog only after that succeeds.

## Phase 4 group boundary

`list_accounts`, `list_parties`, and `list_federation` expose the agent's
account, roster, and federation facts without synthesizing missing identities.
The available-account projection must match the retained catalog exactly;
catalog-blocked profiles remain visible as servers but are never queried for
account identities. Roster reads
accept only user, named-group, and ad-hoc-group party kinds; federation reads
accept only the protocol's Member destination. A malformed row rejects the
whole response. Valid responses are retained under the catalog generation that
authorized them, and a refresh or any mutation clears them so a late read
cannot restore stale authorization facts.

Group mutations resolve account, local group, and remote group identities from
that retained state. Catalog-blocked profiles and inactive groups fail before
the transport is touched. Member demotion and removal additionally require a
unique roster row with a username, `locally_manageable: true`, user party kind,
and an account identity proving it is not the current user. Demotion accepts
only a strictly lower role or Member visibility band; the underlying operation
removes and re-adds the party and rekeys. Member and federation operations are
available only for named groups, matching the client operation's own boundary.

The commands return only `{ applied: true }`; callers reload catalog, account,
roster, and federation facts after success and never replay an ambiguous
change. Explicit resume commands wrap the agent's journaled group creation,
member addition, and member edit operations. Re-running an inactive federation
admission selects one retained 16-byte lowercase-hex operation id and reissues
`AdmitFederatedTeam` with the retained remote identity and Member visibility.
The renderer cannot supply a replacement identity. There is deliberately no
promotion, un-admit, or send-invite command. Invite text is composed in the UI
and copied through the bounded `copy_text` command, which keeps clipboard
hygiene and the main-window check.

Member demotion and removal select the authenticated user party id retained
from the roster, not a display username. Admitted groups and scoped user rows
therefore do not make an otherwise actionable local user ambiguous. The
renderer cannot substitute a different party identity for the selected row.

## Phase 5 first-run boundary

First run uses the same `main` window and app lock. Rust initializes only the
native credential backend. A server check first compares the complete
normalized host-and-port against saved profiles. One match is checked through
that profile's stored protocol and trust root; multiple matches fail closed.
With no match, Rust atomically checks and adds a v0.1.9/WebPKI profile. It
returns the actual selected profile and the accepted host identity and chain
facts. A failed new-profile check never leaves a profile behind. Account
creation, passphrase protection, backup commit, recovery, discovery, and group
creation are single-flight operations. Every synchronous transport call stays
on a blocking worker. Their success reports are exact-shape and request-bound
before Rust returns `{ applied: true }`; a malformed post-mutation success is
treated as an ambiguous fatal boundary failure and blocks another write until
refresh.

`list_pending_operations` returns bounded, authenticated, nonsecret resume
selectors. Account-signup and recovery resume commands re-read that list and
require one exact kind/alias match before issuing the operation. No secret is
stored in a renderer checkpoint. The first-run navigation checkpoint remains
the versioned pure TypeScript state machine; facts and resumability are always
re-derived from the agent after restart.

Every profile preflight decodes the agent's complete persisted `Profile`
record—name, probe, protocol policy, and trust root—rather than a name-only
projection. The canonical serialized shape must match exactly, every profile
must pass the same name/probe/policy/trust coherence rules as the client, and
duplicates or oversized lists reject the entire response.

Owner backup is deliberately two-step. `prepare_owner_backup` is an
app-locked, read-only reveal that returns the phrase once without invalidating
catalog state; `commit_owner_backup` sends the confirmed phrase back to the
agent without persisting it in the desktop. Recovery accepts no serial from
the renderer: Rust generates a positive random serial for a new recovery, and
the agent's authenticated pending record owns it across resume. Team discovery
returns only validated authenticated identities. Pairing uses the separately
bound Phase 6 Start, Accept, Finish, and Resume commands.

## Phase 6 server, device, pairing, and reset boundary

The Phase 6 Rust boundary is live; the corresponding web screens are wired in
their own track. `describe_server_status` is passive: it returns the configured
probe, a retained host identity/chain/tree snapshot when available, whether the
pinned protocol requires a compatibility lease, and that lease's own expiry
timestamp. Tauri verifies the requirement against the independently listed
profile policy. It never calls `Probe` and cannot invent checked-at,
trusted-since, reachability, or latency facts. A profile
whose rollback checkpoint cannot be verified remains blocked; status does not
bypass that verification merely to keep old facts visible. `check_server` is
the explicit network operation and accepts the agent's exact inserted,
advanced, or unchanged verdict. First run accepts the same three verdicts when
it reuses a saved profile, while a newly published profile must still report
inserted.

`add_server` writes only a v0.1.9/Web-PKI profile record and does not contact
the server. Removing that record with `forget_server` leaves the profile's
durable directory, credentials, and checkpoint state on disk, so adding the
same local name reconnects it. Forget requires the exact profile name and an
exact `{ profile, removed: true }` response.

Device and backup reads resolve an opaque account-store id against the retained
catalog, then reject unknown, duplicate, excessive, or malformed agent rows.
The device wire fact is id, an optional authenticated name, role, and whether it
is current. A missing name remains a generic **device**; the desktop never
invents a Mac or hardware label. Names are bounded single-line agent responses.
Entity ids themselves remain typed: the boundary accepts exact software and
YubiKey ids in the list, while software-device removal requires a fresh retained
list and only permits a non-current software id. Current, YubiKey, and unknown
ids are refused before transport.

Pairing exposes Start, Republish/Resume offer, Finish, Accept, and Resume
acceptance. Rust generates the positive acceptance serial; the renderer never
supplies it. Pairing phrases are validated as FOKS KEX phrases, zeroized on
drop, redacted from `Debug`, and never cached in `AppState`. Set, change, and
verify passphrase commands use their distinct agent operations; change does
not invent a current-passphrase argument that the operation does not accept.

Reset is two calls. `describe_reset` returns the exact bounded resumables and
artifact counts plus the agent's short-lived one-use token. `reset_server`
requires both that token and an exact typed profile-name confirmation. The
agent binds the token to the described state, consumes it once, and refuses
drift, expiry, reuse, or a different profile. The token is zeroized, redacted,
and never retained in `AppState`. A malformed success after reset is treated
as fatal post-mutation ambiguity and forces a refresh before another change.

Security keys expose the complete agent lifecycle rather than collapsing it
into a generic device edit: connected-card and local-enrollment lists; create,
provision, and journal-bound resume; sync; PIN/PUK and passphrase operations;
management-key rotation/resume/recovery; subkey recovery; and revocation. The
enrollment list now preserves the protocol's `pending` versus `complete` fact.
It still does not claim which connected serial belongs to which local alias,
and the connected-card model name is validated but deliberately not exposed.
Creating or provisioning requires one currently listed positive card serial,
two distinct retired PIV slots, a bounded unlock code, and positive
retry counts. Provisioning's FOKS software-device serial is generated in Rust.

Every hardware secret is bounded, moved into `SecretString`, and kept out of
`AppState`; every operation re-resolves the profile and enrollment state on a
blocking worker. Revocation requires the exact alias as confirmation, a
complete local enrollment, and a software account whose authenticated current
device is software—so a security key cannot be asked to sign its own
revocation. No key status, friendly device name, or alias-to-card mapping is
inferred. Malformed success after any hardware mutation is fatal ambiguity,
not a success toast or automatic replay.

## Layout

| File | What it is |
| --- | --- |
| `src/lib.rs` | `run()`: tracing, plugins, managed state, the setup hook, and the configuration invariant tests. |
| `src/agent.rs` | Socket resolution, managed launch, hardened state/log files, advisory spawn lock, connection-loss observation, and command error mapping. |
| `src/applock.rs` | OS-authenticated app lock and the Rust-side read gate; polkit on packaged Linux, LocalAuthentication on macOS. |
| `src/clipboard.rs` | Rust-side copy, digest-only tracking, timed and exit clearing, and macOS concealment. |
| `src/commands.rs` | Catalog/read/copy/download, guarded mutation and native file-stream commands, roster/federation, passive server, and first-run commands; all synchronous transport work runs in `spawn_blocking`. |
| `src/startup.rs` | `fatal_startup`: a blocking native dialog and a non-zero exit instead of a crash. |
| `src/dragdrop.rs` | The native drop path. |
| `capabilities/default.json` | The audited webview permission set. |
| `PERMISSIONS.md` | What that set actually resolves to. Read it before adding a permission. |
| `linux/` | The polkit policy the app lock needs, the `.deb`'s desktop-entry template and AppStream metadata, and the rationale for all three. |
| `icons/` | Placeholders. See `icons/README.md`. |

## Remaining release inputs

The Bazel and Tauri package graphs now place the validated managed agent beside
the desktop executable. Crash markers are installed only for the default
managed private state directory; an explicit `--agent-socket` or
`FOKS_AGENT_SOCKET` never authorizes writes beside that externally owned
socket. Crash upload remains disabled.

Publisher and homepage metadata remain unset until the release owner supplies
canonical values. Signing, notarization, graphical installed-package testing,
and real security-key hardware checks are external release gates rather than
facts a developer build can claim.
