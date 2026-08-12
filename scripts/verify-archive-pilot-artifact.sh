#!/bin/bash
set -euo pipefail

EXPECTED_TEAM_ID="63TMLKT8HN"
EXPECTED_IDENTIFIER="com.useminutes.archive"
ZIP_NAME="minutes-archive-pilot-notarized.zip"
SHA_NAME="${ZIP_NAME}.sha256"
PROVENANCE_NAME="signed-archive-provenance.txt"
SUMS_NAME="archive-release-SHA256SUMS.txt"
MANIFEST_NAME="latest-archive.json"

fail() {
  printf 'Archive pilot artifact verification failed: %s\n' "$*" >&2
  exit 1
}

usage() {
  printf 'Usage: %s <artifact-directory>\n' "$(basename "$0")" >&2
  printf 'Accepts the complete protected-run artifact or the exact six-file draft release.\n' >&2
  exit 2
}

[[ $# -eq 1 ]] || usage
[[ "$(uname -s)" == "Darwin" ]] ||
  fail "verification requires macOS Gatekeeper, codesign, and stapler"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
ARTIFACT_DIR="$1"
[[ -d "$ARTIFACT_DIR" ]] || fail "artifact directory does not exist"
ARTIFACT_DIR="$(cd "$ARTIFACT_DIR" && pwd -P)"
PROVENANCE_PATH="$ARTIFACT_DIR/$PROVENANCE_NAME"
[[ -f "$PROVENANCE_PATH" && ! -L "$PROVENANCE_PATH" ]] ||
  fail "missing or linked $PROVENANCE_NAME"

provenance_lines="$(wc -l <"$PROVENANCE_PATH" | tr -d '[:space:]')"
[[ "$provenance_lines" == "6" ]] ||
  fail "$PROVENANCE_NAME must contain the six reviewed fields"
candidate_sha="$(sed -n 's/^candidate=//p' "$PROVENANCE_PATH")"
team_id="$(sed -n 's/^team_id=//p' "$PROVENANCE_PATH")"
identifier="$(sed -n 's/^identifier=//p' "$PROVENANCE_PATH")"
expected_executable_sha="$(sed -n 's/^executable_sha256=//p' "$PROVENANCE_PATH")"
notarized="$(sed -n 's/^notarized=//p' "$PROVENANCE_PATH")"
stapled="$(sed -n 's/^stapled=//p' "$PROVENANCE_PATH")"
[[ ${#candidate_sha} -eq 40 && ! "$candidate_sha" =~ [^0-9a-f] ]] ||
  fail "candidate provenance is not one exact lowercase commit SHA"
[[ "$team_id" == "$EXPECTED_TEAM_ID" ]] ||
  fail "provenance Team ID is not the reviewed Minutes team"
[[ "$identifier" == "$EXPECTED_IDENTIFIER" ]] ||
  fail "provenance bundle identifier is not the Archive production identifier"
[[ ${#expected_executable_sha} -eq 64 && ! "$expected_executable_sha" =~ [^0-9a-f] ]] ||
  fail "executable provenance is not a lowercase SHA-256"
[[ "$notarized" == "true" && "$stapled" == "true" ]] ||
  fail "provenance does not claim both notarization and stapling"
expected_fields="$(grep -Ec '^(candidate|team_id|identifier|executable_sha256|notarized|stapled)=' "$PROVENANCE_PATH")"
[[ "$expected_fields" == "6" ]] ||
  fail "$PROVENANCE_NAME contains an unknown or duplicate field"

VERIFY_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/minutes-archive-verify.XXXXXX")"
MOUNT_POINT=""
cleanup() {
  if [[ -n "$MOUNT_POINT" ]]; then
    hdiutil detach "$MOUNT_POINT" -quiet || true
  fi
  rm -rf "$VERIFY_ROOT"
}
trap cleanup EXIT HUP INT TERM

verify_app() {
  local app="$1"
  local expected_version="${2:-}"
  local executable="$app/Contents/MacOS/minutes-archive-app"
  local info_plist="$app/Contents/Info.plist"
  [[ -d "$app" && -x "$executable" && -f "$info_plist" ]] ||
    fail "package does not contain the expected Minutes Archive application"
  if find "$app" -type l -print -quit | grep -q .; then
    fail "application contains a symbolic link"
  fi
  local bundle_identifier
  bundle_identifier="$(plutil -extract CFBundleIdentifier raw -o - "$info_plist")"
  [[ "$bundle_identifier" == "$EXPECTED_IDENTIFIER" ]] ||
    fail "Info.plist bundle identifier does not match"
  if [[ -n "$expected_version" ]]; then
    local bundle_version
    bundle_version="$(plutil -extract CFBundleShortVersionString raw -o - "$info_plist")"
    [[ "$bundle_version" == "$expected_version" ]] ||
      fail "application version does not match the updater manifest"
  fi

  codesign --verify --deep --strict --verbose=4 "$app"
  local identity signed_team_id signed_identifier
  identity="$(codesign -dv --verbose=4 "$app" 2>&1)"
  signed_team_id="$(awk -F= '/^TeamIdentifier=/{print $2}' <<<"$identity")"
  signed_identifier="$(awk -F= '/^Identifier=/{print $2}' <<<"$identity")"
  [[ "$signed_team_id" == "$EXPECTED_TEAM_ID" ]] || fail "code signature Team ID does not match"
  [[ "$signed_identifier" == "$EXPECTED_IDENTIFIER" ]] || fail "code signature identifier does not match"
  grep -Fq "Authority=Developer ID Application:" <<<"$identity" ||
    fail "application is not signed with a Developer ID Application identity"
  grep -Fq "flags=0x10000(runtime)" <<<"$identity" ||
    grep -Eq "flags=0x[0-9a-f]*10000" <<<"$identity" ||
    fail "application is not signed with the hardened runtime enabled"

  local entitlements_path="$VERIFY_ROOT/entitlements.$$.plist"
  if ! codesign -d --entitlements - "$app" >"$entitlements_path" 2>/dev/null; then
    fail "could not read entitlements; refusing to certify the artifact"
  fi
  if [[ -s "$entitlements_path" ]]; then
    for forbidden_entitlement in \
      "com.apple.security.get-task-allow" \
      "com.apple.security.cs.disable-library-validation" \
      "com.apple.security.cs.allow-dyld-environment-variables" \
      "com.apple.security.cs.allow-unsigned-executable-memory"; do
      if plutil -p "$entitlements_path" | grep -Fq "\"$forbidden_entitlement\" => true"; then
        fail "forbidden entitlement enabled: $forbidden_entitlement"
      fi
    done
  fi
  xcrun stapler validate "$app"
  spctl --assess --type execute --verbose=4 "$app"
  actual_executable_sha="$(shasum -a 256 "$executable" | awk '{print $1}')"
  [[ "$actual_executable_sha" == "$expected_executable_sha" ]] ||
    fail "signed executable SHA-256 does not match provenance"
}

if [[ -e "$ARTIFACT_DIR/$ZIP_NAME" || -e "$ARTIFACT_DIR/$SHA_NAME" ]]; then
  ZIP_PATH="$ARTIFACT_DIR/$ZIP_NAME"
  SHA_PATH="$ARTIFACT_DIR/$SHA_NAME"
  [[ -f "$ZIP_PATH" && -f "$SHA_PATH" && ! -L "$ZIP_PATH" && ! -L "$SHA_PATH" ]] ||
    fail "complete-run artifact must contain regular $ZIP_NAME and $SHA_NAME"
  sha_lines="$(wc -l <"$SHA_PATH" | tr -d '[:space:]')"
  sha_fields="$(awk 'NR == 1 { print NF }' "$SHA_PATH")"
  expected_zip_sha="$(awk 'NR == 1 { print $1 }' "$SHA_PATH")"
  declared_zip_name="$(awk 'NR == 1 { print $2 }' "$SHA_PATH")"
  [[ "$sha_lines" == "1" && "$sha_fields" == "2" && "$declared_zip_name" == "$ZIP_NAME" ]] ||
    fail "$SHA_NAME must bind only $ZIP_NAME"
  [[ ${#expected_zip_sha} -eq 64 && ! "$expected_zip_sha" =~ [^0-9a-f] ]] ||
    fail "$SHA_NAME does not contain a lowercase SHA-256"
  actual_zip_sha="$(shasum -a 256 "$ZIP_PATH" | awk '{print $1}')"
  [[ "$actual_zip_sha" == "$expected_zip_sha" ]] || fail "notarized zip SHA-256 does not match"
  ditto -x -k "$ZIP_PATH" "$VERIFY_ROOT/legacy"
  verify_app "$VERIFY_ROOT/legacy/Minutes Archive.app"
  printf 'artifact_shape=complete-run\n'
  printf 'zip_sha256=%s\n' "$actual_zip_sha"
else
  MANIFEST_PATH="$ARTIFACT_DIR/$MANIFEST_NAME"
  SUMS_PATH="$ARTIFACT_DIR/$SUMS_NAME"
  for path in "$MANIFEST_PATH" "$SUMS_PATH"; do
    [[ -f "$path" && ! -L "$path" ]] || fail "missing or linked $(basename "$path")"
  done
  version="$(python3 - "$MANIFEST_PATH" <<'PY'
import json, sys
value = json.load(open(sys.argv[1])).get("version", "")
if not isinstance(value, str):
    raise SystemExit(1)
print(value)
PY
)" || fail "manifest is not valid JSON"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "manifest version is not X.Y.Z"
  DMG_NAME="Minutes_Archive_${version}_aarch64.dmg"
  UPDATER_NAME="Minutes.Archive_${version}_aarch64.app.tar.gz"
  SIGNATURE_NAME="${UPDATER_NAME}.sig"
  expected_inventory="$VERIFY_ROOT/expected-inventory.txt"
  actual_inventory="$VERIFY_ROOT/actual-inventory.txt"
  printf '%s\n' "$DMG_NAME" "$UPDATER_NAME" "$SIGNATURE_NAME" "$SUMS_NAME" "$MANIFEST_NAME" "$PROVENANCE_NAME" | sort >"$expected_inventory"
  find "$ARTIFACT_DIR" -mindepth 1 -maxdepth 1 -exec basename {} \; | sort >"$actual_inventory"
  diff -u "$expected_inventory" "$actual_inventory" >/dev/null ||
    fail "draft-release directory is not the exact six-file promotion payload"
  while IFS= read -r name; do
    [[ ! -L "$ARTIFACT_DIR/$name" ]] || fail "release asset is a symbolic link: $name"
  done <"$expected_inventory"

  expected_checksum_names="$VERIFY_ROOT/expected-checksum-names.txt"
  actual_checksum_names="$VERIFY_ROOT/actual-checksum-names.txt"
  printf '%s\n' "$DMG_NAME" "$UPDATER_NAME" "$SIGNATURE_NAME" "$MANIFEST_NAME" "$PROVENANCE_NAME" | sort >"$expected_checksum_names"
  awk '{name=$2; sub(/^\*/, "", name); print name}' "$SUMS_PATH" | sort >"$actual_checksum_names"
  diff -u "$expected_checksum_names" "$actual_checksum_names" >/dev/null ||
    fail "$SUMS_NAME does not bind the exact five release files"
  (cd "$ARTIFACT_DIR" && shasum -a 256 -c "$SUMS_NAME")

  expected_url="https://github.com/silverstein/minutes/releases/download/archive-v${version}/${UPDATER_NAME}"
  python3 - "$MANIFEST_PATH" "$ARTIFACT_DIR/$SIGNATURE_NAME" "$version" "$expected_url" <<'PY' ||
import json, pathlib, sys
manifest = json.load(open(sys.argv[1]))
signature = pathlib.Path(sys.argv[2]).read_text().strip()
platforms = manifest.get("platforms")
if not isinstance(platforms, dict) or set(platforms) != {"darwin-aarch64"}:
    raise SystemExit("manifest must contain only darwin-aarch64")
platform = platforms["darwin-aarch64"]
if manifest.get("version") != sys.argv[3]:
    raise SystemExit("manifest version mismatch")
if platform.get("url") != sys.argv[4]:
    raise SystemExit("manifest updater URL mismatch")
if not signature or platform.get("signature") != signature:
    raise SystemExit("manifest signature mismatch")
PY
    fail "manifest does not bind the exact updater release asset"

  cargo run --quiet --manifest-path "$REPO_ROOT/archive/src-tauri/Cargo.toml" \
    --example verify_updater_signature -- \
    "$ARTIFACT_DIR/$UPDATER_NAME" "$ARTIFACT_DIR/$SIGNATURE_NAME" ||
    fail "updater signature does not validate against the embedded Archive public key"

  python3 - "$ARTIFACT_DIR/$UPDATER_NAME" "$VERIFY_ROOT/updater" <<'PY' ||
import pathlib, sys, tarfile
archive_path, destination = sys.argv[1:]
root = pathlib.PurePosixPath("Minutes Archive.app")
with tarfile.open(archive_path, "r:gz") as archive:
    members = archive.getmembers()
    if not members:
        raise SystemExit("empty updater archive")
    for member in members:
        path = pathlib.PurePosixPath(member.name)
        if not path.parts or path.parts[0] != str(root) or ".." in path.parts:
            raise SystemExit(f"unsafe updater path: {member.name}")
        if not (member.isfile() or member.isdir()):
            raise SystemExit("updater contains a link or special file")
    archive.extractall(destination, filter="data")
PY
    fail "updater archive structure is unsafe"
  verify_app "$VERIFY_ROOT/updater/Minutes Archive.app" "$version"

  DMG_PATH="$ARTIFACT_DIR/$DMG_NAME"
  codesign --verify --strict --verbose=4 "$DMG_PATH"
  dmg_identity="$(codesign -dv --verbose=4 "$DMG_PATH" 2>&1)"
  dmg_team_id="$(awk -F= '/^TeamIdentifier=/{print $2}' <<<"$dmg_identity")"
  [[ "$dmg_team_id" == "$EXPECTED_TEAM_ID" ]] ||
    fail "DMG code signature Team ID does not match"
  grep -Fq "Authority=Developer ID Application:" <<<"$dmg_identity" ||
    fail "DMG is not signed with a Developer ID Application identity"
  xcrun stapler validate "$DMG_PATH"
  spctl --assess --type open --context context:primary-signature --verbose=4 "$DMG_PATH"
  attach_output="$(hdiutil attach -readonly -nobrowse "$DMG_PATH")" || fail "could not mount DMG read-only"
  MOUNT_POINT="$(awk -F '\t' '$3 ~ /^\// { print $3; exit }' <<<"$attach_output")"
  [[ -n "$MOUNT_POINT" && -d "$MOUNT_POINT/Minutes Archive.app" ]] || fail "DMG does not contain Minutes Archive.app"
  [[ -L "$MOUNT_POINT/Applications" && "$(readlink "$MOUNT_POINT/Applications")" == "/Applications" ]] ||
    fail "DMG does not contain the expected Applications link"
  visible_mount_entries="$VERIFY_ROOT/visible-mount-entries.txt"
  find "$MOUNT_POINT" -mindepth 1 -maxdepth 1 ! -name '.*' -exec basename {} \; | sort >"$visible_mount_entries"
  diff -u <(printf '%s\n' Applications 'Minutes Archive.app') "$visible_mount_entries" >/dev/null ||
    fail "DMG contains an unexpected visible root payload"
  ditto "$MOUNT_POINT/Minutes Archive.app" "$VERIFY_ROOT/dmg/Minutes Archive.app"
  hdiutil detach "$MOUNT_POINT" -quiet
  MOUNT_POINT=""
  verify_app "$VERIFY_ROOT/dmg/Minutes Archive.app" "$version"
  diff -qr "$VERIFY_ROOT/updater/Minutes Archive.app" "$VERIFY_ROOT/dmg/Minutes Archive.app" >/dev/null ||
    fail "DMG and updater archive do not contain identical application trees"
  printf 'artifact_shape=draft-release\n'
  printf 'release_sums_sha256=%s\n' "$(shasum -a 256 "$SUMS_PATH" | awk '{print $1}')"
fi

printf 'artifact_verification=passed\n'
printf 'candidate_sha=%s\n' "$candidate_sha"
printf 'team_id=%s\n' "$team_id"
printf 'identifier=%s\n' "$identifier"
printf 'executable_sha256=%s\n' "$expected_executable_sha"
