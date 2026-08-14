#!/bin/bash
set -euo pipefail

APP_PATH="${1:-/Applications/Minutes Dev.app}"
KEYCODE="${2:-}"
OUTPUT_PATH="${3:-/tmp/minutes-hotkey-diagnostic.json}"

if [[ ! -d "$APP_PATH" ]]; then
  echo "App not found: $APP_PATH" >&2
  echo "Usage: ./scripts/diagnose-desktop-hotkey.sh [/path/to/App.app] [keycode] [output_path]" >&2
  exit 1
fi

rm -f "$OUTPUT_PATH"

APP_EXECUTABLE="$APP_PATH/Contents/MacOS/minutes-app"
if [[ ! -x "$APP_EXECUTABLE" ]]; then
  echo "Installed app executable not found: $APP_EXECUTABLE" >&2
  exit 1
fi

DIAGNOSTIC_ARGS=(
  --diagnose-hotkey
  --diagnose-hotkey-output "$OUTPUT_PATH"
)
if [[ -n "$KEYCODE" ]]; then
  DIAGNOSTIC_ARGS+=(--diagnose-hotkey-keycode "$KEYCODE")
fi

set +e
"$APP_EXECUTABLE" "${DIAGNOSTIC_ARGS[@]}"
DIAGNOSTIC_EXIT=$?
set -e

if [[ ! -f "$OUTPUT_PATH" ]]; then
  echo "Installed diagnostic exited without writing $OUTPUT_PATH" >&2
  exit 1
fi

STATUS="$(python3 - <<'PY' "$OUTPUT_PATH"
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
payload = json.loads(path.read_text())
print(payload.get("probe", {}).get("status", "unknown"))
PY
)"

if [[ "$STATUS" == "active" && "$DIAGNOSTIC_EXIT" == "0" ]]; then
  exit 0
fi
exit 2
