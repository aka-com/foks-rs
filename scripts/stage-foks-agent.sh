#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
target="$(rustc -vV | sed -n 's/^host: //p')"

if [[ -z "$target" ]]; then
  echo "stage-foks-agent: could not determine the Rust host target" >&2
  exit 1
fi

cd "$repository"
cargo build --locked --release -p foks-agent --bin foks-agent
mkdir -p apps/desktop/src-tauri/binaries
cp target/release/foks-agent "apps/desktop/src-tauri/binaries/foks-agent-$target"
