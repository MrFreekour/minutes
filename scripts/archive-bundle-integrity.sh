#!/bin/bash
# Whole-bundle integrity gate for the Archive pilot.
#
# The prior check only counted executables in Contents/MacOS at depth 1.
# `codesign --verify --deep --strict` does NOT treat Contents/Resources as
# nested code, so an unsigned Mach-O or script parked there passes every
# check, gets sealed into the outer Developer-ID CodeResources, and ships
# notarized. This enumerates the ENTIRE bundle instead.
set -euo pipefail
app="$1"
expected_executable="${2:-minutes-archive-app}"
expected_identifier="${3:-com.useminutes.archive}"

test -d "$app"

# Count Mach-O FILES, not `file` output lines: a universal binary reports one
# line per architecture, so `grep -c` over combined output overcounts wildly.
mach_o=""
while IFS= read -r -d '' f; do
  if file -b "$f" 2>/dev/null | grep -q 'Mach-O'; then
    mach_o+="$f"$'\n'
  fi
done < <(find "$app" -type f -print0)
mach_o_count="$(printf '%s' "$mach_o" | grep -c . || true)"

if [ "$mach_o_count" -ne 1 ]; then
  echo "REJECT: bundle must contain exactly one Mach-O; found $mach_o_count" >&2
  printf '%s' "$mach_o" | sed 's|^|  |' >&2
  exit 1
fi

bundle_identifier="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist"
)"
if [ "$bundle_identifier" != "$expected_identifier" ]; then
  echo "REJECT: CFBundleIdentifier is '$bundle_identifier', expected '$expected_identifier'" >&2
  exit 1
fi

bundle_executable="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist"
)"
if [ "$bundle_executable" != "$expected_executable" ]; then
  echo "REJECT: CFBundleExecutable is '$bundle_executable', expected '$expected_executable'" >&2
  exit 1
fi

entry="$app/Contents/MacOS/$bundle_executable"
test -x "$entry"
# A shell script passes `test -x`, which would make the hardened runtime
# vacuous (the real process becomes /bin/sh) and the provenance hash a
# description of a launcher stub.
if ! file -b "$entry" | grep -q 'Mach-O'; then
  echo "REJECT: entry point is not a Mach-O: $(file -b "$entry")" >&2
  exit 1
fi
# The single Mach-O found must BE the declared entry point.
if [ "$(printf '%s' "$mach_o" | grep -c .)" -eq 1 ] &&
   [ "$(printf '%s' "$mach_o" | tr -d '\n')" != "$entry" ]; then
  echo "REJECT: the only Mach-O is not the declared entry point" >&2
  echo "  found: $(printf '%s' "$mach_o" | tr -d '\n')" >&2
  echo "  entry: $entry" >&2
  exit 1
fi

if find "$app" -type l -print -quit | grep -q .; then
  echo "REJECT: bundle contains a symbolic link" >&2
  exit 1
fi

# Extended attributes are not visible to `find -type f` or `file`, survive
# `ditto -c -k` and `codesign`, and pass `codesign --verify --deep --strict`.
# An arbitrary xattr is therefore a covert channel for shipping content inside
# a signed, notarized bundle that a content-only hash certifies as clean.
# Resource forks are rejected by codesign as detritus; arbitrary names are not.
unexpected_xattrs=""
while IFS= read -r -d '' entry; do
  while IFS= read -r attribute; do
    [ -n "$attribute" ] || continue
    case "$attribute" in
      com.apple.provenance | com.apple.quarantine | com.apple.macl) ;;
      *) unexpected_xattrs+="$attribute on ${entry#"$app"}"$'\n' ;;
    esac
  done < <(xattr "$entry" 2>/dev/null)
done < <(find "$app" -print0)
if [ -n "$unexpected_xattrs" ]; then
  echo "REJECT: bundle carries unexpected extended attributes" >&2
  printf '%s' "$unexpected_xattrs" | sed 's|^|  |' >&2
  exit 1
fi

# Digest of everything shipped: every entry of every type, its extended
# attributes, and file contents.
#
# Three earlier defects, all of which made the provenance record unreliable
# rather than merely incomplete. Directories were never walked, so xattrs on a
# directory smuggled unbounded content with a byte-identical hash. `sort`
# without LC_ALL=C is locale-dependent, so a reviewer reproducing the digest
# locally could legitimately differ from CI. And a newline-joined
# "<hash>  <path>" stream is not injective: a filename containing a newline
# forges entries, so two different trees can collide.
#
# Records are NUL-terminated and field-separated, paths are taken verbatim
# rather than through shasum's escaping, and content is hashed from stdin so
# the filename never enters shasum's output.
tree_hash="$(
  {
    find "$app" -print0 | LC_ALL=C sort -z |
      while IFS= read -r -d '' entry; do
        relative="${entry#"$app"}"
        if [ -L "$entry" ]; then
          kind=l
          payload="$(readlink "$entry")"
        elif [ -d "$entry" ]; then
          kind=d
          payload=""
        elif [ -f "$entry" ]; then
          kind=f
          payload="$(shasum -a 256 < "$entry" | awk '{print $1}')"
          if [ -z "$payload" ]; then
            echo "REJECT: could not read $relative for hashing" >&2
            exit 1
          fi
        else
          kind=o
          payload=""
        fi
        printf '%s\037%s\037%s\000' "$kind" "$relative" "$payload"
        while IFS= read -r attribute; do
          [ -n "$attribute" ] || continue
          attribute_digest="$(
            xattr -p "$attribute" "$entry" 2>/dev/null | shasum -a 256 | awk '{print $1}'
          )"
          printf 'x\037%s\037%s\037%s\000' \
            "$relative" "$attribute" "$attribute_digest"
        done < <(xattr "$entry" 2>/dev/null | LC_ALL=C sort)
      done
  } | shasum -a 256 | awk '{print $1}'
)"
if [ -z "$tree_hash" ]; then
  echo "REJECT: could not compute the bundle tree digest" >&2
  exit 1
fi
printf 'mach_o_count=%s\nidentifier=%s\nentry=%s\ntree_sha256=%s\n' \
  "$mach_o_count" "$bundle_identifier" "$bundle_executable" "$tree_hash"
