import contextlib
import io
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import build_cache
import hooks


class RoutingTests(unittest.TestCase):
    packages = {
        ".": "vpdf",
        "crates/ui": "vpdf-ui",
        "crates/vpdf-gui": "vpdf-gui",
    }

    def test_affected_owners(self):
        cases = [
            (["crates/ui/src/keys.rs"], {"vpdf-ui"}),
            (["crates/ui/src/help.txt"], {"vpdf-ui"}),
            (["src/input.rs", "crates/ui/src/help.txt"], {"vpdf", "vpdf-ui"}),
            (["tests/it/keys.rs"], {"vpdf"}),
        ]
        for paths, expected in cases:
            with self.subTest(paths=paths):
                owners, tasks, reasons = hooks.select_tests(
                    paths, self.packages
                )
                self.assertEqual(owners, expected)
                self.assertFalse(tasks or reasons)

    def test_help_image_owners(self):
        owners, tasks, reasons = hooks.select_tests(
            ["docs/assets/vpdf.png"], self.packages
        )
        self.assertEqual(owners, {"vpdf-gui"})
        self.assertEqual(tasks, {"test:help"})
        self.assertEqual(reasons, [])

    def test_conservative_fallback(self):
        for name in [
            "Cargo.lock",
            "crates/ui/Cargo.toml",
            ".config/mise.toml",
            "rust-toolchain.toml",
            "tests/support/page.rs",
            "benches/support/percentile.rs",
            "scripts/new_tool.py",
            "scripts/build-features.sh",
            "crates/missing/src/lib.rs",
        ]:
            with self.subTest(name=name):
                _, _, reasons = hooks.select_tests([name], self.packages)
                self.assertEqual(reasons, [name])

    def test_python_groups(self):
        cases = {
            "scripts/startup.py": "test:startup",
            "scripts/test_startup.py": "test:startup",
            "scripts/demo-optimize.sh": "test:demo",
            "scripts/demo_media.py": "test:demo",
            "scripts/test_demo_media.py": "test:demo",
            "scripts/demo_window.py": "test:demo",
            "scripts/demo_video.py": "test:demo",
            "scripts/test_demo_video.py": "test:demo",
            "scripts/test_packages.py": "test:packages",
            "scripts/test_build_cache.py": "test:build-cache",
            "benches/live/runtime.py": "test:bench-report",
        }
        for name, expected in cases.items():
            with self.subTest(name=name):
                owners, tasks, reasons = hooks.select_tests([name], {})
                self.assertEqual(tasks, {expected})
                self.assertFalse(owners or reasons)

    def test_prose_skips_tests(self):
        self.assertEqual(
            hooks.select_tests(["README.md", "docs/keys.md"], {}),
            (set(), set(), []),
        )

    def test_manual_checks_pdf(self):
        for name in [
            "docs/users/navigation.md",
            "docs/users/readme.md",
            "scripts/help/manual.typ",
            "scripts/help_assets.py",
            "scripts/help_build.rs",
        ]:
            with self.subTest(name=name):
                self.assertEqual(
                    hooks.select_tests([name], {}),
                    (set(), {"test:help"}, []),
                )

    def test_downstream_filter(self):
        with (
            patch.object(hooks, "package_folders", return_value=self.packages),
            patch.object(hooks, "execute") as run,
        ):
            hooks.affected_tests(
                Path("/snapshot"),
                ["crates/ui/src/help.txt", "src/input.rs"],
                {},
            )
        command = run.call_args.args[0]
        self.assertEqual(
            run.call_args_list[0].args[0], ["mise", "run", "help:build"]
        )
        self.assertEqual(
            command[command.index("-E") + 1], "rdeps(=vpdf) | rdeps(=vpdf-ui)"
        )
        self.assertIn("--workspace", command)
        self.assertIn("--no-tests=fail", command)

    def test_metadata_fallback(self):
        with (
            patch.object(hooks, "package_folders", side_effect=ValueError),
            patch.object(hooks, "execute") as run,
        ):
            hooks.affected_tests(Path("/snapshot"), ["src/input.rs"], {})
        self.assertEqual(run.call_args.args[0], ["mise", "run", "test"])

    def test_fallback_keeps_packaging(self):
        with patch.object(hooks, "execute") as run:
            hooks.affected_tests(
                Path("/snapshot"),
                ["scripts/build-app.sh", "scripts/startup.py"],
                {},
            )
        self.assertEqual(
            [call.args[0] for call in run.call_args_list],
            [["mise", "run", "test"], ["mise", "run", "test:app"]],
        )

    def test_full_skips_worker(self):
        with patch.object(hooks, "execute") as run:
            hooks.affected_tests(
                Path("/snapshot"),
                ["scripts/build_worker.py", "scripts/test_build_worker.py"],
                {},
            )
        self.assertEqual(
            [call.args[0] for call in run.call_args_list],
            [["mise", "run", "test"]],
        )

    def test_metadata_keeps_packaging(self):
        with (
            patch.object(hooks, "package_folders", side_effect=ValueError),
            patch.object(hooks, "execute") as run,
        ):
            hooks.affected_tests(
                Path("/snapshot"), ["src/input.rs", "scripts/build-app.sh"], {}
            )
        self.assertEqual(
            [call.args[0] for call in run.call_args_list],
            [["mise", "run", "test"], ["mise", "run", "test:app"]],
        )

    def test_python_skips_metadata(self):
        with (
            patch.object(hooks, "package_folders") as metadata,
            patch.object(hooks, "execute") as run,
        ):
            hooks.affected_tests(
                Path("/snapshot"), ["benches/live/runtime.py"], {}
            )
        metadata.assert_not_called()
        self.assertEqual(
            run.call_args.args[0], ["mise", "run", "test:bench-report"]
        )

    def test_failures_propagate(self):
        error = subprocess.CalledProcessError(100, "nextest")
        with (
            patch.object(hooks, "execute", side_effect=error),
            self.assertRaises(subprocess.CalledProcessError),
        ):
            hooks.affected_tests(Path("/snapshot"), ["scripts/startup.py"], {})

    def test_lint_checks_only(self):
        with patch.object(hooks, "execute") as run:
            hooks.full_checks(Path("/snapshot"), {})
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(commands[-1], ["mise", "run", "test"])
        self.assertIn(["cargo", "fmt", "--all", "--check"], commands)
        self.assertFalse(any("--fix" in command for command in commands))


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("GIT_")
        }
        env.update(
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_CONFIG_NOSYSTEM="1",
            XDG_CACHE_HOME=str(self.root / "cache"),
        )
        self.environment = patch.dict(os.environ, env, clear=True)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.git("init", "-q")
        self.git("config", "user.name", "Hook Tests")
        self.git("config", "user.email", "hooks@example.invalid")
        self.git("config", "core.hooksPath", str(self.root / ".git/hooks"))
        self.write("README.md", "# Reader\n")
        self.write("rustfmt.toml", 'edition = "2024"\n')
        self.write(
            "scripts/width.sh",
            Path(hooks.__file__).with_name("width.sh").read_text(),
        )
        self.stage("README.md", "rustfmt.toml", "scripts/width.sh")
        self.commit()
        self.output = contextlib.redirect_stdout(io.StringIO())
        self.output.__enter__()
        self.addCleanup(self.output.__exit__, None, None, None)

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root)

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def stage(self, *names):
        self.git("add", "--", *names)

    def commit(self):
        self.git("commit", "-qm", "test: fixture")
        return self.git("rev-parse", "HEAD").decode().strip()

    @contextlib.contextmanager
    def snapshot(self):
        tree = self.git("write-tree").decode().strip()
        with hooks.snapshot(self.root, tree) as (source, env):
            self.addCleanup(shutil.rmtree, source.parent, True)
            yield source, env

    def test_staged_snapshot(self):
        self.write("README.md", "# Staged\n")
        self.stage("README.md")
        self.write("README.md", "# Unstaged\n")
        self.write("untracked.txt", "Not staged\n")
        with self.snapshot() as (source, _):
            self.assertEqual(source, source.resolve())
            self.assertEqual((source / "README.md").read_text(), "# Staged\n")
            self.assertFalse((source / "untracked.txt").exists())

    def test_snapshot_reuses_files(self):
        with self.snapshot() as (source, _):
            timestamp = (source / "README.md").stat().st_mtime_ns
            (source / "leftover.txt").write_text("leftover")
        with self.snapshot() as (source, _):
            self.assertEqual(
                (source / "README.md").stat().st_mtime_ns, timestamp
            )
            self.assertFalse((source / "leftover.txt").exists())

    def test_deleted_snapshot_files(self):
        with self.snapshot():
            pass
        self.git("rm", "README.md")
        with self.snapshot() as (source, _):
            self.assertFalse((source / "README.md").exists())

    def test_symlink_transition(self):
        with self.snapshot():
            pass
        (self.root / "README.md").unlink()
        (self.root / "README.md").symlink_to("rustfmt.toml")
        self.stage("README.md")
        with self.snapshot() as (source, _):
            self.assertTrue((source / "README.md").is_symlink())
        (self.root / "README.md").unlink()
        self.write("README.md", "# Reader\n")
        self.stage("README.md")
        with self.snapshot() as (source, _):
            self.assertFalse((source / "README.md").is_symlink())

    def test_cache_disabled(self):
        with (
            patch.dict(os.environ, VPDF_CACHE=str(self.root / "user-cache")),
            self.snapshot() as (_, env),
        ):
            self.assertEqual(env["VPDF_CACHE"], "off")
            self.assertFalse((self.root / "user-cache").exists())

    def test_shared_build_identity(self):
        with self.snapshot() as (_, env):
            self.assertEqual(
                env["VPDF_BUILD_CACHE_ID"], build_cache.cache_key(self.root)
            )

    def test_git_environment_removed(self):
        index = self.root / "custom-index"
        shutil.copyfile(self.root / ".git/index", index)
        original = index.read_bytes()
        with (
            patch.dict(os.environ, GIT_INDEX_FILE=str(index)),
            self.snapshot() as (_, env),
        ):
            self.assertNotIn("GIT_INDEX_FILE", env)
        self.assertEqual(index.read_bytes(), original)

    def test_external_symlink_rejected(self):
        (self.root / "README.md").unlink()
        (self.root / "README.md").symlink_to("../outside")
        self.stage("README.md")
        with self.assertRaisesRegex(RuntimeError, "symlink escapes"):
            with self.snapshot():
                pass

    def test_renames_and_deletions(self):
        self.write("old name.rs", "fn old() {}\n")
        self.stage("old name.rs")
        self.commit()
        self.git("mv", "old name.rs", "new name.rs")
        self.assertEqual(
            set(hooks.changed_paths(self.root)), {"old name.rs", "new name.rs"}
        )

    def test_formatter_requires_restaging(self):
        self.write("example.py", "value=1\n")
        self.stage("example.py")
        with self.snapshot() as (source, env):
            self.assertFalse(
                hooks.format_staged(self.root, source, ["example.py"], env)
            )
        self.assertEqual((self.root / "example.py").read_text(), "value = 1\n")
        self.assertEqual(self.git("show", ":example.py"), b"value=1\n")

    def test_partial_file_unchanged(self):
        self.write("example.py", "value=1\n")
        self.stage("example.py")
        self.write("example.py", "value=2\n")
        with self.snapshot() as (source, env):
            self.assertFalse(
                hooks.format_staged(self.root, source, ["example.py"], env)
            )
        self.assertEqual((self.root / "example.py").read_text(), "value=2\n")
        self.assertEqual(self.git("show", ":example.py"), b"value=1\n")

    def test_partial_mode_preserved(self):
        path = self.write("example.py", "value=1\n")
        path.chmod(0o644)
        self.stage("example.py")
        path.chmod(0o755)
        with self.snapshot() as (source, env):
            self.assertFalse(
                hooks.format_staged(self.root, source, ["example.py"], env)
            )
        self.assertEqual(path.read_text(), "value=1\n")
        self.assertTrue(path.stat().st_mode & 0o111)
        self.assertEqual(self.git("show", ":example.py"), b"value=1\n")

    def test_partial_parent_preserved(self):
        self.write("tools/example.py", "value=1\n")
        self.stage("tools/example.py")
        external = tempfile.TemporaryDirectory()
        self.addCleanup(external.cleanup)
        target = Path(external.name) / "example.py"
        target.write_text("value=1\n")
        shutil.rmtree(self.root / "tools")
        (self.root / "tools").symlink_to(
            target.parent, target_is_directory=True
        )
        with self.snapshot() as (source, env):
            self.assertFalse(
                hooks.format_staged(
                    self.root, source, ["tools/example.py"], env
                )
            )
        self.assertEqual(target.read_text(), "value=1\n")
        self.assertTrue((self.root / "tools").is_symlink())

    def test_rustfmt_preserves_children(self):
        self.write("src/main.rs", "mod child;\nfn main() {}\n")
        child = "pub fn child( ){}\n"
        self.write("src/child.rs", child)
        self.stage("src/main.rs", "src/child.rs")
        with self.snapshot() as (source, env):
            hooks.format_staged(self.root, source, ["src/main.rs"], env)
            self.assertEqual((source / "src/child.rs").read_text(), child)
        self.assertEqual((self.root / "src/child.rs").read_text(), child)

    def test_empty_stage_skips(self):
        with patch.object(hooks, "snapshot") as snapshot:
            self.assertEqual(hooks.pre_commit(self.root), 0)
        snapshot.assert_not_called()

    def test_prose_avoids_worker(self):
        self.write("README.md", "# Staged\n")
        self.stage("README.md")
        with patch.object(hooks, "snapshot") as snapshot:
            self.assertEqual(hooks.pre_commit(self.root), 0)
        snapshot.assert_not_called()

    def test_formatting_avoids_worker(self):
        self.write("example.py", "value=1\n")
        self.stage("example.py")
        with patch.object(hooks, "snapshot") as snapshot:
            self.assertEqual(hooks.pre_commit(self.root), 1)
        snapshot.assert_not_called()
        self.assertEqual((self.root / "example.py").read_text(), "value = 1\n")

    def test_staged_test_content(self):
        self.write("src/main.rs", "fn main() {}\n")
        self.stage("src/main.rs")
        self.write("src/main.rs", "invalid unstaged code\n")
        seen = []
        with patch.object(
            hooks,
            "affected_tests",
            side_effect=lambda source, paths, env: seen.append(
                (source / "src/main.rs").read_text()
            ),
        ):
            self.assertEqual(hooks.pre_commit(self.root), 0)
        self.assertEqual(seen, ["fn main() {}\n"])
        self.assertEqual(
            (self.root / "src/main.rs").read_text(), "invalid unstaged code\n"
        )

    def test_push_checks_tips(self):
        first = self.git("rev-parse", "HEAD").decode().strip()
        self.write("README.md", "# Second\n")
        self.stage("README.md")
        second = self.commit()
        self.write("README.md", "# Unstaged\n")
        null = "0" * 40
        refs = f"refs/heads/first {first} refs/heads/first {null}\nrefs/heads/second {second} refs/heads/second {first}\nrefs/tags/same {second} refs/tags/same {null}\n"
        seen = []
        with patch.object(
            hooks,
            "full_checks",
            side_effect=lambda source, env: seen.append(
                (source / "README.md").read_text()
            ),
        ):
            self.assertEqual(hooks.pre_push(self.root, io.StringIO(refs)), 0)
        self.assertCountEqual(seen, ["# Reader\n", "# Second\n"])
        self.assertEqual((self.root / "README.md").read_text(), "# Unstaged\n")

    def test_annotated_tag_validation(self):
        commit = self.git("rev-parse", "HEAD").decode().strip()
        self.git("tag", "-am", "fixture", "fixture-tag")
        tag = self.git("rev-parse", "refs/tags/fixture-tag").decode().strip()
        null = "0" * 40
        refs = f"refs/heads/main {commit} refs/heads/main {null}\nrefs/tags/fixture-tag {tag} refs/tags/fixture-tag {null}\n"
        with patch.object(hooks, "full_checks") as checks:
            hooks.pre_push(self.root, io.StringIO(refs))
        self.assertEqual(checks.call_count, 1)

    def test_push_deletion_skips(self):
        null = "0" * 40
        with patch.object(hooks, "full_checks") as checks:
            hooks.pre_push(
                self.root,
                io.StringIO(f"(delete) {null} refs/heads/old {'1' * 40}\n"),
            )
        checks.assert_not_called()

    def test_invalid_push_input(self):
        with self.assertRaisesRegex(RuntimeError, "Invalid push reference"):
            hooks.pre_push(self.root, io.StringIO("invalid\n"))

    def test_unknown_hooks_preserved(self):
        path = self.root / ".git/hooks/pre-push"
        path.write_text("#!/bin/sh\necho custom\n")
        with self.assertRaisesRegex(RuntimeError, "Existing hook"):
            hooks.install_hooks(self.root)
        self.assertFalse((self.root / ".git/hooks/pre-commit").exists())
        self.assertEqual(path.read_text(), "#!/bin/sh\necho custom\n")

    def test_custom_hooks_directory(self):
        folder = self.root / "custom hooks"
        self.git("config", "core.hooksPath", str(folder))
        hooks.install_hooks(self.root)
        self.assertTrue((folder / "pre-commit").stat().st_mode & 0o111)
        self.assertTrue((folder / "pre-push").stat().st_mode & 0o111)

    def test_native_partial_preserved(self):
        self.write("scripts/hooks.py", Path(hooks.__file__).read_text())
        self.write(
            "scripts/build_worker.py",
            Path(hooks.__file__).with_name("build_worker.py").read_text(),
        )
        self.write(
            "scripts/validation.py",
            Path(hooks.__file__).with_name("validation.py").read_text(),
        )
        self.write(
            "scripts/build_cache.py",
            Path(hooks.__file__).with_name("build_cache.py").read_text(),
        )
        shim = self.write(
            "scripts/tools/cargo",
            Path(hooks.__file__)
            .with_name("tools")
            .joinpath("cargo")
            .read_text(),
        )
        shim.chmod(0o755)
        self.write(
            ".config/mise.toml",
            (
                Path(hooks.__file__).resolve().parent.parent
                / ".config/mise.toml"
            ).read_text(),
        )
        self.stage(
            "scripts/hooks.py",
            "scripts/build_cache.py",
            "scripts/build_worker.py",
            "scripts/validation.py",
            "scripts/tools/cargo",
            ".config/mise.toml",
        )
        self.commit()
        hooks.install_hooks(self.root)
        self.write("README.md", "# Reader\n\nStaged.\n")
        self.stage("README.md")
        self.write("README.md", "# Reader\n\nUnstaged.\n")
        subprocess.run(
            ["git", "hook", "run", "pre-commit"], cwd=self.root, check=True
        )
        self.assertEqual(
            (self.root / "README.md").read_text(), "# Reader\n\nUnstaged.\n"
        )
        self.assertEqual(
            self.git("show", ":README.md"), b"# Reader\n\nStaged.\n"
        )


if __name__ == "__main__":
    unittest.main()
