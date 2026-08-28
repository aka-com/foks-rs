# foks-compat-artifact

FOKS-only signed capability leases emitted by hosted mutation/read canaries.
Compatible leases expire within seven days. Drift artifacts are structurally
unable to grant capabilities, so applying one revokes the hosted mutation
surface. Signing seeds are accepted only through private raw-byte files and
never through arguments or environment values.

Schema v2 binds a strictly increasing publication generation, target, pinned
protocol-metadata digest, mutation/read results, expiry, outcome, and complete
capability set into one Ed25519 signature. A signing key must never reuse a
generation for different bytes. The hosted workflow derives generations from
the monotonic workflow run number and attempt, attests each artifact, and
replaces one stable prerelease asset only after signing. If the source
repository is private, set `FOKS_CANARY_PUBLISH_REPOSITORY` to a public,
dedicated artifact repository and provide its narrowly scoped
`FOKS_CANARY_PUBLISH_TOKEN`; the token is exposed only to the publication step.
Clients authenticate the public bytes with the profile-pinned signing key.

This crate has no AKA dependency and is excluded from default workspace builds.
