import os
import subprocess
import unittest
from pathlib import Path


class BuildFeatureTests(unittest.TestCase):
    def test_shipped_features_per_package(self):
        result = subprocess.run(
            [
                "sh",
                "-c",
                ". tools/build/features.sh; "
                'printf %s "$CARGO_RESOLVER_FEATURE_UNIFICATION"',
            ],
            cwd=Path(__file__).resolve().parents[2],
            env=dict(
                os.environ, CARGO_RESOLVER_FEATURE_UNIFICATION="workspace"
            ),
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(result.stdout, "selected")

    def test_debug_rejects_profiles(self):
        for flag in ["--release", "-r", "--profile=app", "--target=x"]:
            with self.subTest(flag=flag):
                result = subprocess.run(
                    ["sh", "tools/build/debug.sh", "vpdf", flag],
                    cwd=Path(__file__).resolve().parents[2],
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 1)
                self.assertIn("no profile or target", result.stderr)

    def test_caller_arguments_preserved(self):
        for value in ["0", "1"]:
            with self.subTest(value=value):
                result = subprocess.run(
                    [
                        "sh",
                        "-c",
                        '. tools/build/features.sh; printf "%s\\n" "$@"',
                        "build",
                        "--timings",
                        "--target-dir",
                        "path with spaces",
                    ],
                    cwd=Path(__file__).resolve().parents[2],
                    env=dict(os.environ, VPDF_BENCH_EVENTS=value),
                    capture_output=True,
                    text=True,
                    check=True,
                )
                wanted = [
                    "--no-default-features",
                    "--timings",
                    "--target-dir",
                    "path with spaces",
                ]
                if value == "1":
                    wanted.extend(["--features", "bench-events"])
                self.assertEqual(result.stdout.splitlines(), wanted)

    def test_injection_requires_opt_in(self):
        root = Path(__file__).resolve().parents[2]
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
                    [
                        "sh",
                        "-c",
                        '. tools/build/features.sh; printf "%s\\n" "$@"',
                    ],
                    cwd=root,
                    env=env,
                    capture_output=True,
                    text=True,
                    check=True,
                )
                self.assertEqual(run.stdout.splitlines(), wanted)
        env["VPDF_BENCH_EVENTS"] = "true"
        run = subprocess.run(
            ["sh", "-c", ". tools/build/features.sh"],
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("VPDF_BENCH_EVENTS requires 0 or 1", run.stderr)
