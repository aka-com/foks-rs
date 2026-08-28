#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cargo run --locked --manifest-path "$repository/Cargo.toml" \
  -p foks-server --example host_rotation_fixtures -- "$work/rust"

for phase in host-add host-revoke; do
  mkdir "$work/$phase"
  (
    cd "$repository/tools/foks-v019-oracle"
    go run . \
      --host localhost:4430 \
      --probe-file "$work/rust/$phase.probe.snowp" \
      --out "$work/$phase"
  )
done

echo "pinned Go v0.1.9 accepted Rust host-key add and revoke chains"
