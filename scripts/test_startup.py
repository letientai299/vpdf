import json
import tempfile
import unittest
from pathlib import Path

import startup


class TraceTests(unittest.TestCase):
    def test_completion_ignores_other_rows(self):
        for tagged in (False, True):
            with self.subTest(tagged=tagged), tempfile.TemporaryDirectory() as directory:
                frames = [{"complete": False}, {"complete": True}]
                if tagged:
                    for frame in frames:
                        frame["kind"] = "frame"
                    frames.insert(0, {"kind": "sample", "rss": 1})
                    frames.insert(2, {"kind": "input", "step": 1})
                    frames.insert(3, {"kind": "mark", "name": "ready"})
                path = Path(directory) / "frames.jsonl"
                path.write_text("\n".join(json.dumps(row) for row in frames))
                row = {
                    "failure": None,
                    "settled_tiles": 1,
                    "complete_ms": 10,
                    "frames": [{"ms": 5}, {"ms": 15}],
                }
                startup.traced(row, path)
                self.assertIsNone(row["failure"])
                self.assertEqual(row["placed_ms"], 10)
                self.assertEqual(row["complete_ms"], 15)
