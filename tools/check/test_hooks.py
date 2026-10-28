import contextlib
import io
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.build import cache as build_cache
from tools.check import hooks


class MergeTests(unittest.TestCase):
    def test_merges_serialize(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "log"
            wt = root / "bin/wt"
            wt.parent.mkdir()
            wt.write_text(
                f"#!/bin/sh\necho start >> '{log}'\nsleep 0.3\n"
                f"echo \"end $*\" >> '{log}'\n"
            )
            wt.chmod(0o755)
            env = os.environ | {
                "PATH": str(wt.parent) + os.pathsep + os.environ["PATH"],
                "XDG_CACHE_HOME": str(root / "cache"),
                "VPDF_BUILD_CACHE_ID": "0123456789abcdef",
            }
            command = [sys.executable, "-B", "-m", "tools.check.merge", "-y"]
            project = Path(hooks.__file__).resolve().parents[2]
            first = subprocess.Popen(command, cwd=project, env=env)
            try:
                deadline = time.monotonic() + 10
                while not log.exists():
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(0.01)
                second = subprocess.run(
                    command,
                    cwd=project,
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                self.assertEqual(first.wait(timeout=10), 0)
            finally:
                if first.poll() is None:
                    first.kill()
                    first.wait()
            self.assertEqual(second.returncode, 0)
            self.assertIn("Waiting for another merge", second.stdout)
            self.assertEqual(
                log.read_text().splitlines(),
                ["start", "end merge -y", "start", "end merge -y"],
            )


class WidthTests(unittest.TestCase):
    def check(self, text, locale):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source.rs"
            path.write_text(text, encoding="utf-8")
            result = subprocess.run(
                [
                    "sh",
                    str(Path(hooks.__file__).with_name("width.sh")),
                    str(path),
                ],
                env=dict(os.environ)
                | {
                    "LC_ALL": locale,
                    "PYTHONUTF8": "0",
                    "PYTHONCOERCECLOCALE": "0",
                },
                capture_output=True,
                text=True,
            )
            return result

    def test_unicode_width_locales(self):
        for locale in ["C", "C.UTF-8"]:
            with self.subTest(locale=locale):
                result = self.check("界" * 80 + "\n", locale)
                self.assertEqual(
                    result.returncode, 0, result.stdout + result.stderr
                )
                self.assertEqual(result.stdout, "")

    def test_width_boundary(self):
        result = self.check("x" * 80 + "\n" + "界" * 81 + "\n", "C")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.assertIn("source.rs:2: exceeds 80 columns", result.stdout)


class RoutingTests(unittest.TestCase):
    packages = {
        ".": "vpdf",
        "crates/ui": "vpdf-ui",
        "crates/vpdf-gui": "vpdf-gui",
        "crates/vpdf-dev": "vpdf-dev",
    }

    def test_affected_owners(self):
        cases = [
            (["crates/ui/src/keys.rs"], {"vpdf-ui"}),
            (["crates/ui/src/help.txt"], {"vpdf-ui"}),
            (["src/input.rs", "crates/ui/src/help.txt"], {"vpdf", "vpdf-ui"}),
            (["tests/it/keys.rs"], {"vpdf"}),
            (["tools/build/cargo.rs"], {"vpdf"}),
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
            "crates/vpdf-dev/support/percentile.rs",
            "tools/new_tool.py",
            "tools/build/features.sh",
            "tools/build/receipts.py",
            "tools/shims/cargo",
            "crates/missing/src/lib.rs",
        ]:
            with self.subTest(name=name):
                _, _, reasons = hooks.select_tests([name], self.packages)
                self.assertEqual(reasons, [name])

    def test_python_groups(self):
        cases = {
            "tools/bench/startup.py": "test:startup",
            "tools/bench/test_startup.py": "test:startup",
            "tools/demo/optimize.sh": "test:demo",
            "tools/demo/capture.py": "test:demo",
            "tools/demo/test_capture.py": "test:demo",
            "tools/demo/media.py": "test:demo",
            "tools/demo/test_media.py": "test:demo",
            "tools/demo/window.py": "test:demo",
            "tools/demo/video.py": "test:demo",
            "tools/demo/test_video.py": "test:demo",
            "tools/check/test_packages.py": "test:packages",
            "tools/build/test_cache.py": "test:build-cache",
            "tools/build/test_features.py": "test:build-features",
            "tools/check/ci.py": "test:ci",
            "tools/check/test_ci.py": "test:ci",
            "tools/bench/live/runtime.py": "test:bench-report",
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
            "tools/help/help/manual.typ",
            "tools/help/assets.py",
            "tools/help/licenses.py",
            "tools/help/test_assets.py",
            "tools/build/help.rs",
        ]:
            with self.subTest(name=name):
                self.assertEqual(
                    hooks.select_tests([name], {}),
                    (set(), {"test:help"}, []),
                )

    def test_dev_crate_skips_nextest(self):
        for paths, expected in [
            (["crates/vpdf-dev/benches/micro.rs"], None),
            (
                ["crates/vpdf-dev/benches/micro.rs", "src/input.rs"],
                "rdeps(=vpdf)",
            ),
        ]:
            with (
                self.subTest(paths=paths),
                patch.object(
                    hooks, "package_folders", return_value=self.packages
                ),
                patch.object(hooks, "execute") as run,
            ):
                hooks.affected_tests(Path("/snapshot"), paths, {})
            commands = [call.args[0] for call in run.call_args_list]
            selections = [c[c.index("-E") + 1] for c in commands if "-E" in c]
            self.assertEqual(selections, [expected] if expected else [])

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
                ["tools/package/app.sh", "tools/bench/startup.py"],
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
                ["tools/build/worker.py", "tools/build/test_worker.py"],
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
                Path("/snapshot"), ["src/input.rs", "tools/package/app.sh"], {}
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
                Path("/snapshot"), ["tools/bench/live/runtime.py"], {}
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
            hooks.affected_tests(
                Path("/snapshot"), ["tools/bench/startup.py"], {}
            )

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
            "tools/check/width.sh",
            Path(hooks.__file__).with_name("width.sh").read_text(),
        )
        self.stage("README.md", "rustfmt.toml", "tools/check/width.sh")
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
        helpers = str(self.root / "git-core")
        with (
            patch.dict(
                os.environ,
                GIT_INDEX_FILE=str(index),
                GIT_EXEC_PATH=helpers,
                PATH=helpers + os.pathsep + os.environ["PATH"],
            ),
            self.snapshot() as (_, env),
        ):
            self.assertNotIn("GIT_INDEX_FILE", env)
            self.assertNotIn("GIT_EXEC_PATH", env)
            self.assertNotIn(helpers, env["PATH"].split(os.pathsep))
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
            "execute",
            side_effect=lambda command, source, env: seen.append(
                (source / "README.md").read_text()
            ),
        ):
            self.assertEqual(hooks.pre_push(self.root, io.StringIO(refs)), 0)
        self.assertCountEqual(seen, ["# Reader\n", "# Second\n"])
        self.assertEqual((self.root / "README.md").read_text(), "# Unstaged\n")

    def test_push_failure_blocks(self):
        commit = self.git("rev-parse", "HEAD").decode().strip()
        refs = f"refs/heads/main {commit} refs/heads/main {'0' * 40}\n"
        with (
            patch.object(
                hooks,
                "execute",
                side_effect=subprocess.CalledProcessError(1, "ci:check"),
            ) as check,
            self.assertRaises(subprocess.CalledProcessError),
        ):
            hooks.pre_push(self.root, io.StringIO(refs))
        self.assertEqual(check.call_args.args[0], ["mise", "run", "ci:check"])

    def test_annotated_tag_validation(self):
        commit = self.git("rev-parse", "HEAD").decode().strip()
        self.git("tag", "-am", "fixture", "fixture-tag")
        tag = self.git("rev-parse", "refs/tags/fixture-tag").decode().strip()
        null = "0" * 40
        refs = f"refs/heads/main {commit} refs/heads/main {null}\nrefs/tags/fixture-tag {tag} refs/tags/fixture-tag {null}\n"
        with patch.object(hooks, "execute") as checks:
            hooks.pre_push(self.root, io.StringIO(refs))
        self.assertEqual(checks.call_count, 1)

    def test_verify_waits_for_running_check(self):
        commit = self.git("rev-parse", "HEAD").decode().strip()
        started = []
        lock = hooks.check_path(self.root, commit, "lock")
        with patch.object(
            hooks, "execute", side_effect=lambda *_: started.append(1)
        ):
            with hooks.locked(lock):
                waiter = threading.Thread(
                    target=hooks.verify, args=(self.root, commit)
                )
                waiter.start()
                waiter.join(0.5)
                self.assertTrue(waiter.is_alive())
                self.assertEqual(started, [])
            waiter.join(10)
        self.assertEqual(started, [1])

    def test_push_deletion_skips(self):
        null = "0" * 40
        with patch.object(hooks, "execute") as checks:
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

    def test_legacy_checkout_hooks(self):
        self.write(
            "scripts/hooks.py",
            "import sys\nprint('legacy:' + ' '.join(sys.argv[1:]))\n",
        )
        hooks.install_hooks(self.root)
        result = subprocess.run(
            [self.root / ".git/hooks/pre-commit"],
            cwd=self.root,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(result.stdout.strip(), "legacy:pre-commit")

    def test_custom_hooks_directory(self):
        folder = self.root / "custom hooks"
        self.git("config", "core.hooksPath", str(folder))
        hooks.install_hooks(self.root)
        self.assertTrue((folder / "pre-commit").stat().st_mode & 0o111)
        self.assertTrue((folder / "pre-push").stat().st_mode & 0o111)

    def install_native(self, config=None):
        root = Path(hooks.__file__).resolve().parents[2]
        names = [
            "tools/__init__.py",
            "tools/check/__init__.py",
            "tools/check/hooks.py",
            "tools/build/__init__.py",
            "tools/build/cache.py",
            "tools/build/receipts.py",
            "tools/build/worker.py",
            "tools/shims/cargo",
        ]
        for name in names:
            self.write(name, (root / name).read_text())
        (self.root / "tools/shims/cargo").chmod(0o755)
        self.write(
            ".config/mise.toml",
            config or (root / ".config/mise.toml").read_text(),
        )
        self.stage(*names, ".config/mise.toml")
        self.commit()
        hooks.install_hooks(self.root)

    def test_native_push_tip(self):
        self.write(
            "tools/probe.py",
            "import sys\n"
            "from pathlib import Path\n\n"
            'print("checked:" + Path("README.md").read_text().strip())\n'
            "sys.exit(23)\n",
        )
        self.stage("tools/probe.py")
        self.install_native(
            '[tasks."ci:check"]\nrun = "python3 -B tools/probe.py"\n'
        )
        commit = self.git("rev-parse", "HEAD").decode().strip()
        self.write("README.md", "# Dirty caller\n")
        refs = f"refs/heads/main {commit} refs/heads/main {'0' * 40}\n"
        result = subprocess.run(
            [str(self.root / ".git/hooks/pre-push")],
            cwd=self.root,
            input=refs,
            text=True,
            capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked:# Reader", result.stdout)
        self.assertNotIn("checked:# Dirty caller", result.stdout)
        self.assertEqual(
            (self.root / "README.md").read_text(), "# Dirty caller\n"
        )
        self.assertEqual(self.git("show", ":README.md"), b"# Reader\n")

    def test_native_partial_preserved(self):
        self.install_native()
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
