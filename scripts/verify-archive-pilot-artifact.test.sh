#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERIFIER="$REPO_ROOT/scripts/verify-archive-pilot-artifact.sh"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/minutes-archive-verifier-test.XXXXXX")"

cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

MOCK_BIN="$TEST_ROOT/bin"
ARTIFACT_DIR="$TEST_ROOT/artifacts"
SOURCE_ROOT="$TEST_ROOT/source"
APP_PATH="$SOURCE_ROOT/Minutes Archive.app"
EXECUTABLE="$APP_PATH/Contents/MacOS/minutes-archive-app"
MOUNT_ROOT="$TEST_ROOT/mount"
ZIP_NAME="minutes-archive-pilot-notarized.zip"
SHA_NAME="${ZIP_NAME}.sha256"
PROVENANCE_NAME="signed-archive-provenance.txt"

mkdir -p "$MOCK_BIN" "$APP_PATH/Contents/MacOS"
printf 'synthetic signed executable fixture\n' >"$EXECUTABLE"
chmod 755 "$EXECUTABLE"
printf '%s\n' \
  '<?xml version="1.0" encoding="UTF-8"?>' \
  '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
  '<plist version="1.0"><dict>' \
  '<key>CFBundleIdentifier</key><string>com.useminutes.archive</string>' \
  '<key>CFBundleShortVersionString</key><string>0.2.0</string>' \
  '</dict></plist>' \
  >"$APP_PATH/Contents/Info.plist"

printf '%s\n' \
  '#!/bin/bash' \
  'target="${!#}"' \
  'if [[ "$1" == "-dv" && "$target" == *.dmg ]]; then' \
  '  printf "Authority=Developer ID Application: Test (63TMLKT8HN)\n" >&2' \
  '  printf "TeamIdentifier=%s\n" "${ARCHIVE_TEST_DMG_TEAM_ID:-63TMLKT8HN}" >&2' \
  '  exit 0' \
  'fi' \
  "if [[ \"\$1\" == \"-dv\" ]]; then" \
  '  printf "Identifier=com.useminutes.archive\n" >&2' \
  '  printf "Authority=Developer ID Application: Test (63TMLKT8HN)\n" >&2' \
  "  printf \"TeamIdentifier=%s\\n\" \"\${ARCHIVE_TEST_TEAM_ID:-63TMLKT8HN}\" >&2" \
  "  printf \"CodeDirectory v=20500 flags=%s\\n\" \"\${ARCHIVE_TEST_CS_FLAGS:-0x10000(runtime)}\" >&2" \
  'fi' \
  'exit 0' \
  >"$MOCK_BIN/codesign"
printf '%s\n' '#!/bin/bash' 'exit 0' >"$MOCK_BIN/xcrun"
printf '%s\n' '#!/bin/bash' 'exit 0' >"$MOCK_BIN/spctl"
printf '%s\n' \
  '#!/bin/bash' \
  'if [[ "$1" == "attach" ]]; then' \
  '  printf "/dev/disk99\tApple_HFS\t%s\n" "$ARCHIVE_TEST_MOUNT_ROOT"' \
  'fi' \
  'exit 0' \
  >"$MOCK_BIN/hdiutil"
printf '%s\n' '#!/bin/bash' 'printf "updater_signature=valid\n"' >"$MOCK_BIN/cargo"
chmod 755 "$MOCK_BIN/codesign" "$MOCK_BIN/xcrun" "$MOCK_BIN/spctl" \
  "$MOCK_BIN/hdiutil" "$MOCK_BIN/cargo"

make_artifact() {
  rm -rf "$ARTIFACT_DIR"
  mkdir -p "$ARTIFACT_DIR"
  ditto -c -k --sequesterRsrc --keepParent \
    "$APP_PATH" "$ARTIFACT_DIR/$ZIP_NAME"
  (
    cd "$ARTIFACT_DIR"
    shasum -a 256 "$ZIP_NAME" >"$SHA_NAME"
  )
  executable_sha="$(shasum -a 256 "$EXECUTABLE" | awk '{print $1}')"
  printf '%s\n' \
    "candidate=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
    "team_id=63TMLKT8HN" \
    "identifier=com.useminutes.archive" \
    "executable_sha256=$executable_sha" \
    "notarized=true" \
    "stapled=true" \
    >"$ARTIFACT_DIR/$PROVENANCE_NAME"
}

expect_failure() {
  description="$1"
  shift
  if "$@" >"$TEST_ROOT/failure.out" 2>&1; then
    printf 'Expected failure did not occur: %s\n' "$description" >&2
    exit 1
  fi
}

make_artifact
PATH="$MOCK_BIN:$PATH" "$VERIFIER" "$ARTIFACT_DIR" >"$TEST_ROOT/success.out"
grep -Fq "artifact_verification=passed" "$TEST_ROOT/success.out"

make_artifact
printf '0  %s\n' "$ZIP_NAME" >"$ARTIFACT_DIR/$SHA_NAME"
expect_failure "mismatched zip digest" \
  env PATH="$MOCK_BIN:$PATH" "$VERIFIER" "$ARTIFACT_DIR"

make_artifact
sed -i '' 's/^team_id=.*/team_id=WRONGTEAM/' \
  "$ARTIFACT_DIR/$PROVENANCE_NAME"
