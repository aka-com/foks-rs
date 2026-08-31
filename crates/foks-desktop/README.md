# foks-desktop

`foks-desktop` is the renderer-independent command library and scripted backend
used by the Tauri desktop. It retains `foks-desktop-backend` as a JSON shell for
protocol transcripts, while `DesktopModel` keeps operation-building logic unit
testable. It does not provide the product executable; the only shipping desktop
binary is `foks-tauri`'s `foks-desktop`. Agent calls in that app run on blocking
workers, never the webview thread. The scripted backend accepts secrets only
through private files, never process arguments.

For the Sol first-run acceptance transcript, invoke the backend once per line;
the process may exit between every line. `pending` is the authenticated,
nonsecret checkpoint read used to decide whether to choose the adjacent resume
command. Secrets are private mode-0600 files, never arguments or checkpoint
fields:

```text
foks-desktop-backend --agent-socket SOCKET initialize
foks-desktop-backend --agent-socket SOCKET check-profile work foks.example
foks-desktop-backend --agent-socket SOCKET pending work
foks-desktop-backend --agent-socket SOCKET create-account work personal --username sol --device-name laptop
foks-desktop-backend --agent-socket SOCKET resume-account work personal
foks-desktop-backend --agent-socket SOCKET backup-prepare work personal paper
foks-desktop-backend --agent-socket SOCKET backup-commit work personal paper --phrase-file PRIVATE_FILE
foks-desktop-backend --agent-socket SOCKET recover-account work recovered --phrase-file PRIVATE_FILE --device-name laptop
foks-desktop-backend --agent-socket SOCKET resume-recovery work recovered --phrase-file PRIVATE_FILE --device-name laptop
foks-desktop-backend --agent-socket SOCKET discover-teams work personal
foks-desktop-backend --agent-socket SOCKET team-create work personal engineering --kind named --name Engineering
foks-desktop-backend --agent-socket SOCKET team-resume work engineering
```

For a private local testkit state, bootstrap explicitly and bind the profile
to the testkit CA. Production defaults remain the native credential backend
and Web PKI. Protocol v2 accepts one explicit trust-root path, which is enough
for the testkit's single CA; the backend requires that path to resolve to one
nonempty regular UTF-8-named file of at most 1 MiB before sending it:

```text
foks-desktop-backend --agent-socket SOCKET initialize --credential-backend private-file
foks-desktop-backend --agent-socket SOCKET check-profile testkit 127.0.0.1:4433 --certificate-der TESTKIT_CA.der
```

New recovery creates a positive serial from OS randomness inside the process.
Resume uses the serial retained by the agent's authenticated pending record.
The current automated backend test verifies that every line maps independently
to its exact typed operation, but it does **not** start `foks-server-testkit`,
spawn a real agent/backend process for each line, or inject an interruption at
every mutation boundary. The plan's process-exit/resume acceptance gate remains
open until that runnable integration harness exists; the transcript above is
not evidence that the end-to-end gate ran.

The scripted boundary includes commands for passive server status, explicit probe,
profile add/forget, device and backup-enrollment lists, typed device removal,
and reset describe/execute. Reset execution reads the one-use token from a
private file rather than an argument. Existing `device-pair-offer`,
`device-pair-republish`, `device-pair-finish`, `device-pair-accept`, and
`device-pair-resume-accept` commands cover every pairing transition; secret
phrases also enter through private files.

Ade's security-key transcript can likewise be split across independent backend
processes. PINs, PUKs, passphrases, and invites are private files; the physical
card serial is selected from `yubi-cards`, while the separate FOKS device
serial used by provisioning is generated inside Rust:

```text
foks-desktop-backend --agent-socket SOCKET yubi-cards work
foks-desktop-backend --agent-socket SOCKET yubi-accounts work
foks-desktop-backend --agent-socket SOCKET yubi-create work work_key --username rae --device-name "YubiKey 42" --card-serial 42 --pin-file PIN --puk-file PUK
foks-desktop-backend --agent-socket SOCKET pending work
foks-desktop-backend --agent-socket SOCKET yubi-resume work work_key --pin-file PIN
foks-desktop-backend --agent-socket SOCKET yubi-provision work personal spare_key --device-name "YubiKey 43" --card-serial 43 --pin-file PIN --puk-file PUK
foks-desktop-backend --agent-socket SOCKET yubi-sync work work_key --pin-file PIN --with-federation
foks-desktop-backend --agent-socket SOCKET yubi-passphrase-set work work_key --pin-file PIN --passphrase-file PASSPHRASE --passphrase-confirmation-file PASSPHRASE_CONFIRMATION
foks-desktop-backend --agent-socket SOCKET yubi-passphrase-verify work work_key --pin-file PIN --passphrase-file PASSPHRASE
foks-desktop-backend --agent-socket SOCKET yubi-revoke work work_key personal --confirm-alias work_key
```

The automated backend test checks exact operation conversion, private-file
reading, and a generated positive provisioning serial. Like the Sol transcript,
this is not yet a hardware/testkit process integration test and does not claim
that a physical card was exercised in CI.

The vault transcript accepts either a local account selector or the complete
authenticated team-store identity. Team selection adds both `--team-alias` and
`--team-id`; the id must be the lowercase named-team (`03`) or ad-hoc-team
(`14`) entity id retained from the authenticated catalog. Reveal always names
the exact version:

