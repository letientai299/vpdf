import tempfile
import unittest
from unittest.mock import Mock, patch

from provenance import checkout, embedded


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
        with tempfile.TemporaryDirectory() as directory:
            self.assertIsNone(checkout(directory)["runtime_checkout_dirty"])
        for effect in (FileNotFoundError(), None):
            with patch(
                "provenance.subprocess.run",
                side_effect=effect,
                return_value=Mock(returncode=1, stdout=""),
            ):
                self.assertIsNone(checkout(".")["runtime_checkout_dirty"])
