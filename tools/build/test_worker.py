import contextlib
import fcntl
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.build import cache as build_cache
from tools.build import worker as build_worker
from tools.check import hooks

ROOT = Path(__file__).resolve().parents[2]


class WorkerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("GIT_", "VPDF_BUILD_", "CARGO_", "CI"))
        }
        self.env.update(
            XDG_CACHE_HOME=str(self.root / "cache"),
            CARGO_HOME=str(self.root / "cargo-home"),
            NEXTEST_TEST_THREADS="4",
            VPDF_BUILD_WORKERS="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_CONFIG_NOSYSTEM="1",
        )
        self.environment = patch.dict(os.environ, self.env, clear=True)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.git("init", "-q")
        self.git("config", "user.name", "Worker Tests")
        self.git("config", "user.email", "worker@example.invalid")
        self.git("config", "core.hooksPath", os.devnull)
        self.write(".gitignore", "/target/\n/tmp/\n")
        self.write(
            "Cargo.toml",
            '[package]\nname="worker-fixture"\nversion="0.1.0"\nedition="2024"\n',
        )
        self.write(
            "src/main.rs",
            'mod value;\nfn main() { println!("{}", value::VALUE); }\n',
        )
        self.write("src/value.rs", 'pub const VALUE: &str = "first";\n')
        self.git("add", "--", ".gitignore", "Cargo.toml", "src")
        self.git("commit", "-qm", "test: fixture")
        self.other = self.root / "other"
        self.git("worktree", "add", "-qb", "other", str(self.other))

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo)

    def write(self, name, content):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    @contextlib.contextmanager
    def snapshot(self, root=None):
        root = root or self.repo
        with build_worker.worker_snapshot(
            root, lambda: build_worker.working_files(root)
        ) as state:
            yield state

    def test_crate_requires_package(self):
        with self.assertRaisesRegex(ValueError, "requires a package"):
            build_worker.run(["test:crate"])

    def test_worktree_reuse(self):
        with self.snapshot() as (source, first):
            timestamp = (source / "src/main.rs").stat().st_mtime_ns
        with self.snapshot(self.other) as (other, second):
            self.assertEqual(other, source)
            self.assertEqual(
                (other / "src/main.rs").stat().st_mtime_ns, timestamp
            )
            self.assertEqual(
                first["CARGO_BUILD_BUILD_DIR"], second["CARGO_BUILD_BUILD_DIR"]
            )
            self.assertEqual(
                first["CARGO_TARGET_DIR"], second["CARGO_TARGET_DIR"]
            )

    def test_queued_snapshot_frozen(self):
        original = self.git("rev-parse", "HEAD").strip()
        locked = build_worker.locked

        @contextlib.contextmanager
        def change_while_waiting(path, *args):
            if path.name == "worker":
                self.write("src/value.rs", 'pub const VALUE: &str = "later";\n')
                self.git("commit", "--allow-empty", "-qm", "test: later")
            with locked(path, *args) as lease:
                yield lease

        with patch.object(build_worker, "locked", change_while_waiting):
            with self.snapshot() as (source, _):
                self.assertIn('"first"', (source / "src/value.rs").read_text())
                self.assertEqual(
                    subprocess.check_output(
                        ["git", "rev-parse", "HEAD"], cwd=source
                    ).strip(),
                    original,
                )
        self.assertIn('"later"', (self.repo / "src/value.rs").read_text())

    def test_working_inputs(self):
        self.write("src/untracked.rs", "pub fn new() {}\n")
        self.write("target/ignored", "ignored")
        self.write("tmp/corpus/paper.pdf", "input")
        with self.snapshot() as (source, _):
            self.assertTrue((source / "src/untracked.rs").exists())
            self.assertFalse((source / "target").exists())
            self.assertEqual((source / "tmp").resolve(), self.repo / "tmp")
        (self.repo / "src/value.rs").unlink()
        (self.repo / "src/untracked.rs").unlink()
        with self.snapshot() as (source, _):
            self.assertFalse((source / "src/value.rs").exists())
            self.assertFalse((source / "src/untracked.rs").exists())

    def test_staged_hooks(self):
        tree = self.git("write-tree").decode().strip()
        self.write("src/value.rs", "invalid unstaged Rust\n")
        with hooks.snapshot(self.repo, tree) as (staged, _):
            self.assertIn('"first"', (staged / "src/value.rs").read_text())
        with self.snapshot() as (working, _):
            self.assertEqual(working, staged)
            self.assertEqual(
                (working / "src/value.rs").read_text(),
                "invalid unstaged Rust\n",
            )

    def test_snapshot_identity(self):
        first = self.git("rev-parse", "HEAD").strip()
        self.git("tag", "v0.1.0")
        self.write("src/value.rs", 'pub const VALUE: &str = "other";\n')
        self.git("add", "--", "src/value.rs")
        self.git("commit", "-qm", "test: change")
        original = (self.repo / ".git/index").read_bytes()
        with hooks.snapshot(self.repo, first.decode()) as (source, _):

            def git(*args):
                return subprocess.check_output(
                    ["git", *args], cwd=source
                ).strip()

            self.assertEqual(git("rev-parse", "HEAD"), first)
            self.assertEqual(
                git("describe", "--tags", "--exact-match"), b"v0.1.0"
            )
            self.assertEqual(git("status", "--porcelain"), b"")
            timestamp = (source / ".git/HEAD").stat().st_mtime_ns
        with hooks.snapshot(self.repo, first.decode()) as (source, _):
            self.assertEqual(
                (source / ".git/HEAD").stat().st_mtime_ns, timestamp
            )
        self.assertEqual((self.repo / ".git/index").read_bytes(), original)

    def test_build_inputs(self):
        self.write(
            "build.rs",
            'fn main() { println!("cargo::rerun-if-changed=input.txt"); }\n',
        )
        self.write("input.txt", "first")
        with self.snapshot() as (source, env):
            first = self.compile(source, env)
        changed = self.write("input.txt", "other")
        os.utime(changed, (1, 1))
        with self.snapshot() as (source, env):
            self.assertGreater((source / "input.txt").stat().st_mtime_ns, 10**9)
            second = self.compile(source, env)
        self.assertTrue(any(not record["fresh"] for record in first))
        self.assertTrue(any(not record["fresh"] for record in second))

    def compile(self, source, env):
        try:
            result = subprocess.run(
                ["cargo", "build", "--offline", "--message-format=json"],
                cwd=source,
                env=build_cache.cargo_env(env),
                check=True,
                capture_output=True,
                text=True,
                timeout=60,
            )
        except (
            subprocess.TimeoutExpired,
            subprocess.CalledProcessError,
        ) as error:

            def text(value):
                return (
                    value.decode(errors="replace")
                    if isinstance(value, bytes)
                    else value or ""
                )

            self.fail(
                f"Fixture Cargo build failed\n{error}\n"
                f"stdout:\n{text(error.stdout)}\nstderr:\n{text(error.stderr)}"
            )
        return [
            record
            for line in result.stdout.splitlines()
            if (record := json.loads(line)).get("reason") == "compiler-artifact"
        ]

    def test_branch_reuse(self):
        with self.snapshot() as (source, env):
            first = self.compile(source, env)
            binary = Path(env["CARGO_TARGET_DIR"]) / "debug/worker-fixture"
            retained = self.repo / "target/retained"
            build_worker.copy_binary(binary, retained)
        with self.snapshot(self.other) as (source, env):
            unchanged = self.compile(source, env)
        self.assertTrue(any(not record["fresh"] for record in first))
        self.assertTrue(all(record["fresh"] for record in unchanged))
        changed = self.other / "src/value.rs"
        original = changed.stat()
        changed.write_text('pub const VALUE: &str = "other";\n')
        os.utime(changed, ns=(original.st_atime_ns, original.st_mtime_ns))
        for tree, expected in [(self.other, "other"), (self.repo, "first")]:
            with self.snapshot(tree) as (source, env):
                records = self.compile(source, env)
                self.assertTrue(any(not record["fresh"] for record in records))
                self.assertEqual(
                    subprocess.check_output([binary], text=True).strip(),
                    expected,
                )
        self.assertEqual(
            subprocess.check_output([retained], text=True).strip(), "first"
        )
        cache = build_cache.cache_root(self.env, self.repo)
        self.assertEqual(
            [p.name for p in (cache / "build").iterdir()], ["worker"]
        )

    def test_cargo_cache_lock(self):
        shared = self.root / "shared-cargo-home"
        shared.mkdir()
        with (shared / ".package-cache").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            with self.snapshot() as (source, env):
                inherited = {"CARGO_HOME": str(shared), **env}
                records = self.compile(source, inherited)
        self.assertTrue(any(not record["fresh"] for record in records))

    def test_compile_diagnostics(self):
        for error in [
            subprocess.TimeoutExpired(
                "cargo",
                60,
                output=b"fixture stdout",
                stderr=b"cache locked",
            ),
            subprocess.CalledProcessError(
                1,
                "cargo",
                output="fixture stdout",
                stderr="compiler failed",
            ),
        ]:
            with (
                self.subTest(error=type(error).__name__),
                patch.object(subprocess, "run", side_effect=error),
                self.assertRaises(AssertionError) as failure,
            ):
                self.compile(self.repo, self.env)
            self.assertIn("fixture stdout", str(failure.exception))
            detail = (
                error.stderr.decode()
                if isinstance(error.stderr, bytes)
                else error.stderr
            )
            self.assertIn(detail, str(failure.exception))

    def test_nested_tasks(self):
        with self.snapshot() as (source, env):
            with (
                patch.dict(os.environ, env, clear=True),
                patch.object(Path, "cwd", return_value=source),
                patch.object(build_worker, "invoke", return_value=0) as invoke,
            ):
                self.assertEqual(build_worker.run(["test"]), 0)
                self.assertEqual(invoke.call_args.kwargs["cwd"], source)

    def test_mutating_tasks(self):
        for task in ["lint", "hooks", "install", "try"]:
            with self.subTest(task=task), self.assertRaises(ValueError):
                build_worker.run([task])

    def test_ci_layout(self):
        for extra in [
            {"CI": "true"},
            {"VPDF_BUILD_MODE": "isolated"},
            {"CARGO_BUILD_BUILD_DIR": str(self.root / "explicit")},
        ]:
            with (
                self.subTest(extra=extra),
                patch.dict(os.environ, extra),
                patch.object(Path, "cwd", return_value=self.repo),
                patch.object(build_worker, "worker_snapshot") as snapshot,
                patch.object(build_worker, "invoke", return_value=0) as invoke,
            ):
                self.assertEqual(build_worker.run(["test"]), 0)
            snapshot.assert_not_called()
            self.assertEqual(invoke.call_args.kwargs["cwd"], self.repo)

    def test_default_task(self):
        task = json.loads(
            subprocess.check_output(
                ["mise", "tasks", "info", "check", "--json"],
                cwd=ROOT,
            )
        )
        for name in [
            "tools/__init__.py",
            "tools/build/__init__.py",
            "tools/build/worker.py",
            "tools/build/cache.py",
        ]:
            self.write(name, (ROOT / name).read_text())
        self.write(
            ".config/mise.toml",
            f"[tasks.check]\nraw_args = {str(task.get('raw_args', False)).lower()}\n"
            f"run = {json.dumps(task['run'][0])}\n"
            '[tasks._check]\nhide = true\ndepends = ["record"]\n'
            'run = "python3 record.py task"\n'
            '[tasks.record]\nrun = "python3 record.py dependency"\n',
        )
        self.write(
            "record.py",
            "import json, os, sys\nfrom pathlib import Path\n"
            "with open(os.environ['WORKER_TASK_LOG'], 'a') as stream:\n"
            "    stream.write(json.dumps([str(Path.cwd()), sys.argv[1:]]) + '\\n')\n",
        )
        output = self.root / "calls.jsonl"
        for extra, arguments in [
            ({}, ["--flag", "a b"]),
            ({"VPDF_BUILD_MODE": "isolated"}, ["--flag", "a b"]),
            ({}, ["--target-dir", "custom output"]),
            ({}, ["--bin", "bench-fixtures"]),
        ]:
            with self.subTest(extra=extra):
                output.unlink(missing_ok=True)
                subprocess.run(
                    ["mise", "run", "check", "--", *arguments],
                    cwd=self.repo,
                    env=self.env
                    | extra
                    | {
                        "WORKER_TASK_LOG": str(output),
                        "MISE_AUTO_INSTALL": "false",
                        "MISE_TRUSTED_CONFIG_PATHS": str(self.repo),
                    },
                    check=True,
                    capture_output=True,
                    timeout=20,
                )
                rows = [
                    json.loads(line) for line in output.read_text().splitlines()
                ]
                self.assertEqual(len(rows), 2)
                self.assertEqual(rows[0][1], ["dependency"])
                self.assertEqual(rows[1][1], ["task", *arguments])
                if extra or arguments[0] in {"--target-dir", "--bin"}:
                    self.assertTrue(
                        all(Path(row[0]) == self.repo for row in rows)
                    )
                else:
                    self.assertTrue(
                        all(Path(row[0]) != self.repo for row in rows)
                    )
                    self.assertEqual(rows[0][0], rows[1][0])

    def test_app_outputs(self):
        target = self.root / "worker/target"
        bundle = target / "dist/Vpdf.app/Contents/MacOS"
        bundle.mkdir(parents=True)
        (bundle / "vpdf-gui").write_text("first")
        (target / "dist/Vpdf.dmg").write_text("image")
        build_worker.copy_outputs("build:app", self.repo, target, {})
        destination = self.repo / "target/dist"
        self.assertEqual(
            (destination / "Vpdf.app/Contents/MacOS/vpdf-gui").read_text(),
            "first",
        )
        self.assertEqual((destination / "Vpdf.dmg").read_text(), "image")

    def test_ci_outputs(self):
        from tools.check.ci import archive_paths

        target = self.root / "worker/target"
        archive, digest = archive_paths(self.repo, {})
        names = [
            "release/vpdf",
            "dist/vpdf",
            f"dist/{archive.name}",
            f"dist/{digest.name}",
        ]
        for name in names:
            path = target / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        build_worker.copy_outputs("ci:check", self.repo, target, {})
        for name in names:
            self.assertEqual((self.repo / "target" / name).read_text(), name)

    def test_install_outputs(self):
        target = self.root / "worker/target"
        (target / "release").mkdir(parents=True)
        (target / "release/vpdf").write_text("cli")
        bundle = target / "dist/Vpdf.app/Contents/MacOS"
        bundle.mkdir(parents=True)
        (bundle / "vpdf-gui").write_text("app")
        stale = self.write("target/dist/Vpdf.dmg", "stale")
        build_worker.copy_outputs("build:install", self.repo, target, {})
        self.assertEqual((self.repo / "target/release/vpdf").read_text(), "cli")
        self.assertEqual(
            (
                self.repo / "target/dist/Vpdf.app/Contents/MacOS/vpdf-gui"
            ).read_text(),
            "app",
        )
        self.assertFalse(stale.exists())

    def test_isolated_binary(self):
        binary = self.write("target/triple/debug/vpdf", "debug")
        with (
            patch.dict(
                os.environ,
                VPDF_BUILD_MODE="isolated",
                CARGO_BUILD_TARGET="triple",
            ),
            patch.object(Path, "cwd", return_value=self.repo),
            patch.object(build_worker, "invoke", return_value=0),
        ):
            self.assertEqual(build_worker.run(["build:debug"]), 0)
        self.assertEqual(
            (self.repo / "target/debug/vpdf").read_bytes(), binary.read_bytes()
        )

    def test_help_outputs(self):
        from tools.help.assets import OUTPUTS

        target = self.root / "worker/target"
        generation = target / "help/generation"
        generation.mkdir(parents=True)
        for name in (*OUTPUTS, "manifest.json"):
            (generation / name).write_text("asset")
        (generation / "notices").mkdir()
        (generation / "notices/source.txt").write_text("renderer input")
        (target / "help/help.pdf").write_text("readable manual")
        for name in ["release/vpdf", "dist/vpdf"]:
            path = target / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        for task in ["help:build", "build", "release"]:
            with (
                self.subTest(task=task),
                patch.object(
                    subprocess, "check_output", return_value=str(generation)
                ),
            ):
                build_worker.copy_outputs(
                    task,
                    self.repo,
                    target,
                    {
                        "VPDF_BUILD_WORKER_FD": "3",
                        "VPDF_BUILD_WORKER": str(target.parent / "source"),
                    },
                )
            link = self.repo / "target/help/help.pdf"
            self.assertFalse(link.is_symlink())
            self.assertEqual(link.read_text(), "readable manual")
            destination = self.repo / "target/help/generation"
            self.assertEqual(
                {path.name for path in destination.iterdir()},
                {*OUTPUTS, "manifest.json"},
            )
            current = self.repo / "target/help/.current"
            self.assertEqual(current.read_text(), "generation\n")
            self.assertEqual(
                [p.name for p in destination.parent.glob(".copy-*")], []
            )
        (destination / "base.pdf").write_text("partial")
        with patch.object(
            subprocess, "check_output", return_value=str(generation)
        ):
            build_worker.copy_help(
                self.repo / "target", target, {"VPDF_BUILD_WORKER_FD": "3"}
            )
        self.assertEqual((destination / "base.pdf").read_text(), "asset")
        for name in ["release/vpdf", "dist/vpdf"]:
            self.assertEqual((self.repo / "target" / name).read_text(), name)

    def test_path_transitions(self):
        with self.snapshot():
            pass
        path = self.repo / "src/value.rs"
        path.unlink()
        path.mkdir()
        (path / "mod.rs").write_text("pub fn value() {}\n")
        with self.snapshot() as (source, _):
            self.assertTrue((source / "src/value.rs/mod.rs").is_file())
        (path / "mod.rs").unlink()
        path.rmdir()
        path.write_text('pub const VALUE: &str = "first";\n')
        with self.snapshot() as (source, _):
            self.assertTrue((source / "src/value.rs").is_file())

    def test_failed_outputs(self):
        output = self.write("target/release/vpdf", "previous")
        with (
            patch.object(Path, "cwd", return_value=self.repo),
            patch.object(build_worker, "invoke", return_value=7),
        ):
            self.assertEqual(build_worker.run(["build"]), 7)
        self.assertEqual(output.read_text(), "previous")
        target = self.root / "worker/target"
        candidate = target / "release/vpdf"
        candidate.parent.mkdir(parents=True)
        candidate.write_text("replacement")
        with (
            patch.object(
                build_worker,
                "copy_help",
                side_effect=OSError("Help copy failed"),
            ),
            self.assertRaises(OSError),
        ):
            build_worker.copy_outputs(
                "build",
                self.repo,
                target,
                {
                    "VPDF_BUILD_WORKER": str(target.parent / "source"),
                },
            )
        self.assertEqual(output.read_text(), "previous")

    def test_scan_errors_keep_task_errors(self):
        with patch.object(
            build_cache, "cache_bytes", side_effect=ValueError("symlink")
        ):
            with self.assertRaisesRegex(ValueError, "symlink"):
                with self.snapshot():
                    pass
            with self.assertRaisesRegex(RuntimeError, "task failed"):
                with self.snapshot():
                    raise RuntimeError("task failed")

    def test_limit_overrides(self):
        with (
            patch.dict(
                os.environ, CARGO_BUILD_JOBS="2", NEXTEST_TEST_THREADS="3"
            ),
            self.snapshot() as (_, env),
        ):
            self.assertEqual(env["CARGO_BUILD_JOBS"], "2")
            self.assertEqual(env["NEXTEST_TEST_THREADS"], "3")

    def test_active_eviction(self):
        with self.snapshot() as (source, env):
            build = source.parent.parent
            build_cache.prune(build, 1, build.parent / "leases")
            self.assertTrue(source.exists())
            self.assertTrue(
                Path(env["CARGO_TARGET_DIR"]).is_relative_to(source.parent)
            )

    def snapshot_script(self):
        return (
            "import sys; from pathlib import Path; "
            "from tools.build.worker import worker_snapshot, working_files; "
            "root = Path(sys.argv[1]); "
            "context = worker_snapshot(root, lambda: working_files(root)); "
            "context.__enter__(); context.__exit__(None, None, None)"
        )

    def test_concurrent_requests(self):
        script = self.snapshot_script()
        with self.snapshot() as (source, _):
            with self.assertRaises(subprocess.TimeoutExpired):
                subprocess.run(
                    [sys.executable, "-c", script, str(self.other)],
                    cwd=ROOT,
                    env=self.env,
                    check=True,
                    timeout=0.5,
                )
            self.assertIn('"first"', (source / "src/value.rs").read_text())
        subprocess.run(
            [sys.executable, "-c", script, str(self.other)],
            cwd=ROOT,
            env=self.env,
            check=True,
            timeout=10,
        )

    def test_pool_affinity_and_isolation(self):
        with patch.dict(os.environ, VPDF_BUILD_WORKERS="2"):
            with self.snapshot() as (first, first_env):
                self.assertEqual(first.parent.name, "worker")
                self.assertEqual(first_env["CARGO_BUILD_JOBS"], "16")
                self.assertEqual(first_env["NEXTEST_TEST_THREADS"], "4")
                (self.other / "src/value.rs").write_text("other snapshot")
                with self.snapshot(self.other) as (second, second_env):
                    self.assertEqual(second.parent.name, "worker-2")
                    self.assertNotEqual(first, second)
                    self.assertEqual(
                        (second / "src/value.rs").read_text(), "other snapshot"
                    )
                    self.assertIn(
                        '"first"', (first / "src/value.rs").read_text()
                    )
                    self.assertEqual(
                        (second / "tmp").resolve(), self.other / "tmp"
                    )
                    self.assertNotEqual(
                        first_env["CARGO_TARGET_DIR"],
                        second_env["CARGO_TARGET_DIR"],
                    )
                    build_cache.prune(
                        first.parent.parent, 1, first.parents[2] / "leases"
                    )
                    self.assertTrue(first.exists())
                    self.assertTrue(second.exists())
                    script = self.snapshot_script()
                    with self.assertRaises(subprocess.TimeoutExpired):
                        subprocess.run(
                            [sys.executable, "-c", script, str(self.repo)],
                            env=dict(os.environ),
                            timeout=0.5,
                            check=True,
                            capture_output=True,
                        )
            with self.snapshot(self.other) as (again, _):
                self.assertEqual(again, second)

    def test_mixed_modes_reserve_budget(self):
        cache = build_cache.cache_root(self.env, self.repo)
        (cache / "leases").mkdir(parents=True)
        with patch.dict(os.environ, VPDF_BUILD_WORKERS="2"):
            with build_cache.locked(cache / "leases/budget-0"):
                context = self.snapshot(self.other)
                second, _ = context.__enter__()
            try:
                self.assertEqual(second.parent.name, "worker-2")
                with self.assertRaises(BlockingIOError):
                    with build_worker.worker_lease(cache, self.repo, 2):
                        with build_cache.locked(
                            cache / "leases/budget-0",
                            fcntl.LOCK_EX | fcntl.LOCK_NB,
                        ):
                            pass
                script = self.snapshot_script()
                with self.assertRaises(subprocess.TimeoutExpired):
                    subprocess.run(
                        [sys.executable, "-c", script, str(self.repo)],
                        env=self.env,
                        timeout=0.5,
                        check=True,
                        capture_output=True,
                    )
            finally:
                context.__exit__(None, None, None)
        with self.snapshot() as (source, env):
            self.assertEqual(source.parent.name, "worker")
            self.assertEqual(len(env["VPDF_BUILD_RESOURCE_FDS"].split(",")), 2)

    def test_legacy_admission(self):
        cache = build_cache.cache_root(self.env, self.repo)
        (cache / "leases").mkdir(parents=True)
        with build_cache.locked(cache / "leases/worker"):
            with self.assertRaises(subprocess.TimeoutExpired):
                subprocess.run(
                    [
                        sys.executable,
                        "-c",
                        self.snapshot_script(),
                        str(self.repo),
                    ],
                    env=self.env | {"VPDF_BUILD_WORKERS": "2"},
                    timeout=0.5,
                    check=True,
                    capture_output=True,
                )

    def test_pool_children_keep_leases(self):
        with patch.dict(os.environ, VPDF_BUILD_WORKERS="2"):
            with self.snapshot():
                with self.snapshot(self.other) as (source, env):
                    output = self.root / "descriptors"
                    script = (
                        "import os; from pathlib import Path; "
                        "from tools.build.cache import worker_descriptors; "
                        f"Path({str(output)!r}).write_text(str(len(worker_descriptors(os.environ))))"
                    )
                    self.assertEqual(
                        build_cache.invoke(
                            [sys.executable, "-c", script], env=env, cwd=ROOT
                        ),
                        0,
                    )
                    self.assertEqual(output.read_text(), "3")
                    broken = env | {"VPDF_BUILD_RESOURCE_FDS": "1:999999"}
                    self.assertEqual(build_cache.worker_descriptors(broken), ())

    def test_pool_of_three(self):
        third = self.root / "third"
        self.git("worktree", "add", "-qb", "third", str(third))
        with (
            patch.dict(os.environ, VPDF_BUILD_WORKERS="3"),
            self.snapshot() as (first, first_env),
            self.snapshot(self.other) as (second, _),
            self.snapshot(third) as (last, last_env),
        ):
            self.assertEqual(
                [first.parent.name, second.parent.name, last.parent.name],
                ["worker", "worker-2", "worker-3"],
            )
            self.assertEqual(first_env["CARGO_BUILD_JOBS"], "16")
            self.assertEqual(last_env["NEXTEST_TEST_THREADS"], "4")
            self.assertIn("2:", last_env["VPDF_BUILD_RESOURCE_FDS"])

    def test_invalid_pool_limits(self):
        for changes in [
            {"VPDF_BUILD_WORKERS": "0"},
            {"VPDF_BUILD_WORKERS": "9"},
            {"VPDF_BUILD_WORKERS": "invalid"},
        ]:
            with self.subTest(changes=changes), patch.dict(os.environ, changes):
                with self.assertRaises(ValueError), self.snapshot():
                    pass

    def test_legacy_snapshot_serializes(self):
        self.write("tools/build/cache.py", "# older validation scripts\n")
        with (
            patch.dict(os.environ, VPDF_BUILD_WORKERS="2"),
            self.snapshot() as (source, env),
        ):
            self.assertEqual(source.parent.name, "worker")
            self.assertEqual(len(env["VPDF_BUILD_RESOURCE_FDS"].split(",")), 2)

    def test_two_slot_snapshot_keeps_pool(self):
        for root in [self.repo, self.other]:
            path = root / "tools/build/cache.py"
            path.parent.mkdir(parents=True)
            path.write_text("WORKER_PROTOCOL = 2\n")
        with (
            patch.dict(os.environ, VPDF_BUILD_WORKERS="4"),
            self.snapshot(),
            self.snapshot(self.other) as (second, _),
        ):
            self.assertEqual(second.parent.name, "worker-2")

    def test_native_limit(self):
        counter = self.root / "counter.py"
        state = self.root / "state.json"
        state.write_text("[0, 0]")
        counter.write_text(
            "import fcntl, json, sys, time\n"
            "def change(delta):\n"
            "    with open(sys.argv[1], 'r+') as stream:\n"
            "        fcntl.flock(stream, fcntl.LOCK_EX)\n"
            "        active, peak = json.load(stream)\n"
            "        active += delta\n"
            "        stream.seek(0)\n"
            "        json.dump([active, max(active, peak)], stream)\n"
            "        stream.truncate()\n"
            "change(1)\n"
            "time.sleep(0.1)\n"
            "change(-1)\n"
        )
        makefile = self.root / "Makefile"
        makefile.write_text(
            f"all: a b c d\na b c d:\n\t@'{sys.executable}' '{counter}' '{state}'\n"
        )
        subprocess.run(
            [
                sys.executable,
                "-m",
                "tools.build.worker",
                "--make",
                "-j32",
                "-f",
                makefile,
            ],
            cwd=ROOT,
            env=self.env | {"CARGO_BUILD_JOBS": "2"},
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(json.loads(state.read_text()), [0, 2])

    def test_inherited_lease(self):
        self.assert_inherited(1, "worker")
        self.assert_inherited(2, "worker-2")

    def assert_inherited(self, count, name):
        cache = build_cache.cache_root(self.env, self.repo)
        (cache / "leases").mkdir(parents=True, exist_ok=True)
        if count == 2:
            with patch.dict(os.environ, VPDF_BUILD_WORKERS="2"):
                with build_cache.locked(cache / "leases/budget-0"):
                    with self.snapshot():
                        pass
        ready = self.root / "ready"
        ready.unlink(missing_ok=True)
        child = (
            "import os, time; from pathlib import Path; "
            f"Path({str(ready)!r}).write_text(str(os.getpid())); "
            "time.sleep(30)"
        )
        script = (
            "import sys; from pathlib import Path; "
            "from tools.build.worker import worker_snapshot, working_files, invoke; "
            f"root = Path({str(self.repo)!r}); "
            "context = worker_snapshot(root, lambda: working_files(root)); "
            "source, env = context.__enter__(); "
            f"invoke([sys.executable, '-c', {child!r}], env=env)"
        )
        wrapper = subprocess.Popen(
            [sys.executable, "-c", script],
            cwd=ROOT,
            env=self.env | {"VPDF_BUILD_WORKERS": str(count)},
        )
        pid = None
        try:
            deadline = time.monotonic() + 10
            while not ready.exists():
                if time.monotonic() > deadline:
                    self.fail("Child failed to start")
                time.sleep(0.01)
            pid = int(ready.read_text())
            wrapper.kill()
            wrapper.wait(timeout=10)
            with self.assertRaises(BlockingIOError):
                with build_cache.locked(
                    cache / "leases" / name, fcntl.LOCK_EX | fcntl.LOCK_NB
                ):
                    pass
            if count == 2:
                with self.assertRaises(BlockingIOError):
                    with build_cache.locked(
                        cache / "leases/worker", fcntl.LOCK_EX | fcntl.LOCK_NB
                    ):
                        pass
            for slot in range(2) if count == 1 else [1]:
                with self.assertRaises(BlockingIOError):
                    with build_cache.locked(
                        cache / f"leases/budget-{slot}",
                        fcntl.LOCK_EX | fcntl.LOCK_NB,
                    ):
                        pass
        finally:
            if wrapper.poll() is None:
                wrapper.kill()
                wrapper.wait(timeout=10)
            if pid:
                with contextlib.suppress(ProcessLookupError):
                    os.kill(pid, signal.SIGTERM)


if __name__ == "__main__":
    unittest.main()
