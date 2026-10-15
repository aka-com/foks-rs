#!/usr/bin/env bash
# Verifies that release executables carry the dependency list that
# `cargo auditable` embeds (see scripts/foks-cargo.sh). Arguments are pairs of
# an executable path and the Cargo package that builds it:
#
#   scripts/verify-auditable-binaries.sh path/to/foks-agent foks-agent ...
#
# Each executable must contain an embedded list whose root package is the named
# package. `cargo audit bin <executable>` reads the same list.
set -euo pipefail

if (( $# == 0 || $# % 2 != 0 )); then
  echo "usage: verify-auditable-binaries.sh <executable> <package> [<executable> <package> ...]" >&2
  exit 2
fi

if ! command -v rust-audit-info >/dev/null 2>&1; then
  echo "verify-auditable-binaries: rust-audit-info is required (cargo install --locked rust-audit-info@0.5.4)" >&2
  exit 1
fi

while (( $# > 0 )); do
  executable=$1
  package=$2
  shift 2
  if [[ ! -f "$executable" ]]; then
    echo "verify-auditable-binaries: $executable does not exist" >&2
    exit 1
  fi
  if ! info="$(rust-audit-info "$executable")"; then
    echo "verify-auditable-binaries: $executable has no embedded dependency list" >&2
    exit 1
  fi
  python3 -c '
import json
import sys

executable, package = sys.argv[1], sys.argv[2]
packages = json.loads(sys.stdin.read())["packages"]
roots = sorted(entry["name"] for entry in packages if entry.get("root"))
if roots != [package]:
    raise SystemExit(f"verify-auditable-binaries: {executable} lists root package(s) {roots}, expected [{package!r}]")
print(f"{executable}: {len(packages)} packages embedded, root {package}")
' "$executable" "$package" <<<"$info"
done
