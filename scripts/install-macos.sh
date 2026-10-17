#!/usr/bin/env bash
# Builds Filmriss in release mode and installs it as a real macOS app:
#   ~/Applications/Filmriss.app   (override with FILMRISS_APP_DIR=/Applications)
#
# The app bundle matters: macOS only grants camera access to apps whose Info.plist
# contains NSCameraUsageDescription – a bare binary would be killed on camera access
# when launched from Finder/Launchpad.
set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "This script is for macOS only." >&2
    exit 1
fi
if ! xcode-select -p >/dev/null 2>&1; then
    echo "Xcode Command Line Tools are missing – run: xcode-select --install" >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust is missing – install it from https://rustup.rs and open a new terminal." >&2
    exit 1
fi

cd "$(dirname "$0")/.."
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
DEST="${FILMRISS_APP_DIR:-$HOME/Applications}"
APP="$DEST/Filmriss.app"

echo "▸ Building Filmriss $VERSION (release) …"
cargo build --release -p filmriss-app

echo "▸ Creating $APP …"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/filmriss "$APP/Contents/MacOS/filmriss"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Filmriss</string>
    <key>CFBundleDisplayName</key>
    <string>Filmriss</string>
    <key>CFBundleIdentifier</key>
    <string>de.filmriss.Filmriss</string>
    <key>CFBundleExecutable</key>
    <string>filmriss</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.photography</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSCameraUsageDescription</key>
    <string>Filmriss needs the camera for the live preview and to take photos.</string>
</dict>
</plist>
PLIST

# Ad-hoc signature (no Apple developer account needed). Required on Apple Silicon.
codesign --force --deep --sign - "$APP"

echo "✓ Installed: $APP"
echo "  Start it from Launchpad/Spotlight, or with: open \"$APP\""
echo "  Logs (for debugging): \"$APP/Contents/MacOS/filmriss\" in a terminal"
