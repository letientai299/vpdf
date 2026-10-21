import os
import subprocess
import unittest
from pathlib import Path


class BuildFeatureTests(unittest.TestCase):
    def test_injection_requires_opt_in(self):
        root = Path(__file__).resolve().parent.parent
        for value, wanted in [
            (None, ["--no-default-features"]),
            ("0", ["--no-default-features"]),
            ("", ["--no-default-features"]),
            ("1", ["--no-default-features", "--features", "bench-events"]),
        ]:
            with self.subTest(value=value):
                env = dict(os.environ)
                env.pop("VPDF_BENCH_EVENTS", None)
                if value is not None:
                    env["VPDF_BENCH_EVENTS"] = value
                run = subprocess.run(
                    ["sh", "-c", '. scripts/build-features.sh; printf "%s\\n" "$@"'],
                    cwd=root, env=env, capture_output=True, text=True, check=True,
                )
                self.assertEqual(run.stdout.splitlines(), wanted)
        env["VPDF_BENCH_EVENTS"] = "true"
        run = subprocess.run(
            ["sh", "-c", ". scripts/build-features.sh"],
            cwd=root, env=env, capture_output=True, text=True,
        )
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("VPDF_BENCH_EVENTS requires 0 or 1", run.stderr)
