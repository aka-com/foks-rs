#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
app="${1:-$repository/target/release/bundle/macos/foks-desktop.app}"
entitlements="$repository/packaging/foks-desktop/macos/entitlements.plist"
identity="${APPLE_SIGNING_IDENTITY:-}"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "sign-macos-app: macOS is required" >&2
  exit 1
fi
if [[ ! -d "$app" ]]; then
  echo "sign-macos-app: application bundle not found: $app" >&2
  exit 1
fi
if [[ -z "$identity" ]]; then
  identities="$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p')"
  if [[ "$(printf '%s\n' "$identities" | grep -c . || true)" -ne 1 ]]; then
    echo "sign-macos-app: set APPLE_SIGNING_IDENTITY or install one Developer ID Application identity" >&2
    exit 1
  fi
  identity="$identities"
fi

helper="$app/Contents/MacOS/foks-agent"
if [[ ! -x "$helper" ]]; then
  echo "sign-macos-app: packaged agent is missing or not executable: $helper" >&2
  exit 1
fi
codesign --force --timestamp --options runtime --entitlements "$entitlements" --sign "$identity" "$helper"
codesign --force --timestamp --options runtime --entitlements "$entitlements" --sign "$identity" "$app"
codesign --verify --deep --strict --verbose=2 "$app"
