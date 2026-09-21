#!/usr/bin/env bash
set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "build-macos-release: macOS is required" >&2
  exit 1
fi

# Both names hold the Apple app-specific password used by notarytool.
notary_password="${APPLE_APP_PASSWORD:-${APPLE_PASSWORD:-}}"
if [[ -z "$notary_password" ]]; then
  echo "build-macos-release: APPLE_APP_PASSWORD or APPLE_PASSWORD is required" >&2
  exit 1
fi

for variable in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID; do
  if [[ -z "${!variable:-}" ]]; then
    echo "build-macos-release: $variable is required" >&2
    exit 1
  fi
done

for tool in cargo codesign hdiutil npm security spctl xcrun; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "build-macos-release: required tool is unavailable: $tool" >&2
    exit 1
  fi
done

if ! security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | grep -Fxq -- "$APPLE_SIGNING_IDENTITY"; then
  echo "build-macos-release: APPLE_SIGNING_IDENTITY is not an installed Developer ID Application identity" >&2
  exit 1
fi

# Validate the notary credentials before producing or replacing any artifact.
xcrun notarytool history \
  --apple-id "$APPLE_ID" \
  --team-id "$APPLE_TEAM_ID" \
  --password "$notary_password" \
  >/dev/null

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repository/Cargo.toml" | head -1)"
if [[ -z "$version" ]]; then
  echo "build-macos-release: workspace version is missing" >&2
  exit 1
fi

app="$repository/target/release/bundle/macos/foks-desktop.app"
release_directory="$repository/target/release/foks-release"
output="$release_directory/foks-desktop-$version-macos-arm64.dmg"
pending="$release_directory/.foks-desktop-$version-macos-arm64.pending.dmg"
mkdir -p "$release_directory"
rm -f -- "$output" "$pending"
trap 'rm -f -- "$pending"' EXIT

cd "$repository"
npm run stage-agent
(
  cd apps/desktop/src-tauri
  "$repository/node_modules/.bin/tauri" build \
    --bundles app \
    --config tauri.bundle.macos.conf.json \
    -- --locked
)

scripts/sign-macos-app.sh "$app"
scripts/notarize-macos-app.sh "$app" "$pending"
mv -- "$pending" "$output"
trap - EXIT

echo "Validated release artifact: $output"
