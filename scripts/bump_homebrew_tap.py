#!/usr/bin/env python3
"""Point the Homebrew tap at a released version.

The tap holds two independent files, and they drift independently. The CLI
formula was bumped faithfully every release while the desktop cask sat at
0.18.2 through six of them, until a user reported that `brew upgrade --cask`
had been a no-op for months (#736). Nothing was broken; the step simply
depended on someone remembering, and release-time steps rot invisibly between
releases.

So this does both, from the release event, with no memory involved.

Design notes:

- The edits are surgical regex substitutions that must match exactly once. A
  tap file that has been restructured makes this refuse rather than guess,
  because a wrong `sha256` in a cask is worse than a stale one: stale installs
  an old version, wrong fails every install with a checksum mismatch.
- The cask hash is computed from the DMG bytes downloaded from the release,
  not read from SHA256SUMS.txt, which does not list the DMG at all.
- Idempotent. Re-running against a tap already at the version is a no-op that
  exits 0, so a re-run of the release workflow is harmless.
- The transforms are pure functions with a --self-test, because the only other
  way to exercise them is to push to the tap for real.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import sys
import urllib.error
import urllib.request

TAP_REPO = "silverstein/homebrew-tap"
SOURCE_REPO = "silverstein/minutes"
CASK_PATH = "Casks/minutes.rb"
FORMULA_PATH = "Formula/minutes.rb"
API = "https://api.github.com"


class BumpError(RuntimeError):
    """A condition that must stop the bump rather than be guessed around."""


# --------------------------------------------------------------------------
# Pure transforms
# --------------------------------------------------------------------------


def substitute_once(pattern: str, replacement: str, text: str, what: str) -> str:
    """Replace exactly one match, or refuse.

    Zero matches means the file moved on and this script no longer understands
    it. More than one means the file is ambiguous. Either way, writing would be
    a guess.
    """
    # MULTILINE so `^` anchors to each line: these directives sit indented
    # inside a cask/formula block, never at the start of the file.
    new_text, count = re.subn(pattern, replacement, text, count=2, flags=re.MULTILINE)
    if count != 1:
        raise BumpError(
            f"expected exactly one {what} in the tap file, found {count}. "
            "Refusing to write a guess; update this script alongside the tap."
        )
    return new_text


def bump_cask(content: str, version: str, sha256: str) -> str:
    content = substitute_once(
        r'^(\s*version\s+)"[^"]+"',
        lambda m: f'{m.group(1)}"{version}"',
        content,
        "version line",
    )
    return substitute_once(
        r'^(\s*sha256\s+)"[0-9a-f]{64}"',
        lambda m: f'{m.group(1)}"{sha256}"',
        content,
        "sha256 line",
    )


def bump_formula(content: str, version: str) -> str:
    return substitute_once(
        r'(tag:\s*)"v[^"]+"',
        lambda m: f'{m.group(1)}"v{version}"',
        content,
        "git tag reference",
    )


def cask_version(content: str) -> str | None:
    match = re.search(r'^\s*version\s+"([^"]+)"', content, re.MULTILINE)
    return match.group(1) if match else None


def formula_version(content: str) -> str | None:
    match = re.search(r'tag:\s*"v([^"]+)"', content)
    return match.group(1) if match else None


# --------------------------------------------------------------------------
# GitHub plumbing
# --------------------------------------------------------------------------


def request(url: str, token: str, method: str = "GET", payload: dict | None = None) -> dict:
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bearer {token}")
    req.add_header("Accept", "application/vnd.github+json")
    req.add_header("X-GitHub-Api-Version", "2022-11-28")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as response:
            return json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors="replace")[:400]
        # Never interpolate the token into an error.
        raise BumpError(f"{method} {url} failed with {error.code}: {body}") from None


def read_tap_file(path: str, token: str) -> tuple[str, str]:
    payload = request(f"{API}/repos/{TAP_REPO}/contents/{path}", token)
    return base64.b64decode(payload["content"]).decode(), payload["sha"]


def write_tap_file(path: str, content: str, sha: str, message: str, token: str) -> str:
    payload = request(
        f"{API}/repos/{TAP_REPO}/contents/{path}",
        token,
        method="PUT",
        payload={
            "message": message,
            "content": base64.b64encode(content.encode()).decode(),
            "sha": sha,
        },
    )
    return payload["commit"]["sha"][:8]


def dmg_sha256(version: str, token: str) -> str | None:
    """SHA-256 of the released DMG, or None when the release has no DMG."""
    asset_name = f"Minutes_{version}_aarch64.dmg"
    release = request(f"{API}/repos/{SOURCE_REPO}/releases/tags/v{version}", token)
    for asset in release.get("assets", []):
        if asset["name"] == asset_name:
            req = urllib.request.Request(asset["url"])
            req.add_header("Authorization", f"Bearer {token}")
            req.add_header("Accept", "application/octet-stream")
            digest = hashlib.sha256()
            with urllib.request.urlopen(req) as response:
                for chunk in iter(lambda: response.read(1024 * 1024), b""):
                    digest.update(chunk)
            return digest.hexdigest()
    return None


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------


def run(version: str, token: str, dry_run: bool) -> int:
    version = version.lstrip("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise BumpError(f"refusing to bump to a non-release version {version!r}")

    changed = False

    formula, formula_sha = read_tap_file(FORMULA_PATH, token)
    if formula_version(formula) == version:
        print(f"formula already at {version}")
    else:
        updated = bump_formula(formula, version)
        print(f"formula {formula_version(formula)} -> {version}")
        if not dry_run:
            commit = write_tap_file(
                FORMULA_PATH,
                updated,
                formula_sha,
                f"minutes {version} (CLI formula)",
                token,
            )
            print(f"  pushed {commit}")
        changed = True

    cask, cask_sha = read_tap_file(CASK_PATH, token)
    if cask_version(cask) == version:
        print(f"cask already at {version}")
    else:
        digest = dmg_sha256(version, token)
        if digest is None:
            # A release without a desktop artifact is not a failure, but it
            # must be visible: silence here is how the cask froze in the first
            # place.
            print(
                f"::warning::release v{version} has no Minutes_{version}_aarch64.dmg; "
                "cask left as is"
            )
        else:
            updated = bump_cask(cask, version, digest)
            print(f"cask {cask_version(cask)} -> {version} (sha256 {digest[:12]}...)")
            if not dry_run:
                commit = write_tap_file(
                    CASK_PATH,
                    updated,
                    cask_sha,
                    f"minutes {version} (desktop cask)",
                    token,
                )
                print(f"  pushed {commit}")
            changed = True

    if not changed:
        print("tap already current; nothing to do")
    elif dry_run:
        print("dry run: nothing was written")
    return 0


FIXTURE_CASK = """cask "minutes" do
  version "0.18.2"
  sha256 "71b267b411ce77c15e19b97a3cd38ad3c8d7c82e080f1ff1702ce7e39c06fbf9"

  url "https://github.com/silverstein/minutes/releases/download/v#{version}/Minutes_#{version}_aarch64.dmg"
  app "Minutes.app"
