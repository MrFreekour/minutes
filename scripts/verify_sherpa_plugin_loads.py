#!/usr/bin/env python3
"""Prove a packaged sherpa plugin actually dlopens, and report its ABI.

The plugin is loaded lazily, and only once sherpa is genuinely selected, so a
packaged CLI can run `--version` perfectly while its plugin is unloadable. The
failure mode is loader search paths: the plugin resolves libsherpa-onnx-c-api
through its own `$ORIGIN`, which is right in the build tree and wrong in an
archive that forgot to ship those libraries beside it.

Run this from a directory that is not the build tree, with LD_LIBRARY_PATH
unset, or it can pass by finding libraries the end user will not have.
"""

import ctypes
import sys
from pathlib import Path

# Matches EXPECTED_ABI_VERSION in crates/core/src/sherpa_plugin.rs. Loading is
# the point of this check, so the bound is deliberately loose: a mismatch is
# the host's business to report, not this script's.
MINIMUM_PLAUSIBLE_ABI = 1


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {Path(argv[0]).name} <path-to-plugin>", file=sys.stderr)
        return 2

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

    try:
        abi = library.minutes_sherpa_abi_version
    except AttributeError:
        print(
            f"{plugin} loaded but exports no minutes_sherpa_abi_version, "
            "so it is not a Minutes sherpa plugin",
            file=sys.stderr,
        )
        return 1

    abi.restype = ctypes.c_uint32
    version = abi()
    if version < MINIMUM_PLAUSIBLE_ABI:
        print(f"{plugin} reports implausible ABI version {version}", file=sys.stderr)
        return 1

    print(f"plugin loaded from {plugin}, ABI {version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
