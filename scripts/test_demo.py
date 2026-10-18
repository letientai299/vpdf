import json
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
            patch.object(demo, "run", return_value="vpdf aaaaaaa (2026-10-18)"),
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
        other = output / "search.webp"
        other.write_bytes(b"unchanged image")
        before = other.stat().st_mtime_ns
        previous = {"image": other.name, "capture": "old", "sha256": demo.digest(other)}
        manifest = {
            "captures": {"old": {"commit": "previous"}},
            "scenes": {"search": previous},
        }
        image = self.root / "figure.webp"
        image.write_bytes(b"new image")
        identity = {"commit": "current"}
        scene = {"image": image.name, "sha256": demo.digest(image)}
        updated = demo.publish_scene(output, manifest, identity, "figure", scene, image)
        self.assertEqual(updated["scenes"]["search"], previous)
        self.assertEqual(other.read_bytes(), b"unchanged image")
        self.assertEqual(other.stat().st_mtime_ns, before)
        self.assertEqual((output / image.name).read_bytes(), image.read_bytes())
        self.assertEqual(json.loads((output / "capture.json").read_text()), updated)
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
                target = output / "figure.webp"
                if existing:
                    target.write_bytes(b"previous image")
                report = output / "capture.json"
                report.write_text('{"previous": true}\n')
                manifest = {"captures": {}, "scenes": {}}
                image = self.root / "candidate.webp"
                image.write_bytes(b"candidate image")
                scene = {"image": target.name}
                with (
                    patch.object(Path, "replace", new=fail_report),
                    self.assertRaises(OSError),
                ):
                    demo.publish_scene(output, manifest, {}, "figure", scene, image)
                self.assertEqual(report.read_text(), '{"previous": true}\n')
                self.assertEqual(target.exists(), existing)
                if existing:
                    self.assertEqual(target.read_bytes(), b"previous image")
                self.assertEqual(manifest, {"captures": {}, "scenes": {}})
                self.assertFalse(list(output.glob(".capture-*")))
