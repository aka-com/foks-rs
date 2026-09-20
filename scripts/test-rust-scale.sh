#!/usr/bin/env bash
set -euo pipefail

# Run only this scale test; other ignored tests require external services or hardware.
cargo test --locked -j 2 -p foks-agent "$@" incremental_history_at_production_page_size -- --ignored
