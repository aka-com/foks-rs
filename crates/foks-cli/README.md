# foks-cli

`foks-rs` is the direct, non-interactive standalone client. Every invocation
requires `--state-dir`; it neither reads nor writes AKA state.

The `mcp kv` and `mcp team` commands expose the account through the resident agent.
See [MCP setup and recovery](../foks-mcp/README.md) for launch configuration, tool
scope, limits and Go compatibility. Install `foks-agent` beside `foks-rs`.

Start a local or hosted profile with:

```text
foks-rs --state-dir /private/client init
foks-rs --state-dir /private/client profile add local localhost:4430 \
  --ca-der /etc/foks/probe-certificate.der
foks-rs --state-dir /private/client profile probe local
```

`init` defaults to macOS Keychain or Linux Secret Service protection and an
external hard-state rollback checkpoint. Isolated tests and explicitly
protected headless automation can request `--key-backend private-file`; that
mode deliberately has no copied-state detection or external rollback
protection.

If the native checkpoint is missing or disagrees with an existing hard-state
database, every checked operation fails closed and prints the exact reset
command form and state directory. That destructive command requires
`--confirm-delete`, removes both the
external checkpoint and hard-state database, and requires the profile to be
probed again. It is not a way to preserve an existing rollback trust history.

Use `--json` for automation. `profile`, `account`, `passphrase`, `kv`, `jobs`,
`device`, `recovery`, and `team` each provide their own `--help`. Signup takes
an optional private `--invite-file` and can take matching `--passphrase-file`
and `--passphrase-confirmation-file` inputs;
the `passphrase` command exposes set, change, and verify. Passphrase inputs must
be private regular files containing one bounded UTF-8 line. Output destinations
for KV downloads and recovery phrases must be new private files; existing
files, symlinks, and permissive secret inputs are rejected.

`profile show NAME --json` returns the exact persisted profile, including its
probe target, so automation does not need to duplicate trust-boundary values.

`team admit-remote` coordinates two already-probed profiles and two active
team records under one canonical dual-profile lock. It persists the remote
binding and removal key before networking, resumes the cross-host mutation
without blind replay, and registers a federation reconciliation job. `team
list-remote` reports the protected bindings; `jobs run-due` also handles their
renewal. No command reads an AKA path or puts a bearer token in SQLite.

Current hosted profiles require an Ed25519 canary public key and the stable
HTTPS URL polled by the agent:

```text
foks-rs --state-dir /private/client profile add hosted foks.app:4430 \
  --generation current-probe-only \
  --canary-public-key <64-lowercase-hex-characters> \
  --canary-url https://github.com/OWNER/REPOSITORY/releases/download/foks-hosted-compat-current/foks-hosted-capabilities.json
```

`profile apply-canary` remains available for an audited manual refresh. Both
paths verify and persist the complete signed, expiring artifact. Grants require
the exact embedded protocol-metadata digest and a monotonic generation; drift,
metadata mismatch, and unknown capability artifacts revoke every non-probe
capability, while expiry is the fetch-failure fallback.

The official hosted RPC endpoint is `foks.app:4430`. `foks.pub:443` is a website
and does not speak the FOKS framed RPC protocol. In the desktop's first-run
chooser, select the existing official CLI account, choose **Use official FOKS
server**, and check the server. The host must match the discovered account.
Choose **Add as a new device**, switch the official CLI to that account with
`foks key switch`, and run `foks --simple-ui key assist`. Paste its code into the
desktop and leave both running until pairing completes. The same handoff is
available under Settings → Connect from FOKS CLI. Keep **Copy this Mac's CLI
device** for deliberate shared-key use; pairing produces a separately revocable
device. Discovery is read-only and does not unlock the Go key store.

The hosted workflow grants `user-sync`, `kv`, `signup`, `device-administration`,
`recovery`, and `passphrases` only after authenticated KV write/read/delete and a
fresh signup, passphrase set/change/verification, pairing between independent
state roots, backup enrollment/recovery, and backup/device revocation all pass.
It also checks account identity, rejected old passphrases, and revoked devices.
`teams` and `federation` are deliberately ungranted: this suite does not exercise
group administration or a second independent host. A grant is compatibility
evidence, not a subscription, invitation, or server-side permission.

