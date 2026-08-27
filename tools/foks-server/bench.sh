#!/bin/sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

echo "FOKS benchmark revision: $(git rev-parse --short HEAD)"
uname -a

cargo test --offline --locked --release \
    -p foks-server-testkit \
    --test production_benchmarks \
    -- \
    --ignored \
    --nocapture \
    --test-threads=1
