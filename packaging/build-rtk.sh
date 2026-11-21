#!/bin/sh
# Builds RTK (Rust Token Killer) as shipped with Orchestra: rtk-oss, the Apache-2.0 edition,
# built WITHOUT RTK's engine (rtk-engine, BUSL-1.1, not allowed inside another product).
# Orchestra's autonomous sessions use it to shorten the outputs of the AI's shell commands.
#
# Usage, from the app folder:  sh packaging/build-rtk.sh [target…]
#   no target: this computer's; e.g. aarch64-apple-darwin x86_64-apple-darwin for a Mac app.
# Result: target/rtk/<target>/rtk for each target. Its licence and attribution, shipped with
# it, are in packaging/licenses (checked against RTK's own LICENSE below).
# ORCHESTRA_NO_RTK=1 skips it (Orchestra then runs its sessions without RTK).
set -e
cd "$(dirname "$0")/.."

[ "${ORCHESTRA_NO_RTK:-0}" = "1" ] && { echo "RTK non fabriqué (ORCHESTRA_NO_RTK=1)."; exit 0; }

# The version shipped: one fixed commit of github.com/rtk-ai/rtk (v0.49.0), checked again
# after the download.
RTK_REPO="https://github.com/rtk-ai/rtk.git"
RTK_REV="ef0b734b6b2f5df26267bf05d90d71b887658580"

SRC="target/rtk/src"
if [ ! -d "$SRC/.git" ]; then
  rm -rf "$SRC"
  mkdir -p "$SRC"
  git -C "$SRC" init -q
  git -C "$SRC" remote add origin "$RTK_REPO"
fi
git -C "$SRC" fetch -q --depth 1 origin "$RTK_REV"
git -C "$SRC" checkout -q --detach FETCH_HEAD
[ "$(git -C "$SRC" rev-parse HEAD)" = "$RTK_REV" ] || { echo "build-rtk.sh : mauvaise version de RTK" >&2; exit 1; }

# Never the BUSL engine: refuse to go on if it is in the dependency graph.
if cargo tree --manifest-path "$SRC/Cargo.toml" --locked --no-default-features -e normal -p rtk | grep -q "rtk-engine"; then
  echo "build-rtk.sh : rtk-engine (BUSL-1.1) est dans le binaire : arrêt." >&2
  exit 1
fi

[ $# -gt 0 ] || set -- "$(rustc -vV | sed -n 's/^host: //p')"
for t in "$@"; do
  echo "Compilation de RTK (rtk-oss) pour $t…"
  # No telemetry address compiled in: RTK never sends usage reports.
  env -u RTK_TELEMETRY_URL cargo build --release --locked --no-default-features \
    --manifest-path "$SRC/Cargo.toml" --target "$t" --target-dir target/rtk/build
  mkdir -p "target/rtk/$t"
  cp "target/rtk/build/$t/release/rtk" "target/rtk/$t/rtk"
done
# The licence shipped and shown in Settings › General is packaging/licenses/rtk-LICENSE.txt:
# it must still be RTK's own.
cmp -s "$SRC/LICENSE" packaging/licenses/rtk-LICENSE.txt || {
  echo "build-rtk.sh : la licence de RTK a changé : mettez à jour packaging/licenses (rtk-LICENSE.txt, LICENCES-TIERCES.txt)." >&2
  exit 1
}
echo "Prêt : target/rtk/<cible>/rtk (rtk-oss, Apache-2.0)"