Operators must provision these workflow requirements before enabling it:

- `FOKS_CANARY_STATE_TAR_B64`: a private tar.gz of a dedicated initialized
  `--key-backend private-file` state directory, containing a subscribed account
  aliased `canary` under profile `hosted-canary`. Both `hosted-canary` and
  `hosted` must target `foks.app:4430`. The former uses `--generation v019` to
  test through an expired lease; the latter uses `current-probe-only` with the
  signing public key and public stable artifact URL. Probe both and verify the
  server identity before archiving. Do not use a personal production account.
- `FOKS_CANARY_SIGNING_SEED_B64`: the base64-encoded private 32-byte Ed25519
  signing seed matching that public key. Keep it separate from client state.
- Repository variable `FOKS_CANARY_ALLOW_DISPOSABLE_SIGNUP=1`: explicit operator
  consent to create one empty `canary…` server account per successful signup
  attempt. Accounts cannot currently be deleted by this CLI. Their temporary
  credentials are destroyed when the suite exits; server operators must allow
  this traffic and arrange account retention/purging. Failed runs may leave
  additional devices or backup keys on these disposable accounts.
- `FOKS_CANARY_SIGNUP_EMAIL` (variable) and `FOKS_CANARY_SIGNUP_INVITE` (secret),
  if the server requires them. The invite is restored as a private file; it
  must permit repeated signups or be replenished. Email verification, account
  approval, billing, rate limits, and device quotas must permit the exercised
  lifecycle without interactive prompts. Missing requirements produce drift,
  not an unsupported grant. No subscription or billing authorization is
  created automatically by this workflow.
- `FOKS_CANARY_PUBLISH_REPOSITORY` (variable, defaults to this repository) must
  identify a public repository, with the workflow's configured release
  publication credentials. Clients need the matching public stable URL and
  trusted signer. If an old stable artifact was signed for the incorrect
  `foks.pub:443` target, retire that asset before bootstrapping the corrected
  target; generation allocation intentionally rejects cross-target history.

The runner needs Python 3, jq, and the workflow's ordinary shell/hash tools.
Missing consent or a failed onboarding check publishes a signed drift artifact
that removes every non-probe grant; a stopped workflow lets the 48-hour lease
expire. Merely changing this repository does not configure secrets, publish a
lease, or enable capabilities in an installed desktop.

To exercise the lifecycle on a disposable local test server, run
`cargo test -p foks-cli --test onboarding_canary`. The canary's cleanup uses
`device revoke PROFILE --account-alias ACCOUNT_ALIAS DEVICE_ID` and
`recovery revoke PROFILE --account-alias ACCOUNT_ALIAS BACKUP_ALIAS BACKUP_ID`; backup enrollment
JSON includes `backup_id_hex` so cleanup never guesses a credential identity.

Organization enrollment and reauthentication use `foks-rs sso`. See the
[OIDC client flow and operator guide](../foks-server/OIDC.md).

Account rename uses an explicit prepare/confirm flow:

```sh
foks-rs account rename prepare --profile local --account-alias work new_username
foks-rs account rename attempt --profile local --account-alias work --operation OPERATION_ID
foks-rs account rename status --profile local --account-alias work --operation OPERATION_ID
foks-rs account rename list --profile local --account-alias work
```

`cancel` accepts the same operation selector before submission. A later `attempt`
only reconciles a request already submitted; it never sends it again. Keep the original
operation handle if a reply is lost. Hardware accounts accept `--pin-file` on prepare,
attempt and status; without an unlocked key, recovery reports that hardware is needed.
Changing a remote username preserves the local account alias.

`team invite` exposes certificate preview, local and remote requests,
administrator decisions, recovery and removal handling. `account bot` manages
durable enrollment, one-time export, resident loading and revocation. `account
admin` configures an account-bound HTTPS origin and requests a checked handoff
from a host that implements the Go administration methods; the standalone Rust
server returns unsupported for that operation. Use each command's `--help` for
the required profile, account alias and protected-input arguments.

Commands that assign a member role accept `--member-access standard` (the
default) or `--member-access restricted`. These names map to the protocol's
member visibility tiers; raw numeric visibility values are not part of the CLI.
