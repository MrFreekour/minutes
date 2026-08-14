#!/usr/bin/env python3
"""Summarize privacy-safe local dictation latency events.

The source records contain timings, coarse target classes, and insertion
outcomes only. This report intentionally ignores the generic log `file` field
and never reads dictation history or transcript artifacts.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import Counter
from pathlib import Path
from typing import Any


METRICS = (
    "press_to_hud_ms",
    "press_to_engine_ready_ms",
    "press_to_listening_ms",
    "speech_to_first_partial_ms",
    "speech_end_to_final_ms",
    "speech_end_to_visible_ms",
    "release_to_final_ms",
    "release_to_visible_ms",
    "final_to_visible_ms",
    "final_to_verification_complete_ms",
    "visible_insert_operation_ms",
    "clipboard_restore_ms",
    "verification_ms",
    "insertion_total_ms",
    "session_total_ms",
)

GATES = {
    "press_to_listening_ms": 500,
    "release_to_visible_ms": 750,
}


def percentile(values: list[int], percent: int) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil((percent / 100) * len(ordered)))
    return ordered[rank - 1]


def load_samples(path: Path) -> list[dict[str, Any]]:
    samples: list[dict[str, Any]] = []
    if not path.exists():
        return samples
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("step") != "dictation_latency":
            continue
        extra = record.get("extra")
        if isinstance(extra, dict):
            samples.append(extra)
    return samples


def build_report(samples: list[dict[str, Any]]) -> dict[str, Any]:
    metric_report: dict[str, Any] = {}
    for metric in METRICS:
        values = [
            int(sample[metric])
            for sample in samples
            if isinstance(sample.get(metric), (int, float))
            and not isinstance(sample.get(metric), bool)
        ]
        metric_report[metric] = {
            "samples": len(values),
            "p50": percentile(values, 50),
            "p90": percentile(values, 90),
            "p95": percentile(values, 95),
            "max": max(values) if values else None,
        }

    gates: dict[str, Any] = {}
    for metric, threshold_ms in GATES.items():
        p95 = metric_report[metric]["p95"]
        gates[metric] = {
            "threshold_p95_ms": threshold_ms,
            "observed_p95_ms": p95,
            "status": "insufficient_data" if p95 is None else ("pass" if p95 <= threshold_ms else "fail"),
        }

    return {
        "sample_count": len(samples),
        "metrics": metric_report,
        "gates": gates,
        "outcomes": dict(sorted(Counter(str(sample.get("outcome", "unknown")) for sample in samples).items())),
        "methods": dict(sorted(Counter(str(sample.get("method", "unknown")) for sample in samples).items())),
        "target_classes": dict(sorted(Counter(str(sample.get("target_class", "unknown")) for sample in samples).items())),
        "engine_cache": dict(
            sorted(
                Counter(
                    "warm" if sample.get("engine_warm") is True else "cold" if sample.get("engine_warm") is False else "unknown"
                    for sample in samples
                ).items()
            )
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--log",
        type=Path,
        default=Path.home() / ".minutes" / "logs" / "minutes.log",
        help="Minutes JSONL log containing dictation_latency steps",
    )
    parser.add_argument(
        "--last",
        type=int,
        metavar="N",
        help="report only the N most recent latency samples",
    )
    args = parser.parse_args()
    if args.last is not None and args.last < 1:
        parser.error("--last must be at least 1")
    samples = load_samples(args.log)
    if args.last is not None:
        samples = samples[-args.last :]
    print(json.dumps(build_report(samples), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
