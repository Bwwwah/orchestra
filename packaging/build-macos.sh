#!/bin/sh
# Builds "Orchestra.app" for every recent Mac (Apple chip and Intel) and a .zip to send.
# Usage, from the app folder:  sh packaging/build-macos.sh
# Result: dist/Orchestra-<version>-macOS.dmg (to install: open, drag onto Applications)
#         dist/Orchestra-<version>-macOS.zip (what the app's own updates download)
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

# RTK (rtk-oss, Apache-2.0, without its BUSL engine), next to the program: Orchestra's
# autonomous sessions shorten the AI's command outputs with it. Skipped with ORCHESTRA_NO_RTK=1.
if [ "${ORCHESTRA_NO_RTK:-0}" != "1" ]; then
  sh packaging/build-rtk.sh aarch64-apple-darwin x86_64-apple-darwin
  lipo -create -output "$APP/Contents/MacOS/rtk" \
    target/rtk/aarch64-apple-darwin/rtk \
    target/rtk/x86_64-apple-darwin/rtk
  cp target/rtk/LICENSE "$APP/Contents/Resources/LICENSE-rtk.txt"
fi

sed "s/__VERSION__/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Icon: every size macOS asks for, made from the logo by `cargo run --example icons`
# (assets/logo-source.png → assets/AppIcon.icns).
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cp packaging/LISEZMOI.txt "dist/Orchestra/LISEZMOI.txt"

# Signature: always the same identity (certificate) and the same bundle identifier, so that
# macOS keeps the permissions after an update; ad hoc with a warning when no certificate is
# given. See packaging/sign-macos.sh and docs/permissions-macos.md.
sh packaging/sign-macos.sh "$APP"
# Notarization by Apple (Developer ID only, when APPLE_ID… are given): the .zip and the
# .dmg below then hold the stapled app.
sh packaging/notarize-macos.sh "$APP"

ditto -c -k --norsrc --keepParent "dist/Orchestra" "dist/Orchestra-$VERSION-macOS.zip"

# Disk image: the app next to a shortcut to Applications.
DMG_DIR="dist/dmg"
mkdir -p "$DMG_DIR"
cp -R "$APP" "$DMG_DIR/"
ln -s /Applications "$DMG_DIR/Applications"
hdiutil create -volname Orchestra -srcfolder "$DMG_DIR" -ov -format UDZO "dist/Orchestra-$VERSION-macOS.dmg" >/dev/null
rm -rf "$DMG_DIR"

echo "Prêt : dist/Orchestra-$VERSION-macOS.dmg (et .zip)"
