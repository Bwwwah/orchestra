#!/bin/sh
# Signs Orchestra.app with the same identity from one version to the next, so that macOS
# keeps its permissions (Screen Recording…) after each update: see docs/permissions-macos.md.
#
# Usage:  sh packaging/sign-macos.sh path/to/Orchestra.app
#
# Which identity, in this order:
#   1. MACOS_CERT_P12 (the certificate and its private key, .p12 in base64) and
#      MACOS_CERT_PASSWORD: imported into a temporary keychain (for GitHub Actions, from the
#      repository's secrets). MACOS_SIGN_IDENTITY names it (default: "Orchestra").
#   2. MACOS_SIGN_IDENTITY alone: a certificate already in the keychain of this Mac
#      (Developer ID Application: …, or a personal "Orchestra" certificate).
#   3. Nothing: ad-hoc signature, with a warning (the permissions are then lost at each update).
# MACOS_REQUIRE_IDENTITY=1 turns the ad-hoc fallback into an error (for the releases).
set -e

APP="$1"
[ -d "$APP" ] || { echo "sign-macos.sh : app introuvable : $APP" >&2; exit 1; }

# The bundle identifier never changes: macOS files the permissions under it.
BUNDLE_ID="app.orchestra.desktop"
GOT_ID=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$APP/Contents/Info.plist")
if [ "$GOT_ID" != "$BUNDLE_ID" ]; then
  echo "sign-macos.sh : identifiant du bundle « $GOT_ID » au lieu de « $BUNDLE_ID » : les permissions seraient perdues." >&2
  exit 1
fi

IDENTITY="${MACOS_SIGN_IDENTITY:-}"
KEYCHAIN=""
cleanup() {
  if [ -n "$KEYCHAIN" ]; then
    security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if [ -n "${MACOS_CERT_P12:-}" ]; then
  IDENTITY="${IDENTITY:-Orchestra}"
  TMP=$(mktemp -d)
  KEYCHAIN="$TMP/orchestra-sign.keychain-db"
  KC_PASS=$(openssl rand -hex 16)
  printf '%s' "$MACOS_CERT_P12" | base64 --decode > "$TMP/cert.p12"
  security create-keychain -p "$KC_PASS" "$KEYCHAIN"
  security set-keychain-settings -lut 3600 "$KEYCHAIN"
  security unlock-keychain -p "$KC_PASS" "$KEYCHAIN"
  security import "$TMP/cert.p12" -k "$KEYCHAIN" -P "${MACOS_CERT_PASSWORD:-}" -T /usr/bin/codesign >/dev/null
  rm -f "$TMP/cert.p12"
  security set-key-partition-list -S apple-tool:,apple: -s -k "$KC_PASS" "$KEYCHAIN" >/dev/null
  # Searched first, the user's own keychains after.
  # shellcheck disable=SC2046
  security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
fi

# Helper programs next to Orchestra (RTK): signed first, on their own, with the same identity
# (notarization wants every program inside the app signed, with the hardened runtime).
RTK_BIN="$APP/Contents/MacOS/rtk"
if [ -f "$RTK_BIN" ]; then
  case "$IDENTITY" in
    "Developer ID Application"*)
      codesign --force --options runtime --timestamp --identifier app.orchestra.rtk --sign "$IDENTITY" "$RTK_BIN" ;;
    "")
      codesign --force --identifier app.orchestra.rtk --sign - "$RTK_BIN" ;;
    *)
      codesign --force --timestamp=none --identifier app.orchestra.rtk --sign "$IDENTITY" "$RTK_BIN" ;;
  esac
fi

if [ -n "$IDENTITY" ]; then
  echo "Signature avec « $IDENTITY »…"
  case "$IDENTITY" in
    "Developer ID Application"*)
      # Developer ID: hardened runtime and Apple's timestamp, both required to notarize.
      codesign --force --deep --options runtime --timestamp \
        --entitlements packaging/entitlements.plist \
        --identifier "$BUNDLE_ID" --sign "$IDENTITY" "$APP"
      ;;
    *)
      codesign --force --deep --timestamp=none --identifier "$BUNDLE_ID" --sign "$IDENTITY" "$APP"
      ;;
  esac
else
  if [ "${MACOS_REQUIRE_IDENTITY:-0}" = "1" ]; then
    echo "sign-macos.sh : aucun certificat (MACOS_SIGN_IDENTITY ou MACOS_CERT_P12) alors que MACOS_REQUIRE_IDENTITY=1." >&2
    exit 1
  fi
  echo "ATTENTION : signature ad hoc. macOS oubliera les permissions (Enregistrement de l'écran…) à chaque mise à jour." >&2
  echo "            Donnez MACOS_SIGN_IDENTITY (voir docs/permissions-macos.md)." >&2
  # Ad-hoc signature: required to run on Apple-chip Macs.
  codesign --force --deep --identifier "$BUNDLE_ID" --sign - "$APP"
fi
codesign --verify --deep --strict "$APP"

# What macOS will check at the next version: with a certificate, the identifier and the
# certificate (stable); ad hoc, only this build's hash (cdhash, changes every time).
REQ=$(codesign -d -r- "$APP" 2>&1 | sed -n 's/^designated => //p')
echo "Exigence désignée : $REQ"
case "$REQ" in
  *"identifier \"$BUNDLE_ID\""*) ;;
  *)
    if [ -n "$IDENTITY" ]; then
      echo "sign-macos.sh : l'exigence désignée ne porte pas l'identifiant $BUNDLE_ID." >&2
      exit 1
    fi
    ;;
esac
