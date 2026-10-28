import os
import tempfile
import unittest
from unittest.mock import Mock, patch

from tools.bench.live.provenance import checkout, embedded


class ProvenanceTests(unittest.TestCase):
    def test_embedded_identity_ignores_checkout(self):
        self.assertEqual(
            embedded("build-source abc true def\nvpdf 0.1"),
            dict(revision="abc", dirty=True, patch="def"),
        )
        self.assertEqual(
            embedded("old binary"), dict(revision=None, dirty=None, patch=None)
        )

    def test_failed_git_is_unknown(self):
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("GIT_")
        }
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.dict(os.environ, env, clear=True),
        ):
            self.assertIsNone(checkout(directory)["runtime_checkout_dirty"])
        for effect in (FileNotFoundError(), None):
            with patch(
                "tools.bench.live.provenance.subprocess.run",
                side_effect=effect,
                return_value=Mock(returncode=1, stdout=""),
            ):
                self.assertIsNone(checkout(".")["runtime_checkout_dirty"])

    def test_explained_build_identity(self):
        for dirty, expected in [
            ("true", True),
            ("false", False),
            ("unknown", None),
        ]:
            with self.subTest(dirty=dirty):
                self.assertEqual(
                    embedded(
                        "Source revision: abc\nGit commit used for this build.\n"
                        f"Local changes: {dirty}\nSource patch: def\n"
                        "Help assets: fingerprint\n"
                    ),
                    dict(revision="abc", dirty=expected, patch="def"),
                )
        self.assertEqual(
            embedded("Source revision: unknown\nLocal changes: unknown\n"),
            dict(revision=None, dirty=None, patch=None),
        )
