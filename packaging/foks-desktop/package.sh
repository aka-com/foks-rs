#!/bin/sh
set -eu

root=$(git rev-parse --show-toplevel)
output=${1:?usage: package.sh OUTPUT_DIRECTORY}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)
mkdir -p "$output"

cargo build --locked --release -p foks-agent -p foks-desktop

case $(uname -s) in
    Darwin)
        app="$output/FOKS.app"
        mkdir -p "$app/Contents/MacOS" "$app/Contents/Helpers" "$app/Contents/Resources"
        cp "$root/target/release/foks-desktop" "$app/Contents/MacOS/foks-desktop"
        cp "$root/target/release/foks-agent" "$app/Contents/Helpers/foks-agent"
        cp "$root/packaging/foks-desktop/macos/Info.plist" "$app/Contents/Info.plist"
        cp "$root/packaging/foks-desktop/macos/entitlements.plist" "$app/Contents/Resources/entitlements.plist"
        sed -i '' "s/__FOKS_VERSION__/$version/g" "$app/Contents/Info.plist"
        if [ "${FOKS_RELEASE:-0}" = 1 ]; then
            : "${FOKS_CODESIGN_IDENTITY:?release packaging requires FOKS_CODESIGN_IDENTITY}"
            codesign --force --timestamp --options runtime \
                --entitlements "$app/Contents/Resources/entitlements.plist" \
                --sign "$FOKS_CODESIGN_IDENTITY" "$app/Contents/Helpers/foks-agent"
            codesign --force --timestamp --options runtime \
                --entitlements "$app/Contents/Resources/entitlements.plist" \
                --sign "$FOKS_CODESIGN_IDENTITY" "$app"
            codesign --verify --deep --strict --verbose=2 "$app"
        else
            codesign --force --deep --sign - "$app"
        fi
        ditto -c -k --keepParent "$app" "$output/foks-desktop-$version-macos.zip"
        ;;
    Linux)
        stage="$output/foks-desktop-$version-linux"
        mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/metainfo" "$stage/lib/systemd/user"
        cp "$root/target/release/foks-desktop" "$stage/bin/"
        cp "$root/target/release/foks-agent" "$stage/bin/"
        cp "$root/packaging/foks-desktop/linux/foks-desktop-launcher" "$stage/bin/"
        cp "$root/packaging/foks-desktop/linux/org.foks.Desktop.desktop" "$stage/share/applications/"
        cp "$root/packaging/foks-desktop/linux/org.foks.Desktop.metainfo.xml" "$stage/share/metainfo/"
        cp "$root/packaging/foks-agent/foks-agent.service" "$stage/lib/systemd/user/"
        tar -C "$output" -czf "$output/foks-desktop-$version-linux.tar.gz" "$(basename "$stage")"
        ;;
    *)
        echo "FOKS desktop packages support macOS and Linux only" >&2
        exit 1
        ;;
esac