end
"""

FIXTURE_FORMULA = """class Minutes < Formula
  url "https://github.com/silverstein/minutes.git", tag: "v0.24.0"
  license "MIT"
end
"""


def self_test() -> int:
    failures = 0
    new_sha = "d" * 64

    updated = bump_cask(FIXTURE_CASK, "0.24.0", new_sha)
    if cask_version(updated) != "0.24.0" or new_sha not in updated:
        print("self-test FAILED: cask bump did not apply", file=sys.stderr)
        failures += 1
    # The url line interpolates #{version} and must survive untouched, or the
    # cask would point at a literal path and every install would 404.
    if '#{version}' not in updated or updated.count("sha256") != 1:
        print("self-test FAILED: cask bump damaged surrounding content", file=sys.stderr)
        failures += 1

    if formula_version(bump_formula(FIXTURE_FORMULA, "0.25.0")) != "0.25.0":
        print("self-test FAILED: formula bump did not apply", file=sys.stderr)
        failures += 1

    # Refusals. Guessing at a restructured tap file is the dangerous outcome:
    # a wrong sha256 fails every install, which is worse than a stale one.
    for label, content, fn in [
        ("a cask with no version line", 'cask "minutes" do\nend\n', lambda c: bump_cask(c, "1.0.0", new_sha)),
        ("a cask with two version lines",
         'version "1.0.0"\nversion "2.0.0"\nsha256 "%s"\n' % ("a" * 64),
         lambda c: bump_cask(c, "1.0.0", new_sha)),
        ("a formula with no tag", "class Minutes < Formula\nend\n", lambda c: bump_formula(c, "1.0.0")),
    ]:
        try:
            fn(content)
        except BumpError:
            print(f"self-test ok: refused {label}")
        else:
            print(f"self-test FAILED: accepted {label}", file=sys.stderr)
            failures += 1

    # Idempotence: bumping to the version already present changes nothing.
    once = bump_cask(FIXTURE_CASK, "0.24.0", new_sha)
    if bump_cask(once, "0.24.0", new_sha) != once:
        print("self-test FAILED: cask bump is not idempotent", file=sys.stderr)
        failures += 1
    else:
        print("self-test ok: cask bump is idempotent")

    if failures == 0:
        print("self-test ok: all transforms behave")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", help="release version, with or without a leading v")
    parser.add_argument("--dry-run", action="store_true", help="report without writing")
    parser.add_argument("--self-test", action="store_true", help="exercise the transforms")
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if not args.version:
        parser.error("--version is required unless --self-test is given")

    token = os.environ.get("HOMEBREW_TAP_TOKEN") or os.environ.get("GITHUB_TOKEN") or ""
    if not token:
        raise BumpError(
            "no token in HOMEBREW_TAP_TOKEN or GITHUB_TOKEN; the default "
            "workflow token cannot write to another repository"
        )
    return run(args.version, token, args.dry_run)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BumpError as error:
        print(f"::error::{error}", file=sys.stderr)
        raise SystemExit(1)
