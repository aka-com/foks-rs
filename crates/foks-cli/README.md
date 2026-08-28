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
mode deliberately has no external rollback detection.

Use `--json` for automation. `profile`, `account`, `kv`, `jobs`, `device`,
`recovery`, and `team` each provide their own `--help`. Output destinations for
KV downloads and recovery phrases must be new private files; existing files,
symlinks, and permissive secret inputs are rejected.

Current hosted profiles require an Ed25519 canary public key. `profile
apply-canary` verifies and persists the complete signed, expiring artifact;
drift artifacts revoke every non-probe capability and expired leases fail
closed.
