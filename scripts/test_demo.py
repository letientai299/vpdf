import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import demo


class DemoTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.binary = self.root / "vpdf"
        self.binary.write_bytes(b"reader executable")
        (self.root / "paper.pdf").write_bytes(b"document input")

    def test_selected_dependencies(self):
        hashes = demo.input_hashes(self.root, ["figure", "mouse-menu"])
        self.assertEqual(set(hashes), {"paper"})
        self.assertEqual(
            set(demo.scene_documents("overview")), {"paper", "book", "scan"}
        )

    def test_executable_revision(self):
        reported = "a" * 40
        resolved = SimpleNamespace(returncode=0, stdout=reported + "\n")
        with (
            patch.object(
                demo,
                "run",
                return_value=(
                    "Reader description\nhttps://example.com\n"
                    "vpdf aaaaaaa (2026-10-18)"
                ),
            ),
            patch.object(subprocess, "run", return_value=resolved),
        ):
            identity = demo.binary_identity(self.binary, "debug")
        self.assertEqual(identity["commit"], reported)
        self.assertEqual(identity["build_profile"], "debug")
        self.assertEqual(identity["binary_sha256"], demo.digest(self.binary))

    def test_unknown_revision(self):
        with patch.object(demo, "run", return_value="vpdf 0.1.0"):
            identity = demo.binary_identity(self.binary, "unknown")
        self.assertIsNone(identity["commit"])
        self.assertEqual(identity["binary_version"], "vpdf 0.1.0")
        self.assertEqual(identity["build_profile"], "unknown")

    def test_changed_inputs(self):
        args = SimpleNamespace(binary=self.binary, corpus=self.root)
        identity = {
            "binary_sha256": demo.digest(self.binary),
            "inputs": demo.input_hashes(self.root, ["figure"]),
        }
        for path in (self.binary, self.root / "paper.pdf"):
            with self.subTest(path=path.name):
                original = path.read_bytes()
                path.write_bytes(b"changed after capture began")
                with self.assertRaises(RuntimeError):
                    demo.verify_inputs(args, identity)
                path.write_bytes(original)

    def test_selected_publication(self):
        output = self.root / "assets"
        output.mkdir()
        other = output / "search.jpg"
        other.write_bytes(b"unchanged image")
        before = other.stat().st_mtime_ns
        previous = {
            "image": other.name,
            "capture": "old",
            "sha256": demo.digest(other),
        }
        manifest = {
            "captures": {"old": {"commit": "previous"}},
            "scenes": {"search": previous},
        }
        image = self.root / "figure.jpg"
        image.write_bytes(b"new image")
        identity = {"commit": "current"}
        scene = {"image": image.name, "sha256": demo.digest(image)}
        updated = demo.publish_scene(
            output, manifest, identity, "figure", scene, image
        )
        self.assertEqual(updated["scenes"]["search"], previous)
        self.assertEqual(other.read_bytes(), b"unchanged image")
        self.assertEqual(other.stat().st_mtime_ns, before)
        self.assertEqual((output / image.name).read_bytes(), image.read_bytes())
        self.assertEqual(
            json.loads((output / "capture.json").read_text()), updated
        )
        self.assertNotIn("figure", manifest["scenes"])

    def test_manifest_failure(self):
        original_replace = Path.replace

        def fail_report(source, target):
            if source.name == "capture.json":
                raise OSError("manifest blocked")
            return original_replace(source, target)

        for existing in (False, True):
            with self.subTest(existing=existing):
                output = self.root / str(existing)
                output.mkdir()
                target = output / "figure.jpg"
                if existing:
                    target.write_bytes(b"previous image")
                report = output / "capture.json"
                report.write_text('{"previous": true}\n')
                manifest = {"captures": {}, "scenes": {}}
                image = self.root / "candidate.jpg"
                image.write_bytes(b"candidate image")
                scene = {"image": target.name}
                with (
                    patch.object(Path, "replace", new=fail_report),
                    self.assertRaises(OSError),
                ):
                    demo.publish_scene(
                        output, manifest, {}, "figure", scene, image
                    )
                self.assertEqual(report.read_text(), '{"previous": true}\n')
                self.assertEqual(target.exists(), existing)
                if existing:
                    self.assertEqual(target.read_bytes(), b"previous image")
                self.assertEqual(manifest, {"captures": {}, "scenes": {}})
                self.assertFalse(list(output.glob(".capture-*")))


@unittest.skipUnless(shutil.which("magick"), "ImageMagick required")
class OptimizerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "capture.png"
        self.target = self.root / "capture.jpg"

    def optimize(self):
        return subprocess.run(
            [
                "sh",
                str(demo.ROOT / "scripts/demo-optimize.sh"),
                str(self.source),
                str(self.target),
            ],
            capture_output=True,
            text=True,
        )

    def test_jpeg_output(self):
        subprocess.run(
            [
                "magick",
                "-size",
                "2560x1440",
                "gradient:red-blue",
                "-set",
                "comment",
                "private capture metadata",
                str(self.source),
            ],
            check=True,
        )
        original = self.source.read_bytes()
        result = self.optimize()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.source.read_bytes(), original)
        self.assertTrue(self.target.read_bytes().startswith(b"\xff\xd8"))
        decoded = subprocess.check_output(
            [
                "magick",
                str(self.target),
                "-format",
                "%m %wx%h %[jpeg:sampling-factor] %[interlace] %Q %c",
                "info:",
            ],
            text=True,
        )
        self.assertEqual(decoded.strip(), "JPEG 2560x1440 1x1,1x1,1x1 JPEG 92")
        self.assertEqual(self.target.stat().st_mode & 0o777, 0o644)
        self.assertFalse(list(self.root.glob("capture.jpg.*")))

    def test_invalid_capture(self):
        self.target.write_bytes(b"previous image")
        for size in ("1280x720", "1440x2560"):
            with self.subTest(size=size):
                subprocess.run(
                    ["magick", "-size", size, "xc:white", str(self.source)],
                    check=True,
                )
                result = self.optimize()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Capture must be 2560x1440", result.stderr)
                self.assertEqual(self.target.read_bytes(), b"previous image")
                self.assertFalse(list(self.root.glob("capture.jpg.*")))

    def test_failed_encoding(self):
        helper = self.root / "magick"
        helper.write_text(
            "#!/bin/sh\n"
            'case "$1" in\n'
            "  identify) printf 2560x1440 ;;\n"
            '  -regard-warnings) printf "%s" "$DEMO_DECODED" ;;\n'
            "  *)\n"
            "    for argument do target=$argument; done\n"
            '    printf partial > "${target#jpeg:}"\n'
            '    exit "$DEMO_ENCODE_STATUS" ;;\n'
            "esac\n"
        )
        helper.chmod(0o755)
        cases = ((1, ""), (0, "PNG 2560x1440"), (0, "JPEG 1280x720"))
        for existing in (False, True):
            for status, decoded in cases:
                with self.subTest(
                    existing=existing, status=status, decoded=decoded
                ):
                    if existing:
                        self.target.write_bytes(b"previous image")
                    environment = {
                        "PATH": str(self.root)
                        + os.pathsep
                        + os.environ["PATH"],
                        "DEMO_ENCODE_STATUS": str(status),
                        "DEMO_DECODED": decoded,
                    }
                    with patch.dict(os.environ, environment):
                        result = self.optimize()
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(self.target.exists(), existing)
                    if existing:
                        self.assertEqual(
                            self.target.read_bytes(), b"previous image"
                        )
                    self.assertFalse(list(self.root.glob("capture.jpg.*")))