expect_failure "wrong provenance team" \
  env PATH="$MOCK_BIN:$PATH" "$VERIFIER" "$ARTIFACT_DIR"

make_artifact
expect_failure "wrong code-signing team" \
  env PATH="$MOCK_BIN:$PATH" ARCHIVE_TEST_TEAM_ID=WRONGTEAM \
  "$VERIFIER" "$ARTIFACT_DIR"

make_artifact
# Without the hardened runtime the forbidden-entitlement list below is moot:
# DYLD_INSERT_LIBRARIES can inject into the process holding the in-memory
# index of privileged documents.
expect_failure "not signed with the hardened runtime" \
  env PATH="$MOCK_BIN:$PATH" ARCHIVE_TEST_CS_FLAGS="0x2(adhoc)" \
  "$VERIFIER" "$ARTIFACT_DIR"

make_artifact
sed -i '' 's/^stapled=true$/unexpected=true/' \
  "$ARTIFACT_DIR/$PROVENANCE_NAME"
expect_failure "unexpected provenance field" \
  env PATH="$MOCK_BIN:$PATH" "$VERIFIER" "$ARTIFACT_DIR"

make_draft_release() {
  rm -rf "$ARTIFACT_DIR" "$MOUNT_ROOT"
  mkdir -p "$ARTIFACT_DIR" "$MOUNT_ROOT"
  ditto "$APP_PATH" "$MOUNT_ROOT/Minutes Archive.app"
  ln -s /Applications "$MOUNT_ROOT/Applications"
  COPYFILE_DISABLE=1 tar -C "$SOURCE_ROOT" -czf \
    "$ARTIFACT_DIR/Minutes.Archive_0.2.0_aarch64.app.tar.gz" \
    "Minutes Archive.app"
  printf 'synthetic dmg fixture\n' >"$ARTIFACT_DIR/Minutes_Archive_0.2.0_aarch64.dmg"
  printf 'synthetic signature fixture\n' \
    >"$ARTIFACT_DIR/Minutes.Archive_0.2.0_aarch64.app.tar.gz.sig"
  printf '%s\n' \
    '{' \
    '  "version": "0.2.0",' \
    '  "platforms": {' \
    '    "darwin-aarch64": {' \
    '      "signature": "synthetic signature fixture",' \
    '      "url": "https://github.com/silverstein/minutes/releases/download/archive-v0.2.0/Minutes.Archive_0.2.0_aarch64.app.tar.gz"' \
    '    }' \
    '  }' \
    '}' \
    >"$ARTIFACT_DIR/latest-archive.json"
  executable_sha="$(shasum -a 256 "$EXECUTABLE" | awk '{print $1}')"
  printf '%s\n' \
    "candidate=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
    "team_id=63TMLKT8HN" \
    "identifier=com.useminutes.archive" \
    "executable_sha256=$executable_sha" \
    "notarized=true" \
    "stapled=true" \
    >"$ARTIFACT_DIR/$PROVENANCE_NAME"
  (
    cd "$ARTIFACT_DIR"
    shasum -a 256 \
      Minutes_Archive_0.2.0_aarch64.dmg \
      Minutes.Archive_0.2.0_aarch64.app.tar.gz \
      Minutes.Archive_0.2.0_aarch64.app.tar.gz.sig \
      latest-archive.json \
      signed-archive-provenance.txt \
      >archive-release-SHA256SUMS.txt
  )
}

make_draft_release
PATH="$MOCK_BIN:$PATH" ARCHIVE_TEST_MOUNT_ROOT="$MOUNT_ROOT" \
  "$VERIFIER" "$ARTIFACT_DIR" >"$TEST_ROOT/draft-success.out"
grep -Fq "artifact_shape=draft-release" "$TEST_ROOT/draft-success.out"
grep -Fq "artifact_verification=passed" "$TEST_ROOT/draft-success.out"

make_draft_release
printf 'unexpected\n' >"$ARTIFACT_DIR/extra-file"
expect_failure "unexpected draft release asset" \
  env PATH="$MOCK_BIN:$PATH" ARCHIVE_TEST_MOUNT_ROOT="$MOUNT_ROOT" \
  "$VERIFIER" "$ARTIFACT_DIR"

make_draft_release
printf 'changed\n' >>"$ARTIFACT_DIR/Minutes.Archive_0.2.0_aarch64.app.tar.gz"
expect_failure "changed updater bytes" \
  env PATH="$MOCK_BIN:$PATH" ARCHIVE_TEST_MOUNT_ROOT="$MOUNT_ROOT" \
  "$VERIFIER" "$ARTIFACT_DIR"

make_draft_release
expect_failure "wrong DMG code-signing team" \
  env PATH="$MOCK_BIN:$PATH" ARCHIVE_TEST_MOUNT_ROOT="$MOUNT_ROOT" \
  ARCHIVE_TEST_DMG_TEAM_ID=WRONGTEAM "$VERIFIER" "$ARTIFACT_DIR"

make_draft_release
mkdir -p "$MOUNT_ROOT/Unexpected Installer.app"
expect_failure "unexpected visible DMG payload" \
  env PATH="$MOCK_BIN:$PATH" ARCHIVE_TEST_MOUNT_ROOT="$MOUNT_ROOT" \
  "$VERIFIER" "$ARTIFACT_DIR"

printf 'archive_pilot_artifact_verifier_tests=passed\n'
