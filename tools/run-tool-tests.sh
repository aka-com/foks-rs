#!/usr/bin/env bash
# Runs the Node and Python unit tests of the qualification tools. Tests that
# need lab artifacts (Wasm builds, native vectors) report themselves as
# skipped unless the environment variables they name are set.
# tools/foks-celld-guarded is not included: its suite is bound to the macOS
# lab host (see its README).
set -euo pipefail

tools=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)

for directory in foks-metadata-spike foks-celld-proof foks-schema-cookie-probe foks-sqlite-policy; do
    echo "node --test: tools/$directory"
    (cd "$tools/$directory" && node --test)
done

# Python 3.12 and later fail a discovery that finds no tests, so each
# pattern is listed only for the directories whose files match it.
python_suites=(
    "foks-metadata-spike test*.py"
    "foks-metadata-spike *_test.py"
    "foks-celld-proof *_test.py"
    "foks-schema-cookie-probe test*.py"
    "foks-sqlite-policy test*.py"
    "foks-hosted-transport-check test*.py"
)
for suite in "${python_suites[@]}"; do
    directory=${suite%% *}
    pattern=${suite#* }
    echo "python3 -m unittest ($pattern): tools/$directory"
    (cd "$tools/$directory" && python3 -m unittest discover -p "$pattern")
done
