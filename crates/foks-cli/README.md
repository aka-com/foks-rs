# foks-cli

`foks-rs` is the direct, non-interactive standalone client. Every invocation
requires `--state-dir`; it neither reads nor writes AKA state.

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
foks-rs --state-dir /private/client profile add hosted foks.pub:443 \
  --generation current-probe-only \
  --canary-public-key <64-lowercase-hex-characters> \
  --canary-url https://github.com/OWNER/REPOSITORY/releases/download/foks-hosted-compat-current/foks-hosted-capabilities.json
```

`profile apply-canary` remains available for an audited manual refresh. Both
paths verify and persist the complete signed, expiring artifact. Grants require
the exact embedded protocol-metadata digest and a monotonic generation; drift,
metadata mismatch, and unknown capability artifacts revoke every non-probe
capability, while expiry is the fetch-failure fallback.
