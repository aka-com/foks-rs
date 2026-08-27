#!/bin/sh
set -eu

tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repository=$(CDPATH= cd -- "$tool_dir/../.." && pwd -P)

# The official v0.1.9 oracle validates the deterministic named/ad-hoc team,
# edit, removal, TeamAdmin, and team-KV frames. This remains opt-in because the
# Go module may need to populate its compiler/module cache.
go test -C "$tool_dir" \
    -run '^(TestMutationFixturesAreByteReproducible|TestCheckedUserRPCFixtures)$' \
    -count=1 -v
cargo test --manifest-path "$repository/Cargo.toml" --offline --locked \
    -p foks-server-testkit --test conformance team_
