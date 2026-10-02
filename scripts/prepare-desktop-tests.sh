#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repository"

# Native tests execute on the host. Forward Cargo profile/build options, and
# obtain the executable from Cargo rather than assuming a target directory.
host_triple="$(rustc -vV | sed -n 's/^host: //p')"
test -n "$host_triple"
artifacts="$(mktemp)"
trap 'rm -f "$artifacts"' EXIT
cargo build --locked -j 2 -p foks-agent --bin foks-agent "$@" --message-format=json-render-diagnostics >"$artifacts"
python3 - "$artifacts" "$repository/apps/desktop/src-tauri/binaries/foks-agent-$host_triple" <<'PY'
import json
import pathlib
import shutil
import sys

messages = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines()]
executables = {
    message["executable"]
    for message in messages
    if message.get("reason") == "compiler-artifact"
    and message.get("target", {}).get("name") == "foks-agent"
    and message.get("executable")
}
if len(executables) != 1:
    raise SystemExit("expected exactly one standalone foks-agent build artifact")
destination = pathlib.Path(sys.argv[2])
destination.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(executables.pop(), destination)
PY
