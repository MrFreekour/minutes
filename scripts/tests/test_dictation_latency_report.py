import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "dictation_latency_report.py"
SPEC = importlib.util.spec_from_file_location("dictation_latency_report", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class DictationLatencyReportTests(unittest.TestCase):
    def test_percentiles_use_nearest_rank(self):
        self.assertEqual(MODULE.percentile([40, 10, 30, 20], 50), 20)
        self.assertEqual(MODULE.percentile([40, 10, 30, 20], 95), 40)
        self.assertIsNone(MODULE.percentile([], 95))

    def test_report_reads_only_latency_extras(self):
        records = [
            {
                "step": "dictation_latency",
                "file": "must-not-appear",
                "extra": {
                    "release_to_visible_ms": 400,
                    "press_to_listening_ms": 250,
                    "target_class": "document",
                    "outcome": "typed",
                    "method": "native_ax",
                    "engine_warm": True,
                },
            },
            {
                "step": "dictation_latency",
                "extra": {
                    "release_to_visible_ms": 900,
                    "press_to_listening_ms": 300,
                    "target_class": "terminal",
                    "outcome": "pasted",
                    "method": "clipboard_paste",
                    "engine_warm": False,
                },
            },
            {"step": "transcribe", "extra": {"release_to_visible_ms": 1}},
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "minutes.log"
            path.write_text("\n".join(json.dumps(record) for record in records), encoding="utf-8")
            report = MODULE.build_report(MODULE.load_samples(path))

        self.assertEqual(report["sample_count"], 2)
        self.assertEqual(report["metrics"]["release_to_visible_ms"]["p50"], 400)
        self.assertEqual(report["metrics"]["release_to_visible_ms"]["p95"], 900)
        self.assertEqual(report["gates"]["release_to_visible_ms"]["status"], "fail")
        self.assertNotIn("must-not-appear", json.dumps(report))

    def test_cli_can_limit_report_to_recent_acceptance_samples(self):
        records = [
            {
                "step": "dictation_latency",
                "extra": {
                    "release_to_visible_ms": latency,
                    "press_to_listening_ms": 250,
                    "outcome": "typed",
                    "method": "native_ax",
                },
            }
            for latency in (1200, 240, 260)
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "minutes.log"
            path.write_text("\n".join(json.dumps(record) for record in records), encoding="utf-8")
            completed = subprocess.run(
                [sys.executable, str(SCRIPT), "--log", str(path), "--last", "2"],
                check=True,
                capture_output=True,
                text=True,
            )

        report = json.loads(completed.stdout)
        self.assertEqual(report["sample_count"], 2)
        self.assertEqual(report["metrics"]["release_to_visible_ms"]["p95"], 260)
        self.assertEqual(report["gates"]["release_to_visible_ms"]["status"], "pass")


if __name__ == "__main__":
    unittest.main()
