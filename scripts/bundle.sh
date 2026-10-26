#!/usr/bin/env bash
# Builds a release binary and wraps it in an ad-hoc signed Murmur.app so macOS
# can attach Microphone / Accessibility / Input Monitoring grants to it.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release
APP=target/Murmur.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/murmur "$APP/Contents/MacOS/Murmur"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Murmur</string>
  <key>CFBundleDisplayName</key><string>Murmur</string>
  <key>CFBundleIdentifier</key><string>dev.naeem.murmur</string>
  <key>CFBundleExecutable</key><string>Murmur</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Murmur records your voice while you hold the dictation key and transcribes it on this Mac.</string>
</dict></plist>
PLIST

# A stable identity keeps macOS privacy grants across rebuilds; ad-hoc signatures are tied to
# the binary's hash, so every rebuild would need Accessibility granted again.
IDENTITY="${MURMUR_SIGN_IDENTITY:-Murmur Dev}"
if security find-identity -p codesigning | grep -q "\"$IDENTITY\""; then
  codesign --force --deep --sign "$IDENTITY" "$APP"
else
  echo "warning: signing identity '$IDENTITY' not found; signing ad-hoc (privacy grants reset on every rebuild)" >&2
  codesign --force --deep --sign - "$APP"
fi
echo "Built $APP"
