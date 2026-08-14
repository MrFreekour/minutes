#!/bin/bash
# Move Tauri's temporary graph-worker sidecar into a real embedded XPC service,
# sign that service inside-out, and bind its exact CodeDirectory hash into the
# already-built parent executable. The caller must sign the outer app afterward.
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "Usage: $0 APP_BUNDLE SIGNING_IDENTITY ENTITLEMENTS_PLIST" >&2
  exit 2
fi

APP_BUNDLE="$1"
SIGNING_IDENTITY="$2"
ENTITLEMENTS_PLIST="$3"
SOURCE_WORKER="$APP_BUNDLE/Contents/MacOS/minutes-graph-worker"
XPC_BUNDLE="$APP_BUNDLE/Contents/XPCServices/com.useminutes.graph-worker.xpc"
XPC_CONTENTS="$XPC_BUNDLE/Contents"
XPC_EXECUTABLE="$XPC_CONTENTS/MacOS/minutes-graph-worker"
XPC_INFO="$XPC_CONTENTS/Info.plist"
GRAPH_WORKER_CDHASH="$APP_BUNDLE/Contents/Resources/minutes-graph-worker.cdhash"

test -d "$APP_BUNDLE"
test -f "$SOURCE_WORKER"
test ! -L "$SOURCE_WORKER"
test -f "$ENTITLEMENTS_PLIST"
file "$SOURCE_WORKER" | grep -q "Mach-O"

rm -rf "$XPC_BUNDLE"
mkdir -p "$XPC_CONTENTS/MacOS"
mv -f "$SOURCE_WORKER" "$XPC_EXECUTABLE"
chmod 755 "$XPC_EXECUTABLE"
test ! -e "$SOURCE_WORKER"

APP_VERSION="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
    "$APP_BUNDLE/Contents/Info.plist"
)"
python3 - \
  "crates/cli/assets/minutes-graph-worker-Info.plist" \
  "$XPC_INFO" \
  "$APP_VERSION" <<'PY'
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text()
if source.count("__MINUTES_VERSION__") != 2:
    raise SystemExit("graph XPC Info.plist template version markers are ambiguous")
version = sys.argv[3]
if not version or any(character not in "0123456789.-" for character in version):
    raise SystemExit("graph XPC app version is invalid")
pathlib.Path(sys.argv[2]).write_text(source.replace("__MINUTES_VERSION__", version))
PY
plutil -lint "$XPC_INFO" >/dev/null

if [[ "$SIGNING_IDENTITY" == "-" ]]; then
  codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS_PLIST" \
    --identifier com.useminutes.graph-worker \
    --sign - \
    "$XPC_BUNDLE"
else
  codesign --force --options runtime --timestamp \
    --entitlements "$ENTITLEMENTS_PLIST" \
    --identifier com.useminutes.graph-worker \
    --sign "$SIGNING_IDENTITY" \
    "$XPC_BUNDLE"
fi
codesign --verify --strict --verbose=4 "$XPC_BUNDLE"

# Capture the full report first, then parse it. Piping `codesign` straight
# into an awk that exits at the CDHash line closes the pipe while codesign is
# still writing the authority/timestamp chain, killing it with SIGPIPE; under
# `set -o pipefail` that aborts the whole build with exit 141. Ad-hoc signing
# emits few enough lines to finish first, so this only ever broke real signed
# builds. The parser also reads to EOF rather than exiting early.
graph_worker_signing_report="$(codesign -dvvv "$XPC_EXECUTABLE" 2>&1)"
graph_worker_cdhash="$(
  printf '%s\n' "$graph_worker_signing_report" |
    awk -F= '/^CDHash=/ && cdhash == "" { cdhash = tolower($2) } END { print cdhash }'
)"
if [[ ! "$graph_worker_cdhash" =~ ^[0-9a-f]{40}$ ]]; then
  echo "Signed graph XPC worker did not expose one exact CodeDirectory hash." >&2
  exit 1
fi
mkdir -p "$(dirname "$GRAPH_WORKER_CDHASH")"
printf '%s\n' "$graph_worker_cdhash" > "$GRAPH_WORKER_CDHASH"
chmod 444 "$GRAPH_WORKER_CDHASH"

APP_EXECUTABLE_NAME="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' \
    "$APP_BUNDLE/Contents/Info.plist"
)"
APP_EXECUTABLE="$APP_BUNDLE/Contents/MacOS/$APP_EXECUTABLE_NAME"
python3 scripts/seal_graph_worker_hash.py \
  "$APP_EXECUTABLE" "$graph_worker_cdhash"
python3 scripts/seal_graph_worker_hash.py \
  --verify "$APP_EXECUTABLE" "$graph_worker_cdhash"
