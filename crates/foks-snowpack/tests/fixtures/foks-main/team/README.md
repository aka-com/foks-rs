# go-foks main team status fixtures

Generated from go-foks `35dc03766234572551733c0a8cb4ac9aab0f2f11` (unreleased
v0.1.10; module version `v0.1.10-0.20261012000410-35dc03766234`) by
`tools/foks-main-oracle`:

```sh
cd tools/foks-main-oracle
FOKS_MAIN_FIXTURE_OUT=../../crates/foks-snowpack/tests/fixtures/foks-main \
  go test -run TestTeamStatusFixtures -count=1 .
```

Without `FOKS_MAIN_FIXTURE_OUT` the same test compares its output with these
files. Every frame uses sequence number 1. `TestTeamOwnerGuard` in the same
file emits no files; it asserts go-foks main's `GameplanOpts.RequireOwner`
behaviour for a sole-owner self-demotion (refused with
`change would leave team without an owner` when set, accepted when unset, as
in chain replay).

| File | Content |
| --- | --- |
| `vo-bearer-token-not-found-response.frame` | `TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR` (7016, void payload), the reply to a view-only bearer token the server holds no row for (`5a9ff13`) |
| `ownerless-edit-roster-error-response.frame` | `TEAM_ROSTER_ERROR` (7006) with `edit would leave team without an owner`, the server's refusal of an edit that leaves a team without an owner (`df2acf1`) |

`SHA256SUMS` lists each file's SHA-256 in `sha256sum` format.
`crates/foks-rpc/tests/server_responses.rs` checks that the Rust server
encodes both statuses to exactly these bytes and that the client decodes them.
