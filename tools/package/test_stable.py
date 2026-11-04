import tempfile
import unittest
from pathlib import Path

from tools.package.stable import validate, verify_version


class StableTests(unittest.TestCase):
    def test_release_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[package]\nversion = "0.2.0"\nrepository = "https://example.com/vpdf"\n'
            )
            (root / "changelogs").mkdir()
            notes = root / "changelogs/v0.1.0.md"
            notes.write_text("# Vpdf v0.1.0\n\nFeatures\n")
            validate(root, "v0.1.0")
            output = "vpdf 0.1.0\n\n# Vpdf v0.1.0\n\nFeatures\n\nhttps://example.com/vpdf\n"
            verify_version(root, "v0.1.0", output)
            for wrong in [
                output.replace("Features", "Old notes"),
                output.replace("vpdf 0.1.0", "vpdf dev"),
            ]:
                with self.assertRaisesRegex(ValueError, "Packaged version"):
                    verify_version(root, "v0.1.0", wrong)
            for tag in [
                "dev",
                "snapshot-abc",
                "v0.1.0-rc.1",
                "v01.1.0",
            ]:
                with self.subTest(tag=tag), self.assertRaises(ValueError):
                    validate(root, tag)
            for content in ["# Vpdf v0.2.0\n\nFeatures", "# Vpdf v0.1.0\n\n"]:
                notes.write_text(content)
                with (
                    self.subTest(content=content),
                    self.assertRaises(ValueError),
                ):
                    validate(root, "v0.1.0")
