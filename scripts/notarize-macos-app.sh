#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
app="${1:-$repository/target/release/bundle/macos/foks-desktop.app}"
output="${2:-$repository/target/release/bundle/macos/foks-desktop-macos.zip}"

for variable in APPLE_ID APPLE_TEAM_ID APPLE_APP_PASSWORD; do
  if [[ -z "${!variable:-}" ]]; then
    echo "notarize-macos-app: $variable is required" >&2
    exit 1
  fi
done
if [[ ! -d "$app" ]]; then
  echo "notarize-macos-app: application bundle not found: $app" >&2
  exit 1
fi

submission="$(mktemp -t foks-notary.XXXXXX).zip"
trap 'rm -f "$submission"' EXIT
ditto -c -k --sequesterRsrc --keepParent "$app" "$submission"
xcrun notarytool submit "$submission" \
  --apple-id "$APPLE_ID" \
  --team-id "$APPLE_TEAM_ID" \
  --password "$APPLE_APP_PASSWORD" \
  --wait
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type execute --verbose "$app"
mkdir -p "$(dirname "$output")"
ditto -c -k --sequesterRsrc --keepParent "$app" "$output"
