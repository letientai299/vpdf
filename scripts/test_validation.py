import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

import validation


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = {
            "XDG_CACHE_HOME": str(self.root / "cache"),
            "VPDF_BUILD_CACHE_ID": "0123456789abcdef",
        }
        self.identity = patch.object(
            validation, "fingerprint", return_value="first"
        )
        self.identity.start()
        self.addCleanup(self.identity.stop)

    def check(self, scope="test", action=None, **options):
        action = action or Mock()
        validation.run_check(self.root, self.env, scope, action, **options)
        return action

    def test_success_reused(self):
        action = Mock()
        self.check(action=action)
        self.check(action=action)
        self.assertEqual(action.call_count, 1)

    def test_full_covers_subset(self):
        self.check()
        action = self.check("affected:model", covered_by="test")
        action.assert_not_called()
        self.check("lint").assert_called_once()

    def test_subset_cannot_cover_full(self):
        self.check("affected:model")
        self.check().assert_called_once()

    def test_failure_not_saved(self):
        for error in [RuntimeError("failed"), KeyboardInterrupt()]:
            with self.subTest(error=error), self.assertRaises(type(error)):
                self.check(action=Mock(side_effect=error))
        self.check().assert_called_once()

    def test_changed_inputs_not_saved(self):
        with patch.object(validation, "fingerprint", side_effect=["a", "b"]):
            self.check()
        with patch.object(validation, "fingerprint", return_value="a"):
            self.check().assert_called_once()

    def test_forced_failure_invalidates(self):
        self.check()
        self.env["VPDF_VALIDATE_FORCE"] = "1"
        with self.assertRaises(RuntimeError):
            self.check(action=Mock(side_effect=RuntimeError("failed")))
        self.env.pop("VPDF_VALIDATE_FORCE")
        self.check().assert_called_once()

    def test_forced_full_invalidates_subsets(self):
        self.check("affected:model")
        self.check("test:startup")
        self.check("lint")
        with patch.object(validation, "fingerprint", return_value="other"):
            self.check()
        self.env["VPDF_VALIDATE_FORCE"] = "1"
        with self.assertRaises(RuntimeError):
            self.check(action=Mock(side_effect=RuntimeError("failed")))
        self.env.pop("VPDF_VALIDATE_FORCE")
        for scope in ["test", "affected:model", "test:startup"]:
            self.assertFalse(
                validation.completed(self.root, self.env, "first", scope)
            )
        self.assertTrue(
            validation.completed(self.root, self.env, "first", "lint")
        )
        self.assertTrue(
            validation.completed(self.root, self.env, "other", "test")
        )

    def test_unavailable_identity_runs(self):
        with patch.object(validation, "fingerprint", return_value=None):
            action = Mock()
            self.check(action=action)
            self.check(action=action)
            self.assertEqual(action.call_count, 2)

    def test_corrupt_receipt_runs(self):
        path = validation.receipt_path(self.root, self.env, "first", "test")
        path.parent.mkdir(parents=True)
        path.write_text("invalid json")
        self.check().assert_called_once()

    def test_snapshot_switch_preserves_receipt(self):
        first = self.check()
        with patch.object(validation, "fingerprint", return_value="second"):
            self.check().assert_called_once()
        self.check(action=first)
        first.assert_called_once()


class FingerprintTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.env = {
            "PATH": os.environ["PATH"],
            "HOME": str(self.root),
            "VPDF_BUILD_WORKER": str(self.root),
            "VPDF_CACHE": "off",
            "VPDF_CORPUS": str(self.root / "corpus"),
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
        }
        self.git("init", "-q")
        self.git("config", "user.name", "Validation Tests")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "core.hooksPath", os.devnull)
        (self.root / "source").write_text("first")
        (self.root / ".gitignore").write_text("/corpus/\n")
        self.git("add", "source", ".gitignore")
        self.git("commit", "-qm", "test: fixture")
        tools = patch.object(validation, "TOOLS", ("python3",))
        tools.start()
        self.addCleanup(tools.stop)

    def git(self, *args):
        return subprocess.check_output(
            ["git", *args], cwd=self.root, env=self.env
        )

    def identity(self, **changes):
        return validation.fingerprint(self.root, self.env | changes)

    def test_source_mode_environment(self):
        original = self.identity()
        self.assertIsNotNone(original)
        (self.root / "source").write_text("second")
        self.assertNotEqual(self.identity(), original)
        (self.root / "source").write_text("first")
        self.assertEqual(self.identity(), original)
        (self.root / "source").chmod(0o755)
        self.assertNotEqual(self.identity(), original)
        self.assertNotEqual(self.identity(RUST_BACKTRACE="1"), self.identity())

    def test_provenance_invalidates(self):
        original = self.identity()
        self.git("commit", "--allow-empty", "-qm", "test: provenance")
        self.assertNotEqual(self.identity(), original)
        original = self.identity()
        self.git("tag", "v0.1.0")
        self.assertNotEqual(self.identity(), original)

    def test_corpus_contents_invalidate(self):
        corpus = self.root / "corpus"
        corpus.mkdir()
        original = self.identity()
        path = corpus / "input"
        path.write_bytes(b"first")
        self.assertNotEqual(self.identity(), original)
        original = self.identity()
        path.write_bytes(b"other")
        self.assertNotEqual(self.identity(), original)

    def test_relative_corpus_contents(self):
        corpus = self.root / "corpus"
        corpus.mkdir()
        original = self.identity(VPDF_CORPUS="corpus")
        (corpus / "input").write_bytes(b"changed")
        self.assertNotEqual(self.identity(VPDF_CORPUS="corpus"), original)

    def test_force_and_custom_inputs_disable(self):
        for changes in [
            {"CI": "true"},
            {"NEXTEST_RETRIES": "1"},
            {"NEXTEST_CONFIG_FILE": "/custom.toml"},
            {"VPDF_SEARCH_PMPP": "/external.pdf"},
            {"VPDF_CACHE": "/external/cache"},
            {"VPDF_BUILD_MODE": "isolated"},
            {"VPDF_BUILD_WORKER": ""},
            {"RUSTFLAGS": "-g"},
            {"RUSTC_WRAPPER": "custom"},
            {"CC": "custom"},
            {"LIBCLANG_PATH": "/custom"},
        ]:
            with self.subTest(changes=changes):
                self.assertIsNone(self.identity(**changes))

    def test_symlinks_disable(self):
        (self.root / "link").symlink_to("source")
        self.assertIsNone(self.identity())

    def test_execution_metadata_ignored(self):
        self.assertEqual(
            self.identity(),
            self.identity(
                VPDF_BUILD_WORKER_FD="19",
                MISE_TASK_NAME="commit",
                MISE_TASK_TIMINGS="/different/request",
                MISE_TASK_COLOR="0",
                MISE_TASK_PGID_MANAGED="1234",
                CODEX_TOOL_CALL_ID="different-call",
            ),
        )

    def test_cargo_config_disables(self):
        config = self.root / ".cargo/config.toml"
        config.parent.mkdir()
        config.write_text('[build]\nrustflags = ["-g"]\n')
        self.assertIsNone(self.identity())

    def test_environment_home_configs(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            for name in [".cargo/config.toml", ".config/nextest/config.toml"]:
                with self.subTest(name=name):
                    config = home / name
                    config.parent.mkdir(parents=True, exist_ok=True)
                    config.write_text("# custom configuration\n")
                    self.assertIsNone(self.identity(HOME=str(home)))
                    config.unlink()

    def test_host_home_ignored(self):
        original = self.identity()
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            (home / ".cargo").mkdir()
            (home / ".cargo/config.toml").write_text("# host config\n")
            with patch.object(Path, "home", return_value=home):
                self.assertEqual(self.identity(), original)

    def test_git_config_invalidates(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "gitconfig"
            included = Path(directory) / "included"
            config.write_text(f'[include]\npath = "{included}"\n')
            included.write_text("[diff]\nalgorithm = myers\n")
            env = {"GIT_CONFIG_GLOBAL": str(config)}
            original = self.identity(**env)
            self.assertIsNotNone(original)
            included.write_text("[diff]\nalgorithm = histogram\n")
            self.assertNotEqual(self.identity(**env), original)

    def test_cargo_shim_ignored(self):
        tools = self.root / "scripts/tools"
        tools.mkdir(parents=True)
        shim = tools / "python3"
        shim.write_text("#!/bin/sh\necho unexpected\n")
        shim.chmod(0o755)
        self.assertEqual(
            self.identity(),
            self.identity(PATH=str(tools) + os.pathsep + self.env["PATH"]),
        )

    def test_parent_index_ignored(self):
        original = self.identity()
        with patch.dict(os.environ, GIT_INDEX_FILE=str(self.root / "missing")):
            self.assertEqual(self.identity(), original)

    def test_nextest_config_disables(self):
        config = self.root / ".config/nextest.toml"
        config.parent.mkdir()
        config.write_text("[profile.default]\nretries = 1\n")
        self.assertIsNone(self.identity())

    def test_tool_failure_disables(self):
        with patch.object(validation, "TOOLS", ("missing-vpdf-tool",)):
            self.assertIsNone(self.identity())
