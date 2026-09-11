#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
app="${1:-$repository/target/release/bundle/macos/foks-desktop.app}"
output="${2:-$repository/target/release/bundle/dmg/foks-desktop-macos.dmg}"
identity="${APPLE_SIGNING_IDENTITY:-}"

# Both names hold the Apple app-specific password used by notarytool.
notary_password="${APPLE_APP_PASSWORD:-${APPLE_PASSWORD:-}}"
if [[ -z "$notary_password" ]]; then
  echo "notarize-macos-app: APPLE_APP_PASSWORD or APPLE_PASSWORD is required" >&2
  exit 1
fi

for variable in APPLE_ID APPLE_TEAM_ID; do
  if [[ -z "${!variable:-}" ]]; then
    echo "notarize-macos-app: $variable is required" >&2
    exit 1
  fi
done
if [[ ! -d "$app" ]]; then
  echo "notarize-macos-app: application bundle not found: $app" >&2
  exit 1
fi
codesign --verify --deep --strict --verbose=2 "$app"
if [[ -z "$identity" ]]; then
  identities="$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p')"
  if [[ "$(printf '%s\n' "$identities" | grep -c . || true)" -ne 1 ]]; then
    echo "notarize-macos-app: set APPLE_SIGNING_IDENTITY or install one Developer ID Application identity" >&2
    exit 1
  fi
  identity="$identities"
fi

staging="$(mktemp -d -t foks-dmg.XXXXXX)"
trap 'rm -rf "$staging"' EXIT
ditto "$app" "$staging/foks-desktop.app"
ln -s /Applications "$staging/Applications"
mkdir -p "$(dirname "$output")"
hdiutil create -volname "FOKS Desktop" -srcfolder "$staging" -ov -format UDZO "$output"
codesign --force --timestamp --sign "$identity" "$output"
codesign --verify --strict --verbose=2 "$output"
xcrun notarytool submit "$output" \
  --apple-id "$APPLE_ID" \
  --team-id "$APPLE_TEAM_ID" \
  --password "$notary_password" \
  --wait
xcrun stapler staple "$output"
xcrun stapler validate "$output"
spctl --assess --type open --context context:primary-signature --verbose "$output"
