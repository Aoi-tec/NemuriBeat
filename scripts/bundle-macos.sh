#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mode=release
if [ "${1:-}" = "--debug" ]; then mode=debug; cargo build -p nemuribeat; else cargo build --release -p nemuribeat; fi
bundle=target/NemuriBeat.app
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp apps/desktop/assets/branding/NemuriBeat.icns "$bundle/Contents/Resources/"
cp "target/$mode/nemuribeat" "$bundle/Contents/MacOS/nemuribeat.new"
mv "$bundle/Contents/MacOS/nemuribeat.new" "$bundle/Contents/MacOS/nemuribeat"
cp apps/desktop/Info.plist "$bundle/Contents/Info.plist"
printf APPL???? > "$bundle/Contents/PkgInfo"
codesign --force --sign - "$bundle"
printf '\nCreated %s\n' "$bundle"
