#!/usr/bin/env python3
"""Run the signed Apple Speech byte path under a same-UID open-holder watcher."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import plistlib
import re
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time


MAX_HELD_FILE_BYTES = 64 * 1024 * 1024
EXPECTED_TEAM_ID = "63TMLKT8HN"
EXPECTED_APP_IDENTIFIERS = {
    "com.useminutes.desktop",
    "com.useminutes.desktop.dev",
}
EXPECTED_WORKER_IDENTIFIER = "com.useminutes.apple-speech-worker"

# The app spends up to two full worker wall clocks (2 x 180s) inside the two
# authenticated probes, before app launch and two strict nested code-signature
# validations of the whole bundle. The previous 420s budget left roughly 60s
# for all of that, so a slow runner would have surfaced as an evidence-free
# timeout rather than a real result.
ACCEPTANCE_TIMEOUT_SECONDS = 900
DIAGNOSTIC_STREAM_LIMIT = 32 * 1024
DIAGNOSTIC_LOG_LINES = 400
CRASH_REPORT_DIRECTORIES = (
    pathlib.Path.home() / "Library/Logs/DiagnosticReports",
    pathlib.Path("/Library/Logs/DiagnosticReports"),
)
CRASH_REPORT_PREFIXES = ("minutes-apple-speech-worker", "minutes-graph-worker", "Minutes")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", required=True, type=pathlib.Path)
    parser.add_argument("--candidate-sha", required=True)
    return parser.parse_args()


def codesign_details(path: pathlib.Path) -> str:
    result = subprocess.run(
        ["codesign", "-dvvv", str(path)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    return result.stdout


def detail_value(details: str, key: str) -> str:
    match = re.search(rf"^{re.escape(key)}=(.+)$", details, re.MULTILINE)
    if match is None:
        raise RuntimeError(f"missing signed identity field: {key}")
    return match.group(1).strip()


def signed_paths(app: pathlib.Path) -> tuple[pathlib.Path, pathlib.Path]:
    with (app / "Contents" / "Info.plist").open("rb") as source:
        executable_name = plistlib.load(source)["CFBundleExecutable"]
    executable = app / "Contents" / "MacOS" / executable_name
    worker = (
        app
        / "Contents"
        / "XPCServices"
        / "com.useminutes.apple-speech-worker.xpc"
        / "Contents"
        / "MacOS"
        / "minutes-apple-speech-worker"
    )
    if not executable.is_file() or not worker.is_file():
        raise RuntimeError("signed app or Apple Speech worker executable is missing")
    return executable, worker


def verify_signed_identity(
    app: pathlib.Path,
    executable: pathlib.Path,
    worker: pathlib.Path,
) -> str:
    subprocess.run(
        ["codesign", "--verify", "--strict", "--verbose=4", str(app)],
        check=True,
    )
    app_details = codesign_details(executable)
    worker_details = codesign_details(worker)
    app_identifier = detail_value(app_details, "Identifier")
    if app_identifier not in EXPECTED_APP_IDENTIFIERS:
        raise RuntimeError("signed runtime app identifier is not allowlisted")
    if detail_value(app_details, "TeamIdentifier") != EXPECTED_TEAM_ID:
        raise RuntimeError("signed runtime app Team ID is not allowlisted")
    if detail_value(worker_details, "Identifier") != EXPECTED_WORKER_IDENTIFIER:
        raise RuntimeError("signed runtime Apple Speech worker identifier is incorrect")
    if detail_value(worker_details, "TeamIdentifier") != EXPECTED_TEAM_ID:
        raise RuntimeError("signed runtime Apple Speech worker Team ID is incorrect")
    worker_cdhash = detail_value(worker_details, "CDHash").lower()
    if re.fullmatch(r"[0-9a-f]{40}", worker_cdhash) is None:
        raise RuntimeError("signed runtime Apple Speech worker CDHash is invalid")
    recorded = (
        app / "Contents" / "Resources" / "minutes-apple-speech-worker.cdhash"
    ).read_text(encoding="ascii").strip()
    if recorded != worker_cdhash:
        raise RuntimeError("signed runtime Apple Speech worker CDHash receipt is stale")
    marker = b"MINUTES_APPLE_SPEECH_WORKER_CDHASH_V1=" + worker_cdhash.encode(
        "ascii"
    )
    if executable.read_bytes().count(marker) != 1:
        raise RuntimeError("signed runtime parent is not bound to the exact worker")
    return worker_cdhash


def user_temp_root() -> pathlib.Path:
    result = subprocess.run(
        ["getconf", "DARWIN_USER_TEMP_DIR"],
        check=True,
        capture_output=True,
        text=True,
    )
    root = pathlib.Path(result.stdout.strip()).resolve()
    if not root.is_dir():
        raise RuntimeError("Darwin user temporary directory is unavailable")
    return root


def regular_file_keys(root: pathlib.Path) -> set[tuple[int, int]]:
    keys: set[tuple[int, int]] = set()
    for directory, _, names in os.walk(root, followlinks=False):
        for name in names:
            path = pathlib.Path(directory) / name
            try:
                info = path.lstat()
            except OSError:
                continue
            if stat.S_ISREG(info.st_mode):
                keys.add((info.st_dev, info.st_ino))
    return keys


class SameUidOpenHolder:
    def __init__(self, roots: list[pathlib.Path]) -> None:
        self.roots = roots
        self.baseline = set().union(*(regular_file_keys(root) for root in roots))
        self.seen = set(self.baseline)
        self.held: list[int] = []
        self.stop = threading.Event()
        self.ready = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self.thread.start()
        if not self.ready.wait(timeout=5):
            raise RuntimeError("same-UID open-holder watcher did not start")

    def close(self) -> list[bytes]:
        self.stop.set()
        self.thread.join(timeout=5)
        if self.thread.is_alive():
            raise RuntimeError("same-UID open-holder watcher did not stop")
        contents: list[bytes] = []
        for descriptor in self.held:
            try:
                os.lseek(descriptor, 0, os.SEEK_SET)
                chunks = []
                remaining = MAX_HELD_FILE_BYTES
                while remaining > 0:
                    chunk = os.read(descriptor, min(1024 * 1024, remaining))
                    if not chunk:
                        break
                    chunks.append(chunk)
                    remaining -= len(chunk)
                contents.append(b"".join(chunks))
            finally:
                os.close(descriptor)
        return contents

    def _run(self) -> None:
        self.ready.set()
        while not self.stop.is_set():
            for root in self.roots:
                for directory, _, names in os.walk(root, followlinks=False):
                    for name in names:
                        path = pathlib.Path(directory) / name
                        try:
                            info = path.lstat()
                        except OSError:
                            continue
                        key = (info.st_dev, info.st_ino)
                        if key in self.seen or not stat.S_ISREG(info.st_mode):
                            continue
                        self.seen.add(key)
                        try:
                            descriptor = os.open(
                                path,
                                os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
                            )
                        except OSError:
                            continue
                        self.held.append(descriptor)
            self.stop.wait(0.002)


def canary_patterns() -> tuple[bytes, bytes]:
    bits = [0x3E000000 + (index % 4096) for index in range(4096)]
    raw_f32 = b"".join(struct.pack("<I", value) for value in bits)
    pcm_i16 = b"".join(
        struct.pack(
            "<h",
            int(struct.unpack("<f", struct.pack("<I", value))[0] * 32767.0),
        )
        for value in bits
    )
    return raw_f32, pcm_i16


def decode_stream(stream) -> str:
    """Normalize a captured stream that may be bytes, str, or absent."""
    if stream is None:
        return ""
    if isinstance(stream, bytes):
        return stream.decode("utf-8", "replace")
    return stream


def bounded_stream(stream: str) -> str:
    """Bound a captured stream so one runaway helper cannot flood the log."""
    text = decode_stream(stream)
    if not text:
        return "<empty>"
    if len(text) <= DIAGNOSTIC_STREAM_LIMIT:
        return text
    return f"<truncated to final {DIAGNOSTIC_STREAM_LIMIT} characters>\n" + text[
        -DIAGNOSTIC_STREAM_LIMIT:
    ]


def signal_label(returncode) -> str | None:
    """Render a negative exit status as its terminating signal name."""
    if returncode is None or returncode >= 0:
        return None
    try:
        return signal.Signals(-returncode).name
    except ValueError:
        return f"signal {-returncode}"


def unified_log_excerpt(started_at: float) -> str:
    """Collect helper-scoped unified log entries emitted during the run."""
    start = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(started_at))
    predicate = (
        'process == "minutes-apple-speech-worker"'
        ' OR process == "minutes-graph-worker"'
        ' OR subsystem == "com.apple.xpc.launchd"'
    )
    try:
        completed = subprocess.run(
            ["log", "show", "--start", start, "--style", "compact", "--predicate", predicate],
            capture_output=True,
            text=True,
            timeout=120,
        )
    except (OSError, subprocess.SubprocessError) as error:
        return f"<unified log unavailable: {error}>"
    lines = completed.stdout.splitlines()
    if not lines:
        return f"<no matching entries; log exited {completed.returncode}>"
    return "\n".join(lines[:DIAGNOSTIC_LOG_LINES])


def crash_report_excerpts(started_at: float) -> list[tuple[pathlib.Path, str]]:
    """Return crash reports for our own processes written during this run."""
    reports = []
    for directory in CRASH_REPORT_DIRECTORIES:
        try:
            entries = sorted(directory.iterdir())
        except OSError:
            continue
        for entry in entries:
            if entry.suffix not in {".ips", ".crash"}:
                continue
            if not entry.name.startswith(CRASH_REPORT_PREFIXES):
                continue
            try:
                if entry.stat().st_mtime < started_at:
                    continue
                reports.append((entry, entry.read_text("utf-8", "replace")))
            except OSError as error:
                reports.append((entry, f"<unreadable: {error}>"))
    return reports


def emit_failure_diagnostics(started_at: float, returncode, stdout, stderr) -> None:
    """Print helper failure evidence to stderr.

    Diagnostics must never reach stdout: the workflow tees stdout into
    ``signed-runtime-provenance.json``, so anything printed there would corrupt
    the receipt. This is safe to emit in full because the runtime job holds no
    secrets and the only audio in the process is the synthetic canary, so no
    signing material and no private utterance can appear here.
    """
    def write(line: str) -> None:
        print(line, file=sys.stderr)

    write("=== signed Apple Speech acceptance diagnostics ===")
    write(f"exit status: {returncode if returncode is not None else 'timed out'}")
    terminator = signal_label(returncode)
    if terminator:
        write(f"terminated by: {terminator}")
    write("--- child stdout ---")
    write(bounded_stream(stdout))
    write("--- child stderr ---")
    write(bounded_stream(stderr))
    write("--- unified log ---")
    write(unified_log_excerpt(started_at))
    reports = crash_report_excerpts(started_at)
    if not reports:
        write("--- crash reports: none written during this run ---")
    for path, body in reports:
        write(f"--- crash report {path} ---")
        write(bounded_stream(body))
    write("=== end diagnostics ===")
    sys.stderr.flush()


def main() -> int:
    args = parse_args()
    if re.fullmatch(r"[0-9a-f]{40}", args.candidate_sha) is None:
        raise RuntimeError("candidate SHA must be full lowercase hex")
    app = args.app.resolve()
    executable, worker = signed_paths(app)
    worker_cdhash = verify_signed_identity(app, executable, worker)
    os_major = int(
        subprocess.run(
            ["sw_vers", "-productVersion"],
            check=True,
            capture_output=True,
            text=True,
        )
        .stdout.split(".", 1)[0]
    )
    if os_major < 26:
        raise RuntimeError("signed Apple Speech runtime acceptance requires macOS 26+")

    shared_temp = user_temp_root()
    with tempfile.TemporaryDirectory(
        prefix="minutes-apple-speech-acceptance-",
        dir=shared_temp,
    ) as isolated:
        isolated_temp = pathlib.Path(isolated).resolve()
        watcher = SameUidOpenHolder([isolated_temp, shared_temp])
        watcher.start()
        environment = os.environ.copy()
        environment["TMPDIR"] = str(isolated_temp)
        started_at = time.time()
        try:
            result = subprocess.run(
                [str(executable), "--apple-speech-transport-acceptance"],
                env=environment,
                capture_output=True,
                text=True,
                timeout=ACCEPTANCE_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired as expired:
            # TimeoutExpired's repr omits the captured streams, so a wedged
            # helper would otherwise destroy every byte of evidence.
            emit_failure_diagnostics(
                started_at,
                None,
                decode_stream(expired.stdout),
                decode_stream(expired.stderr),
            )
            raise RuntimeError(
                "signed Apple Speech transport acceptance timed out after "
                f"{ACCEPTANCE_TIMEOUT_SECONDS}s"
            ) from expired
        finally:
            time.sleep(0.1)
            held_contents = watcher.close()

    if result.returncode != 0:
        emit_failure_diagnostics(
            started_at, result.returncode, result.stdout, result.stderr
        )
        raise RuntimeError(
            "signed Apple Speech transport acceptance failed with "
            f"exit {result.returncode}: {result.stderr[-2000:]}"
        )
    if "apple-speech-signed-byte-transport=accepted" not in result.stdout:
        raise RuntimeError("signed app did not emit the content-free acceptance receipt")
    runtime_supported_match = re.search(
        r"^apple-speech-signed-runtime-supported=(true|false)$",
        result.stdout,
        re.MULTILINE,
    )
    if runtime_supported_match is None:
        raise RuntimeError("signed app did not report whether the Speech runtime was supported")
    runtime_supported = runtime_supported_match.group(1) == "true"
    raw_f32, pcm_i16 = canary_patterns()
    for contents in held_contents:
        if raw_f32 in contents or pcm_i16 in contents:
            raise RuntimeError(
                "same-UID open holder recovered the synthetic utterance from a named file"
            )

    receipt = {
        "candidateSha": args.candidate_sha,
        "appIdentifier": detail_value(codesign_details(executable), "Identifier"),
        "teamIdentifier": EXPECTED_TEAM_ID,
        "workerIdentifier": EXPECTED_WORKER_IDENTIFIER,
        "workerCdhash": worker_cdhash,
        "macosMajor": os_major,
        "sameUid": os.geteuid(),
        "heldNewRegularFiles": len(held_contents),
        "namedAudioCanaryObserved": False,
        "productGateExpectedClosed": True,
        "signedByteTransport": "accepted",
        # "accepted" attests that the authenticated byte path carried the
        # canary into the Swift bridge. It does not by itself attest that the
        # Speech analyzer ran: the bridge reports runtimeSupported false when
        # the framework declines after the bytes arrive. Record it so the
        # receipt cannot be read as more than it proves.
        "runtimeSupported": runtime_supported,
    }
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
