import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.package.stable import validate, verify_version, version_text
from tools.package.test_notes import Repository


class StableTests(unittest.TestCase):
    def test_release_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(
                '[package]\nversion = "0.2.0"\nrepository = "https://example.com/vpdf"\n'
            )
            validate("v0.1.0")
            output = "vpdf 0.1.0\n\n# Vpdf v0.1.0\n\nFeatures\n\nhttps://example.com/vpdf\n"
            with patch(
                "tools.package.stable.render",
                return_value="# Vpdf v0.1.0\n\nFeatures",
            ):
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
                    validate(tag)

    def test_generated_notes(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Repository(directory)
            repo.commit("feat: first release", tag="v0.1.0")
            self.assertEqual(
                version_text(repo.root, "v0.1.0"),
                "vpdf 0.1.0\n\n# Vpdf v0.1.0\n\n## Features\n\n"
                "- First release\n\nhttps://example.com/vpdf\n",
            )
