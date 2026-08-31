#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
state_root="${FOKS_DEV_ROOT:-${TMPDIR:-/tmp}/aka-foks-dev-${UID}}"
server_pid=""
agent_pid=""

usage() {
  cat <<'EOF'
Usage: scripts/foks-dev.sh

Build and run a local FOKS server, agent, and desktop application.

Environment:
  FOKS_DEV_ROOT  Persistent server and client state directory.
                 Defaults to $TMPDIR/aka-foks-dev-$UID.
EOF
}

fail() {
  echo "foks-dev: $*" >&2
  exit 1
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "$1 is required"
  fi
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [[ -n "$agent_pid" ]] && kill -0 "$agent_pid" 2>/dev/null; then
    kill -TERM "$agent_pid" 2>/dev/null
  fi
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill -TERM "$server_pid" 2>/dev/null
  fi
  if [[ -n "$agent_pid" ]]; then
    wait "$agent_pid" 2>/dev/null
  fi
  if [[ -n "$server_pid" ]]; then
    wait "$server_pid" 2>/dev/null
  fi
  return "$status"
}

wait_for_server() {
  local deadline=$((SECONDS + 30))
  while ! curl --connect-timeout 1 --max-time 1 --fail --silent --output /dev/null \
    http://127.0.0.1:9090/readyz; do
    if ! kill -0 "$server_pid" 2>/dev/null; then
      wait "$server_pid" || true
      fail "the local server exited before becoming ready"
    fi
    if ((SECONDS >= deadline)); then
      fail "timed out waiting for the local server at http://127.0.0.1:9090/readyz"
    fi
    sleep 0.2
  done
}

wait_for_agent() {
  local socket=$1
  local log=$2
  local deadline=$((SECONDS + 30))
  while ! grep --fixed-strings --quiet "FOKS agent ready: $socket" "$log"; do
    if ! kill -0 "$agent_pid" 2>/dev/null; then
      wait "$agent_pid" || true
      sed 's/^/foks-dev: agent: /' "$log" >&2
      fail "the local agent exited before becoming ready"
    fi
    if ((SECONDS >= deadline)); then
      fail "timed out waiting for the local agent socket at $socket"
    fi
    sleep 0.2
  done
}

if [[ $# -gt 0 ]]; then
  if [[ $# -eq 1 && ("$1" == "-h" || "$1" == "--help") ]]; then
    usage
    exit 0
  fi
  usage >&2
  exit 2
fi

require_command cargo
require_command curl
require_command pnpm

umask 077
mkdir -p "$state_root"
state_root="$(cd "$state_root" && pwd -P)"
chmod 700 "$state_root"

server_dir="$state_root/server"
client_dir="$state_root/client"
server_config="$server_dir/server.toml"
server_certificate="$server_dir/probe-certificate.der"
agent_socket="$client_dir/agent.sock"
agent_log="$state_root/agent.log"

cd "$repo_root"
echo "Building the FOKS server, agent, and command-line client..."
cargo build \
  -p foks-server --bin foks-server \
  -p foks-agent --bin foks-agent \
  -p foks-cli --bin foks-rs

server_binary="$repo_root/target/debug/foks-server"
agent_binary="$repo_root/target/debug/foks-agent"
client_binary="$repo_root/target/debug/foks-rs"

if [[ ! -f "$server_config" ]]; then
  if [[ -e "$server_dir" ]]; then
    fail "$server_dir exists without server.toml; move it aside or choose another FOKS_DEV_ROOT"
  fi
  echo "Initializing the local FOKS server in $server_dir..."
  "$server_binary" init --directory "$server_dir" --canonical-name localhost
fi

if [[ ! -f "$client_dir/client-state.toml" ]]; then
  echo "Initializing the local FOKS client in $client_dir..."
  "$client_binary" --state-dir "$client_dir" init --key-backend private-file
fi

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

echo "Starting the local FOKS server..."
"$server_binary" serve-config --config "$server_config" &
server_pid=$!
wait_for_server

profile_error=""
if profile_error=$("$client_binary" --state-dir "$client_dir" profile show local 2>&1); then
  if ! profile_error=$("$client_binary" --state-dir "$client_dir" profile verify \
    local localhost:4430 --ca-der "$server_certificate" 2>&1); then
    fail "the existing local profile is not app-managed: $profile_error"
  fi
elif [[ "$profile_error" == *"FOKS profile is missing"* ]]; then
  echo "Adding the pinned local server profile..."
  "$client_binary" --state-dir "$client_dir" profile add \
    local localhost:4430 --ca-der "$server_certificate"
else
  fail "could not inspect the local profile: $profile_error"
fi

echo "Verifying the local server profile..."
"$client_binary" --state-dir "$client_dir" profile probe local

echo "Starting the local FOKS agent..."
: >"$agent_log"
"$agent_binary" --state-dir "$client_dir" --socket "$agent_socket" 2>"$agent_log" &
agent_pid=$!
wait_for_agent "$agent_socket" "$agent_log"

echo "Starting the FOKS desktop application. State is stored in $state_root"
echo "Agent diagnostics are written to $agent_log"
FOKS_AGENT_SOCKET="$agent_socket" FOKS_MANAGED_PROFILE=local pnpm run foks:dev:tauri
