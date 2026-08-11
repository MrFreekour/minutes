#!/usr/bin/env python3
"""Prove a packaged sherpa plugin is one the shipped host will actually use.

The plugin is dlopened lazily, and only once sherpa is genuinely selected, so a
packaged CLI runs `--version` perfectly while its plugin is unloadable. The
usual failure is loader search paths: the plugin resolves
libsherpa-onnx-c-api.so through its own `$ORIGIN`, which is satisfied in the
build tree and not in an archive that forgot to ship those libraries beside it.

"Loads" is necessary but not sufficient. The host refuses a plugin whose ABI is
not exactly its own, and resolves several symbols after that check, so a plugin
can load cleanly and still be rejected at runtime. Both the expected ABI and
the required symbols are therefore read out of the host source rather than
restated here: a copy would drift, and the drift would be invisible until a
release shipped an engine the binary beside it refuses to load.

Run this from a directory that is not the build tree, with LD_LIBRARY_PATH
unset, or it can pass by finding libraries the end user will not have.
"""

from __future__ import annotations

import ctypes
import re
import sys
from pathlib import Path

# Resolved from this file, not the working directory: the caller deliberately
# runs from outside the build tree so the plugin cannot resolve its libraries
# through a path the end user will not have.
HOST_SOURCE = Path(__file__).resolve().parent.parent / "crates/core/src/sherpa_plugin.rs"


def host_abi_version(source: str) -> int:
    match = re.search(r"const\s+EXPECTED_ABI_VERSION:\s*u32\s*=\s*(\d+)\s*;", source)
    if match is None:
        raise SystemExit(
            f"could not find EXPECTED_ABI_VERSION in {HOST_SOURCE}; "
            "this check derives it from the host and cannot guess"
        )
    return int(match.group(1))


def host_required_symbols(source: str) -> list[str]:
    """Every symbol the host resolves, in the order it resolves them.

    `library.get(b"...")` is the only way the loader takes a symbol, so this
    tracks the host automatically as its FFI surface grows.
    """
    symbols = re.findall(r'\.get\(b"([A-Za-z0-9_]+)"\)', source)
    if not symbols:
        raise SystemExit(
            f"found no resolved symbols in {HOST_SOURCE}; refusing to pass a "
            "plugin on the strength of a check that verifies nothing"
        )
    # Preserve order, drop duplicates.
    return list(dict.fromkeys(symbols))


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {Path(argv[0]).name} <path-to-plugin>", file=sys.stderr)
        return 2

    if not HOST_SOURCE.is_file():
        print(
            f"{HOST_SOURCE} not found; run this from the repository root so the "
            "expected ABI and symbols can be read from the host",
            file=sys.stderr,
        )
        return 2
    source = HOST_SOURCE.read_text(encoding="utf8")
    expected_abi = host_abi_version(source)
    required = host_required_symbols(source)

    plugin = Path(argv[1])
    if not plugin.is_file():
        print(f"plugin not found at {plugin}", file=sys.stderr)
        return 1

    try:
        library = ctypes.CDLL(str(plugin))
    except OSError as error:
        # Almost always a missing sherpa shared library next to the plugin.
        print(f"plugin at {plugin} could not be loaded: {error}", file=sys.stderr)
        return 1

    missing = [name for name in required if not hasattr(library, name)]
    if missing:
        print(
            f"{plugin} loaded but does not export {', '.join(missing)}, which "
            f"{HOST_SOURCE} resolves; the host would reject it",
            file=sys.stderr,
        )
        return 1

    abi = library.minutes_sherpa_abi_version
    abi.restype = ctypes.c_uint32
    version = abi()
    if version != expected_abi:
        print(
            f"{plugin} reports ABI {version}, but the host in {HOST_SOURCE} "
            f"requires exactly {expected_abi}; shipping this would publish an "
            "engine the binary beside it refuses to load",
            file=sys.stderr,
        )
        return 1

    print(
        f"plugin loaded from {plugin}: ABI {version} matches the host, "
        f"and all {len(required)} required symbols are exported"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
