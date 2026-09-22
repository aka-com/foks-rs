# foks-compat-artifact

FOKS-only signed capability leases emitted by hosted mutation/read canaries.
Compatible leases expire within seven days. Drift artifacts are structurally
unable to grant capabilities, so applying one revokes the hosted mutation
surface. Signing seeds are accepted only through private raw-byte files and
never through arguments or environment values.

Schema v2 binds a strictly increasing publication generation, target, pinned
protocol-metadata digest, mutation/read results, expiry, outcome, and complete
capability set into one Ed25519 signature. A signing key must never reuse a
generation for different bytes. The hosted workflow authenticates the current
stable artifact with the signing key, adds a unique workflow run/attempt
allocation to its generation, and refuses to publish if that stable input
changes before replacement. Signing and local lease application must both
succeed before an artifact can leave the signing job. Third-party actions and
compilation run in an unprivileged build job before the audited tools are
transferred to the signing job; that job contains no action steps and has only
read permissions. Attestation and release publication run in another job which
receives only the public signed bytes, never the signing seed or canary state.
If the source repository is private, set `FOKS_CANARY_PUBLISH_REPOSITORY` to a
public, dedicated artifact repository and provide its narrowly scoped
`FOKS_CANARY_PUBLISH_TOKEN`; the token is exposed only to the publication job.
Clients authenticate the public bytes with the profile-pinned signing key.

This crate has no dependency outside the FOKS workspace and is excluded from
default workspace builds.
