#!/bin/sh
# Has Apple notarize Orchestra.app (signed with a Developer ID by sign-macos.sh), then
# staples the ticket to it: Gatekeeper then opens it at once, without « Ouvrir quand même ».
#
# Usage:  sh packaging/notarize-macos.sh path/to/Orchestra.app
#
# Needs (GitHub secrets of the same names for the releases):
#   APPLE_ID            the Apple account's e-mail address
#   APPLE_TEAM_ID       the team identifier (10 characters, developer.apple.com › Membership)
#   APPLE_APP_PASSWORD  an app-specific password (account.apple.com › Sign-In and Security)
# Without them: nothing is done (the app stays signed, not notarized).
set -e

APP="$1"
[ -d "$APP" ] || { echo "notarize-macos.sh : app introuvable : $APP" >&2; exit 1; }

if [ -z "${APPLE_ID:-}" ] || [ -z "${APPLE_TEAM_ID:-}" ] || [ -z "${APPLE_APP_PASSWORD:-}" ]; then
  echo "Notarisation sautée (APPLE_ID, APPLE_TEAM_ID ou APPLE_APP_PASSWORD manquant)."
  exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
ZIP="$TMP/Orchestra-notarisation.zip"
ditto -c -k --norsrc --keepParent "$APP" "$ZIP"

echo "Envoi à Apple pour notarisation (quelques minutes)…"
xcrun notarytool submit "$ZIP" \
  --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" \
  --wait --timeout 30m

# The ticket goes inside the app, so it opens even offline.
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
spctl --assess --type execute --verbose "$APP"
echo "Notarisée et agrafée."
