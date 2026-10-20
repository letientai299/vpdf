import csv
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from types import SimpleNamespace

import compare
import metrics


class HarnessTests(unittest.TestCase):
    def test_quantiles_and_skips(self):
        samples = [dict(page=0, destination=None, milliseconds=None, preparation_ms=None,
                        text_cache_bytes=0, ready_ms=None)]
        row = dict(document="/a/manual.pdf", case="hover", prepared="list", attempts=1,
                   samples=samples, screen=[120, 40, 8, 16], rendering=False)
        with self.assertRaisesRegex(ValueError, "no timings"):
            metrics.read("first-idle", json.dumps(row))
        samples.append(dict(page=1, destination=None, milliseconds=2.0, preparation_ms=.5,
                            text_cache_bytes=0, ready_ms=None))
        row["attempts"] = 2
        values = metrics.read("first-idle", json.dumps(row))
        self.assertEqual(values["/a/manual.pdf.hover.list.skipped"], 1)
        self.assertEqual(values["/a/manual.pdf.hover.list.milliseconds.n"], 1)
        self.assertEqual(metrics.quantiles(list(range(1, 21)))["p95"], 19)
        with self.assertRaises(ValueError):
            metrics.quantiles([float("nan")])
        self.assertEqual(metrics.quantiles([]), {})

    def test_workload_signature(self):
        row = dict(document="/a/manual.pdf", case="hover", prepared="list", attempts=1,
                   screen=[120, 40, 8, 16], rendering=False,
                   samples=[dict(page=0, destination=None, milliseconds=1)])
        first = compare.signature("first-idle", json.dumps(row))
        row["samples"][0]["milliseconds"] = 3
        self.assertEqual(first, compare.signature("first-idle", json.dumps(row)))
        row["samples"][0]["page"] = 1
        self.assertNotEqual(first, compare.signature("first-idle", json.dumps(row)))

    def test_paired_summary_and_failures(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "metrics.csv"
            records = {("latency", "time"): {"baseline": {0: 1, 1: 100, 2: 3},
                                           "candidate": {0: 3, 1: 101, 2: 4}}}
            metrics.write_csv(path, records)
            with path.open() as file:
                row = next(csv.DictReader(file))
            self.assertEqual(float(row["delta"]), 1)
            self.assertEqual(float(row["paired_median_delta"]), 1)
            path.unlink()
            del records["latency", "time"]["candidate"][2]
            with self.assertRaisesRegex(ValueError, "Unpaired"):
                metrics.write_csv(path, records)
            self.assertFalse(path.exists())

    def test_repeated_stage_identity(self):
        rows = [dict(document="/a/manual.pdf", page=1, attempt=i, stages=dict(load=i + 1))
                for i in range(8)]
        values = metrics.read("stages", json.dumps(rows))
        self.assertEqual(len([key for key in values if key.endswith(".load")]), 8)
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            metrics.read("stages", json.dumps(rows + rows[:1]))

    def test_document_name_collisions(self):
        rows = [dict(document=f"/{folder}/manual.pdf", pages=1, open_ms=value,
                     screen=[120, 40, 8, 16]) for folder, value in [("a", 1), ("b", 2)]]
        values = metrics.read("latency", json.dumps(rows))
        self.assertEqual(values["/a/manual.pdf.open_ms"], 1)
        self.assertEqual(values["/b/manual.pdf.open_ms"], 2)

    def test_per_child_rss_units(self):
        usage = SimpleNamespace(ru_utime=1.0, ru_stime=.5, ru_maxrss=4096,
                                ru_nvcsw=3, ru_nivcsw=2)
        self.assertEqual(compare.usage_values(usage, "Darwin")["peak_rss_kib"], 4)
        self.assertEqual(compare.usage_values(usage, "Linux")["peak_rss_kib"], 4096)
        self.assertEqual(compare.usage_values(usage, "Linux")["cpu_total_s"], 1.5)

    def test_group_timeout_and_reaping(self):
        child = subprocess.Popen(["sh", "-c", "exec sleep 30"], start_new_session=True)
        with self.assertRaises(TimeoutError):
            compare.wait(child, .01)
        self.assertEqual(child.poll(), -9)
        with self.assertRaises(ChildProcessError):
            compare.os.waitpid(child.pid, compare.os.WNOHANG)


if __name__ == "__main__":
    unittest.main()
