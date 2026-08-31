#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository"

cargo build -p foks-agent --bin foks-agent
cargo build -p foks-desktop --bin foks-desktop-backend

agent_binary="$repository/target/debug/foks-agent"
backend_binary="$repository/target/debug/foks-desktop-backend"
test -x "$agent_binary"
test -x "$backend_binary"

FOKS_AGENT_TEST_BINARY="$agent_binary" \
FOKS_DESKTOP_BACKEND_TEST_BINARY="$backend_binary" \
  cargo test -p foks-server-testkit --test desktop_command_layer \
    sol_process_reentry_and_real_kv_conflict_against_testkit -- --ignored --exact
