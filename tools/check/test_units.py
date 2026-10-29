import contextlib
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.check import units

PASSING = "".join(f"    def test_{index}(self): pass\n" for index in range(6))
MODULE = "import unittest\n\nclass Tests(unittest.TestCase):\n{}"


class UnitShardTests(unittest.TestCase):
    def run_suite(self, name, files):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(sys, "path", list(sys.path)),
            patch.dict(sys.modules),
        ):
            # Discovery caches packages by name.
            package = Path(directory) / name
            package.mkdir()
            (package / "__init__.py").touch()
            for file, text in files.items():
                (package / file).write_text(text)
            output = io.StringIO()
            previous = os.getcwd()
            os.chdir(directory)
            try:
                with (
                    patch(
                        "sys.argv",
                        ["units", "-s", name, "-p", "test_*.py"],
                    ),
                    contextlib.redirect_stdout(output),
                    contextlib.redirect_stderr(output),
                ):
                    return units.main(), output.getvalue()
            finally:
                os.chdir(previous)

    def test_exit_status(self):
        passing = {"test_pass.py": MODULE.format(PASSING)}
        failing = MODULE.format(PASSING + "    def test_x(self): self.fail()\n")
        skipped = "import unittest\nraise unittest.SkipTest('off')\n"
        for name, files, code, text in [
            ("passing", passing, 0, "OK"),
            ("failing", {"test_fail.py": failing}, 1, "Failed shards: 1"),
            ("skipped", passing | {"test_skip.py": skipped}, 0, "skipped=1"),
            ("broken", passing | {"test_bad.py": "import gone\n"}, 1, "gone"),
            ("empty", {}, 1, "No tests found"),
        ]:
            with self.subTest(name=name):
                result, output = self.run_suite(name, files)
                self.assertEqual(result, code)
                self.assertIn(text, output)
                if files:
                    self.assertEqual(output.count("Ran "), units.SHARDS)


if __name__ == "__main__":
    unittest.main()
