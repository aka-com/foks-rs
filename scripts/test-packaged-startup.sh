#!/usr/bin/env bash
set -euo pipefail
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repository"
# An optional executable path also supports an extracted Linux package.
executable="${1:-target/release/bundle/macos/foks-desktop.app/Contents/MacOS/foks-desktop}"
test -x "$executable"
"$executable" --smoke-test-packaged-startup