```text
foks-desktop-backend --agent-socket SOCKET kv-read work personal /logins/github.com 9
foks-desktop-backend --agent-socket SOCKET kv-read work personal /wifi/password 19 --team-alias household --team-id TEAM_ID_HEX
```

Scripted writes take plaintext only from private regular files. Text and link
creates carry `Create`; edits, replacements, and nonrecursive removes require
the inspected `ExactVersion`. Existing read/write roles are explicit on edit
and replacement as `owner`, `admin`, or `member:VISIBILITY` so the transcript
does not silently replace catalog facts:

```text
foks-desktop-backend --agent-socket SOCKET kv-create-text work personal /password --value-file PRIVATE_VALUE
foks-desktop-backend --agent-socket SOCKET kv-create-link work personal /current --target-file PRIVATE_TARGET
foks-desktop-backend --agent-socket SOCKET kv-edit-text work personal /password 7 --read-role member:-1 --write-role admin --value-file PRIVATE_VALUE
foks-desktop-backend --agent-socket SOCKET kv-remove work personal /password 8
foks-desktop-backend --agent-socket SOCKET kv-create-file work personal /bundle.tar --source-file PRIVATE_FILE
foks-desktop-backend --agent-socket SOCKET kv-replace-file work personal /bundle.tar 11 --read-role member:-16384 --write-role admin --source-file PRIVATE_FILE
```

`tests/backend_transcript.rs` executes the real backend process against a
private scripted Unix agent. It proves one version-bound team reveal request,
the create/CAS/remove guards, no retry after a forced conflict, and an 84 MiB
sparse file carried only as bounded `put_kv_stream` frames rather than inline
content. Native drag delivery and webview resident-set measurement remain
shipping-platform acceptance checks rather than properties this backend shell
can synthesize.

The Tauri product architecture stays on the
native FOKS hierarchy: profiles contain accounts and their personal stores; accounts can
join or create teams, whose stores and party rosters retain their authenticated team
identity. The client may aggregate those stores into one Items view, but does not add a
generic container or a device-local content source. See
[`dev/foks-desktop/README.md`](../../dev/foks-desktop/README.md) for the design contract and
implementation stages.

The target local boundary is protocol v2: the desktop launches a detached agent that
survives window closure; the agent can start in a restricted bootstrap mode, which the
desktop drives behind a full-window blocking state, and then becomes ready without
exposing credential storage to the GUI. Up to four catalog reads run concurrently;
mutations are single-flight. Account and team stores both support paged metadata listing,
large files use version-bound chunks, item read/write roles are visible, and every edit is
compare-and-swap guarded.

Items now implements that contract. Values remain masked until Reveal, large
values are fetched in exact-version chunks, and team stores expose no mutation
controls. New account-store files, folders, and symlinks default both native
roles to Owner. File and symlink replacement preserves the roles displayed by
the catalog, while replacement and removal send the displayed version as a
mandatory compare-and-swap precondition. Values above the safe inline frame
bound use the same-socket upload automatically without a plaintext staging
file; ambiguous post-commit failures trigger a catalog refresh rather than an
automatic retry.

Parties can create native named or ad-hoc teams and resume an interrupted
creation from its local team alias. Named teams take the FOKS-visible name;
ad-hoc teams deliberately do not. Rename and closure remain absent because the
current FOKS application contract does not provide them.

Settings exposes the existing FOKS owner-device and recovery workflows through
local protocol v2: list and provision software owner devices, resume an
interrupted provision, prepare and commit an owner backup, and recover or resume recovery
to a new owner device. Owner backup is a two-step prepare/commit flow; its
generated phrase is returned once for offline storage and is not persisted
before confirmation. YubiKey passphrase set, change, and verification use the same
hardware-backed client flows as the CLI; all PIN, passphrase, and recovery
phrase inputs use bounded secret fields and clear only after a valid request is
dispatched.

For a managed state directory, the desktop resolves `foks-agent` beside its own packaged
executable (or from explicit `--agent-binary`), launches it only when no authenticated
socket is available, and then polls `AgentStatus`. The Tauri shell remains mounted but
input-blocked while the agent is connecting or initializing. First run then
uses one atomic checked-publication operation for a v0.1.9-compatible FOKS
profile and can create or explicitly resume an account.
`foks-desktop-backend` exposes each first-run and resume operation as an
independent process invocation, including backup prepare/commit, recovery,
authenticated team discovery, and group create/resume. Recovery serials are
generated inside the Rust process and never accepted as command arguments;
secret inputs come only from private bounded files. There is no device-local
item-store branch; device pairing is exposed by both the backend transcript
surface and the Tauri command boundary.

The Teams screen can admit a remote profile's active team into a selected
local named team, list protected remote bindings, and surface action-required
reconciliation failures. The graphical path deliberately defaults to the
member role; explicit admin/owner selection remains available in the CLI and
typed agent protocol.

Keeping this crate separate prevents Tauri and platform webview dependencies
from entering FOKS protocol, storage, or agent crates. Onboarding and passphrase
operations cross only the private typed agent boundary; the desktop never opens
credentials or SQLite directly.

macOS and Linux use private Unix sockets. The shipping Tauri binary owns the
platform webview integration and managed-agent placement. Packaging, signing,
updates, and local redacted crash markers are specified in
`packaging/foks-desktop/RELEASE_POLICY.md`. Windows is not a target.
