import io
import os
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.build import receipts
from tools.check import ci


class CheckTests(unittest.TestCase):
    stages = ["corpus", "lint", "test", "test:events", "release", "archive"]

    def run_checks(self, failing=None):
        seen = []

        def stage(name):
            seen.append(name)
            if name == failing:
                raise subprocess.CalledProcessError(1, name)

        with (
            patch.object(ci, "working_files", return_value={}),
            patch.object(
                ci,
                "execute",
                side_effect=lambda command, *args: stage(command[-1]),
            ),
            patch.object(
                ci, "check_lint", side_effect=lambda *args: stage("lint")
            ),
            patch.object(
                ci, "archive", side_effect=lambda *args: stage("archive")
            ),
        ):
            if failing:
                with self.assertRaises(subprocess.CalledProcessError):
                    ci.checks(Path("/snapshot"), {})
            else:
                ci.checks(Path("/snapshot"), {})
        return seen

    def test_complete_contract(self):
        self.assertEqual(self.run_checks(), self.stages)

    def test_each_failure_blocks(self):
        for index, stage in enumerate(self.stages):
            with self.subTest(stage=stage):
                self.assertEqual(
                    self.run_checks(stage), self.stages[: index + 1]
                )

    def test_source_mutation_rejected(self):
        with (
            patch.object(
                ci,
                "working_files",
                side_effect=[{}, {"changed": (b"100644", b"new")}],
            ),
            patch.object(ci, "execute"),
            patch.object(ci, "check_lint"),
            patch.object(ci, "archive"),
            self.assertRaisesRegex(RuntimeError, "changed source"),
        ):
            ci.checks(Path("/snapshot"), {})


class CacheTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = {
            "XDG_CACHE_HOME": str(self.root / "cache"),
            "VPDF_BUILD_CACHE_ID": "0123456789abcdef",
        }
        self.seen = []
        self.failure = None
        for target, options in [
            ("working_files", {"return_value": {}}),
            (
                "execute",
                {"side_effect": lambda command, *args: self.stage(command[-1])},
            ),
            ("check_lint", {"side_effect": lambda *args: self.stage("lint")}),
            ("archive", {"side_effect": lambda *args: self.stage("archive")}),
        ]:
            mock = patch.object(ci, target, **options)
            mock.start()
            self.addCleanup(mock.stop)
        identity = patch.object(
            receipts, "fingerprint", return_value="ci-inputs"
        )
        identity.start()
        self.addCleanup(identity.stop)

    def stage(self, name):
        self.seen.append(name)
        if name == self.failure:
            raise subprocess.CalledProcessError(1, name)
        if name == "archive":
            packed, digest = ci.archive_paths(self.root, self.env)
            for path in [
                packed.parent.parent / "release/vpdf",
                packed.parent / "vpdf",
                packed,
                digest,
            ]:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("verified")

    def check(self):
        self.seen.clear()
        ci.checks(self.root, self.env)
        return self.seen.copy()

    def test_repeat_reuses_stages(self):
        self.assertEqual(self.check(), CheckTests.stages)
        self.assertEqual(self.check(), ["corpus", "help:build"])

    def test_partial_success_reused(self):
        self.failure = "test:events"
        with self.assertRaises(subprocess.CalledProcessError):
            self.check()
        self.failure = None
        self.assertEqual(
            self.check(),
            ["corpus", "help:build", "test:events", "release", "archive"],
        )

    def test_damage_only_repacks(self):
        self.check()
        packed, digest = ci.archive_paths(self.root, self.env)
        for path in [packed.parent / "vpdf", packed, digest]:
            path.unlink()
            self.assertEqual(
                self.check(), ["corpus", "help:build", "release", "archive"]
            )

    def test_force_reruns_stages(self):
        self.check()
        self.env["VPDF_VALIDATE_FORCE"] = "1"
        self.assertEqual(self.check(), CheckTests.stages)

    def test_existing_receipt_reused(self):
        receipts.run_check(self.root, self.env, "test", lambda: None)
        self.assertEqual(
            self.check(),
            ["corpus", "lint", "test:events", "release", "archive"],
        )

    def test_ci_disables_reuse(self):
        self.check()
        with patch.object(receipts, "fingerprint", return_value=None):
            self.assertEqual(self.check(), CheckTests.stages)


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = dict(os.environ)
        self.env["CARGO_TARGET_DIR"] = str(self.root / "custom-target")
        self.binary = self.root / "custom-target/dist/vpdf"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_text('#!/bin/sh\nprintf "archive-smoke\\n"\n')
        self.binary.chmod(0o755)
        self.archive, self.digest = ci.archive_paths(self.root, self.env)

    def test_roundtrip(self):
        ci.archive(self.root, self.env)
        self.assertTrue(self.archive.is_file())
        self.assertEqual(
            self.digest.read_text(),
            f"{ci.checksum(self.archive)}  {self.archive.name}\n",
        )
        with tarfile.open(self.archive) as packed:
            self.assertEqual(packed.getnames(), ["vpdf"])
            self.assertEqual(
                packed.extractfile("vpdf").read(), self.binary.read_bytes()
            )
        ci.verify_archive(self.archive, self.digest, self.root, self.env)

    def test_corrupt_archive_rejected(self):
        ci.archive(self.root, self.env)
        with self.archive.open("ab") as stream:
            stream.write(b"corrupt")
        with self.assertRaisesRegex(RuntimeError, "checksum"):
            ci.verify_archive(self.archive, self.digest, self.root, self.env)

    def test_invalid_member_rejected(self):
        for name, kind, mode in [
            ("../vpdf", tarfile.REGTYPE, 0o755),
            ("vpdf", tarfile.SYMTYPE, 0o755),
            ("vpdf", tarfile.REGTYPE, 0o644),
        ]:
            with self.subTest(name=name, kind=kind, mode=mode):
                member = tarfile.TarInfo(name)
                member.type = kind
                member.mode = mode
                with tarfile.open(self.archive, "w:gz") as packed:
                    packed.addfile(member, io.BytesIO())
                self.digest.write_text(
                    f"{ci.checksum(self.archive)}  {self.archive.name}\n"
                )
                with self.assertRaisesRegex(RuntimeError, "executable"):
                    ci.verify_archive(
                        self.archive, self.digest, self.root, self.env
                    )

    def test_failed_smoke_preserves(self):
        ci.archive(self.root, self.env)
        original = self.archive.read_bytes(), self.digest.read_bytes()
        self.binary.write_text("#!/bin/sh\nexit 42\n")
        with self.assertRaises(subprocess.CalledProcessError):
            ci.archive(self.root, self.env)
        self.assertEqual(
            (self.archive.read_bytes(), self.digest.read_bytes()), original
        )
        self.assertEqual(
            {path.name for path in self.binary.parent.iterdir()},
            {"vpdf", self.archive.name, self.digest.name},
        )

    def test_platform_names(self):
        for system, machine, expected in [
            ("Darwin", "arm64", "macos-aarch64"),
            ("Linux", "x86_64", "linux-x86_64"),
        ]:
            with (
                self.subTest(system=system),
                patch.object(ci.platform, "system", return_value=system),
                patch.object(ci.platform, "machine", return_value=machine),
            ):
                archive, _ = ci.archive_paths(self.root, self.env)
                self.assertEqual(archive.name, f"vpdf-{expected}.tar.gz")


if __name__ == "__main__":
    unittest.main()
