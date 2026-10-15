#!/usr/bin/env bash
# Runs cargo for builds of the shipped executables. With
# FOKS_AUDITABLE_BUILD=1 it runs `cargo auditable` instead, which embeds the
# resolved dependency list in each executable so that `cargo audit bin` can
# check a shipped binary against advisories published after its release.
#
# stage-foks-agent.sh calls this script for the managed agent, and
# apps/desktop/src-tauri/tauri.conf.json names it as the Tauri build runner for
# foks-desktop. Tauri invokes the runner as `<runner> build ...` (or `run` for
# `tauri dev`), which is why the runner is a script: `cargo-auditable` itself
# only accepts `auditable` as its first argument.
set -euo pipefail

case "${FOKS_AUDITABLE_BUILD:-}" in
  "")
    exec cargo "$@"
    ;;
  1)
    if ! command -v cargo-auditable >/dev/null 2>&1; then
      echo "foks-cargo: FOKS_AUDITABLE_BUILD=1 requires cargo-auditable (cargo install --locked cargo-auditable@0.7.7)" >&2
      exit 1
    fi
    exec cargo auditable "$@"
    ;;
  *)
    echo "foks-cargo: FOKS_AUDITABLE_BUILD must be 1 or unset, not '$FOKS_AUDITABLE_BUILD'" >&2
    exit 1
    ;;
esac
