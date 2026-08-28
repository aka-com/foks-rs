# foks-compat-artifact

FOKS-only signed capability leases emitted by hosted mutation/read canaries.
Compatible leases expire within seven days. Drift artifacts are structurally
unable to grant capabilities, so applying one revokes the hosted mutation
surface. Signing seeds are accepted only through private raw-byte files and
never through arguments or environment values.

This crate has no AKA dependency and is excluded from default workspace builds.
