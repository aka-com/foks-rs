#!/bin/bash
#
# Prepares a Claude Code on the web container to build and test this
# repository. Three things are missing from a bare container, and each one
# fails late and unhelpfully:
#
#   - Node dependencies, without which `npm run typecheck` reports missing
#     type definitions for 'node' rather than missing packages.
#   - The native libraries `foks-desktop-app` links against. Their absence
#     surfaces one at a time, as a pkg-config failure partway through a build.
#   - A staged foks-agent. The Tauri build script refuses with
#     "resource path binaries/foks-agent-<target> doesn't exist", and the CLI
#     integration tests launch the sibling debug binary directly.
#
# The apt list is the one .github/workflows/foks-desktop.yml installs for its
# Linux job; keep the two together.

set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(dirname "${BASH_SOURCE[0]}")/../..}"

# Probed through pkg-config, which is what the -sys crates' build scripts
# consult: dpkg can list a package whose files are gone, or a version that
# never shipped the .pc file the build needs.
declare -A native_packages=(
  [gtk+-3.0]=libgtk-3-dev
  [libpcsclite]=libpcsclite-dev
  [webkit2gtk-4.1]=libwebkit2gtk-4.1-dev
)
missing=()
for module in "${!native_packages[@]}"; do
  pkg-config --exists "$module" 2>/dev/null || missing+=("${native_packages[$module]}")
done
# An environment whose network policy blocks apt cannot install the native
# packages. The stub script then stands in for them: cargo can check, lint
# and build the crates, and the agent's own tests run, but foks-desktop-app
# cannot be linked or run. Its exports are needed by every cargo command,
# so they are sourced here for the agent build below and named at the end
# for the commands that follow.
stubbed=""
if [ "${#missing[@]}" -gt 0 ]; then
  echo "session-start: installing ${missing[*]}"
  sudo_prefix=""
  [ "$(id -u)" -eq 0 ] || sudo_prefix="sudo"
  if $sudo_prefix apt-get update --quiet \
    && DEBIAN_FRONTEND=noninteractive $sudo_prefix apt-get install --yes --quiet "${missing[@]}"; then
    :
  else
    echo "session-start: apt could not install ${missing[*]}; using scripts/container-native-stubs.sh instead"
    # Capture first so a fetch or generation failure stops setup instead of
    # being hidden by a successful source of an empty process substitution.
    stub_exports="$(scripts/container-native-stubs.sh)"
    # shellcheck disable=SC1090
    source <(printf '%s\n' "$stub_exports")
    stubbed="yes"
  fi
fi

# Bootstrap the exact npm declared by packageManager without requiring global
# install permissions. The image's npm/npx is used only to obtain that pinned
# CLI; the pinned CLI is the process that interprets package-lock.json.
if [ ! -d node_modules ] || [ package-lock.json -nt node_modules ]; then
  npm_package_manager="$(node -p "JSON.parse(require('fs').readFileSync('package.json', 'utf8')).packageManager")"
  if [[ ! "$npm_package_manager" =~ ^npm@([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
    echo "session-start: packageManager must pin an exact npm version" >&2
    exit 1
  fi
  expected_npm_version="${BASH_REMATCH[1]}"
  pinned_npm=(npx --yes --package="$npm_package_manager" -- npm)
  actual_npm_version="$("${pinned_npm[@]}" --version)"
  if [ "$actual_npm_version" != "$expected_npm_version" ]; then
    echo "session-start: expected npm $expected_npm_version, got $actual_npm_version" >&2
    exit 1
  fi
  echo "session-start: installing Node dependencies with $npm_package_manager"
  "${pinned_npm[@]}" ci --no-audit --no-fund
else
  echo "session-start: Node dependencies are current"
fi

# A debug agent is enough for the tests and builds far faster than the release
# binary `npm run stage-agent` produces. Cargo skips the work when it is fresh.
target="$(rustc -vV | sed -n 's/^host: //p')"
if [ -z "$target" ]; then
  echo "session-start: could not determine the Rust host target" >&2
  exit 1
fi
echo "session-start: building and staging foks-agent for $target"
cargo build --locked -p foks-agent --bin foks-agent
mkdir -p apps/desktop/src-tauri/binaries
cp target/debug/foks-agent "apps/desktop/src-tauri/binaries/foks-agent-$target"

if [ -n "$stubbed" ]; then
  echo "session-start: ready; run cargo commands with the stub exports applied:"
  echo "  source <(scripts/container-native-stubs.sh)"
  echo "  (apply them to cargo only: the LD_LIBRARY_PATH they set breaks Chromium)"
else
  echo "session-start: ready"
fi
