#!/usr/bin/env bash
# Build, sign, and install the narrow UI screenshot helper.
#
# Must run in a Terminal on the machine, not over SSH: codesign needs the login
# keychain, which an SSH session cannot reach (it fails with
# errSecInternalComponent).
#
# Signing matters. Without a stable identity the binary is ad-hoc signed, TCC
# identifies it by content hash, and every rebuild silently invalidates the
# Screen Recording grant. That is the same failure mode as a locally rebuilt dev
# app losing Accessibility: the entry stays listed in System Settings but the
# running binary no longer matches it.

set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "$script_dir/.." rev-parse --show-toplevel)"
source_file="$repo_root/tauri/src-tauri/src/ui_shot.swift"
install_dir="$HOME/.minutes/bin"
install_path="$install_dir/ui_shot"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "install-ui-shot: macOS only" >&2
  exit 1
fi

# Pick a signing identity. Prefer an explicit override, then a Developer ID,
# then Apple Development. Any stable identity keeps the grant across rebuilds;
# ad-hoc does not.
identity="${MINUTES_DEV_SIGNING_IDENTITY:-}"
if [[ -z "$identity" ]]; then
  identity="$(security find-identity -v -p codesigning 2>/dev/null \
    | grep -oE '"(Developer ID Application|Apple Development): [^"]+"' \
    | head -1 | tr -d '"')" || true
fi
if [[ -z "$identity" ]]; then
  echo "install-ui-shot: no codesigning identity found." >&2
  echo "  Check with: security find-identity -v -p codesigning" >&2
  echo "  Without one the grant breaks on every rebuild." >&2
  exit 1
fi

echo "building  $source_file"
swiftc -parse-as-library -target arm64-apple-macos14.0 "$source_file" -o "/tmp/ui_shot.build"

mkdir -p "$install_dir"
install -m 755 "/tmp/ui_shot.build" "$install_path"
rm -f "/tmp/ui_shot.build"

echo "signing   $identity"
if ! codesign --force --options runtime --sign "$identity" "$install_path" 2>/tmp/ui-shot-sign.err; then
  echo >&2
  echo "install-ui-shot: codesign failed." >&2
  sed 's/^/  /' /tmp/ui-shot-sign.err >&2
  if grep -q 'errSecInternalComponent' /tmp/ui-shot-sign.err 2>/dev/null; then
    cat >&2 <<'SSHFAIL'

  errSecInternalComponent means the login keychain is not reachable from this
  session. That happens over SSH, including your own SSH session: the keychain
  needs an interactive login context.

  Run this in Terminal.app on the Mac itself, not over SSH.
SSHFAIL
  fi
  exit 1
fi

# Verify the signature actually took rather than trusting the exit code. An
# ad-hoc signature here means the TCC grant will break on the next rebuild, which
# fails silently and confusingly later, so catch it now.
authority="$(codesign -dv --verbose=2 "$install_path" 2>&1 | grep -m1 '^Authority=' || true)"
if [[ -z "$authority" ]]; then
  echo >&2
  echo "install-ui-shot: signing reported success but the binary is ad-hoc signed." >&2
  echo "  A TCC grant keyed to a content hash breaks on every rebuild." >&2
  echo "  Run this in Terminal.app on the Mac, not over SSH." >&2
  exit 1
fi
echo "installed $install_path"
echo "          ${authority:-Authority=<unsigned>}"

# Install the LaunchAgent. This is what makes remote captures work: macOS
# attributes capture to the responsible process, so an SSH-invoked capture is
# attributed to sshd and denied even when this binary is granted. An agent runs
# in the GUI session, where the grant applies.
agent_label="app.minutes.uishot"
agent_plist="$HOME/Library/LaunchAgents/$agent_label.plist"
log_path="$HOME/.minutes/ui-shot/agent.log"
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.minutes/ui-shot"

sed -e "s|REPLACE_WITH_HELPER_PATH|$install_path|" \
    -e "s|REPLACE_WITH_LOG_PATH|$log_path|" \
    "$repo_root/tauri/src-tauri/assets/$agent_label.plist" >"$agent_plist"

# bootout before bootstrap so a rebuilt binary is actually picked up rather than
# leaving the old one running.
launchctl bootout "gui/$(id -u)/$agent_label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$agent_plist"
echo "agent     $agent_label loaded (log: $log_path)"

echo
if "$install_path" --check >/dev/null 2>&1; then
  echo "Screen Recording: granted. Ready to use."
  echo "Remote captures go through the agent; local ones can call the binary directly."
else
  cat <<'GRANT'
Screen Recording: NOT granted yet.

  System Settings > Privacy & Security > Screen & System Audio Recording
  Add: ~/.minutes/bin/ui_shot

If an entry is already listed but this still reports not granted, remove it with
the minus button and re-add it. A stale record for an older build stays listed
without matching the current binary; toggling it off and on re-enables the same
stale record.

Verify with: ~/.minutes/bin/ui_shot --check
GRANT
fi
