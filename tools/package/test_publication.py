import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.package import publication


class PublicationTests(unittest.TestCase):
    def test_immutable_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, rebuilt = root / "first", root / "rebuilt"
            for path, payload in [(first, b"original"), (rebuilt, b"rebuilt")]:
                path.mkdir()
                for platform in publication.PLATFORMS:
                    name = f"vpdf-{platform}.tar.gz"
                    (path / name).write_bytes(payload)
                    (path / f"{name}.sha256").write_text(
                        f"{publication.digest(path / name)}  {name}\n"
                    )
            source = dict(
                tag="v0.1.0", revision="abc", changelog_sha256="notes"
            )
            with patch.object(publication, "identity", return_value=source):
                for path in [first, rebuilt]:
                    publication.write(root, "v0.1.0", path)
                    publication.verify(root, "v0.1.0", path)
                original = (first / "source.json").read_bytes()
                publication.verify(root, "v0.1.0", first)
                self.assertEqual((first / "source.json").read_bytes(), original)
                with patch.object(
                    publication,
                    "identity",
                    return_value=source | dict(revision="other"),
                ):
                    with self.assertRaisesRegex(ValueError, "source identity"):
                        publication.verify(root, "v0.1.0", first)
                manifest = json.loads(original)
                manifest["assets"]["vpdf-linux-x86_64.tar.gz"] = "wrong"
                (first / "source.json").write_text(json.dumps(manifest))
                with self.assertRaisesRegex(ValueError, "asset manifest"):
                    publication.verify(root, "v0.1.0", first)
                (first / "vpdf-linux-x86_64.tar.gz").write_bytes(b"corrupt")
                with self.assertRaisesRegex(ValueError, "checksum"):
                    publication.verify(root, "v0.1.0", first)
