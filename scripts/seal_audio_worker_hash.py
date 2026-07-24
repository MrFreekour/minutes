#!/usr/bin/env python3
"""Bind one exact signed private-audio helper to the parent Mach-O."""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import tempfile


MARKER = b"MINUTES_AUDIO_WORKER_CDHASH_V1="
PLACEHOLDER = MARKER + (b"0" * 40)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("executable", type=pathlib.Path, nargs="?")
    parser.add_argument("cdhash", nargs="?")
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args()


def seal(executable: pathlib.Path, cdhash: str, *, verify: bool) -> None:
    if re.fullmatch(r"[0-9a-f]{40}", cdhash) is None:
        raise SystemExit("audio worker CodeDirectory hash must be 40 lowercase hex bytes")

    expected = MARKER + cdhash.encode("ascii")
    contents = executable.read_bytes()
    if verify:
        if contents.count(expected) != 1 or PLACEHOLDER in contents:
            raise SystemExit("parent executable does not contain one exact audio-worker seal")
        return

    if contents.count(PLACEHOLDER) != 1:
        raise SystemExit("parent executable must contain one unbound audio-worker seal")
    if contents.count(MARKER) != 1:
        raise SystemExit("parent executable contains an ambiguous audio-worker seal")

    offset = contents.index(PLACEHOLDER)
    with executable.open("r+b", buffering=0) as target:
        target.seek(offset)
        target.write(expected)
        target.flush()
        os.fsync(target.fileno())

    sealed = executable.read_bytes()
    if sealed.count(expected) != 1 or PLACEHOLDER in sealed:
        raise SystemExit("parent executable audio-worker seal verification failed")


def self_test() -> None:
    cdhash = "00112233445566778899aabbccddeeff00112233"
    with tempfile.TemporaryDirectory() as directory:
        executable = pathlib.Path(directory) / "Minutes"
        executable.write_bytes(b"prefix" + PLACEHOLDER + b"suffix")
        seal(executable, cdhash, verify=False)
        seal(executable, cdhash, verify=True)

        duplicate = pathlib.Path(directory) / "Duplicate"
        duplicate.write_bytes(PLACEHOLDER + PLACEHOLDER)
        try:
            seal(duplicate, cdhash, verify=False)
        except SystemExit:
            pass
        else:
            raise SystemExit("duplicate audio-worker seals were accepted")

        try:
            seal(executable, "A" * 40, verify=True)
        except SystemExit:
            pass
        else:
            raise SystemExit("uppercase audio-worker hash was accepted")


def main() -> None:
    args = parse_args()
    if args.self_test:
        if args.executable is not None or args.cdhash is not None or args.verify:
            raise SystemExit("--self-test does not accept an executable, hash, or --verify")
        self_test()
        return
    if args.executable is None or args.cdhash is None:
        raise SystemExit("executable and cdhash are required")
    seal(args.executable, args.cdhash, verify=args.verify)


if __name__ == "__main__":
    main()
