#!/bin/sh
# Builds "Orchestra.app" for every recent Mac (Apple chip and Intel) and a .zip to send.
# Usage, from the app folder:  sh packaging/build-macos.sh
# Result: dist/Orchestra-<version>-macOS.zip
set -e
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="dist/Orchestra/Orchestra.app"

echo "Compilation (puce Apple)…"
cargo build --release --target aarch64-apple-darwin
echo "Compilation (Intel)…"
cargo build --release --target x86_64-apple-darwin

rm -rf dist
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# One program for both kinds of Mac.
lipo -create -output "$APP/Contents/MacOS/orchestra" \
  target/aarch64-apple-darwin/release/orchestra \
  target/x86_64-apple-darwin/release/orchestra

sed "s/__VERSION__/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Icon: every size macOS asks for, from the 1024 px picture.
ICONSET="dist/AppIcon.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s packaging/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d packaging/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

cp packaging/LISEZMOI.txt "dist/Orchestra/LISEZMOI.txt"

# Ad-hoc signature: required to run on Apple-chip Macs (not an Apple-issued certificate).
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

ditto -c -k --norsrc --keepParent "dist/Orchestra" "dist/Orchestra-$VERSION-macOS.zip"
echo "Prêt : dist/Orchestra-$VERSION-macOS.zip"
