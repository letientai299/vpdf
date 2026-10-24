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

import build_cache
import build_worker
import hooks


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
        result = subprocess.run(
            ["cargo", "build", "--offline", "--message-format=json"],
            cwd=source,
            env=build_cache.cargo_env(env),
            check=True,
            capture_output=True,
            text=True,
            timeout=60,
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
        scripts = Path(build_worker.__file__).parent
        task = json.loads(
            subprocess.check_output(
                ["mise", "tasks", "info", "check", "--json"],
                cwd=scripts.parent,
            )
        )
        for name in ["build_worker.py", "build_cache.py"]:
            self.write(f"scripts/{name}", (scripts / name).read_text())
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
            ({}, ["--example", "bench-fixtures"]),
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
                if extra or arguments[0] in {"--target-dir", "--example"}:
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
        from help_assets import OUTPUTS

        target = self.root / "worker/target"
        generation = target / "help/generation"
        generation.mkdir(parents=True)
        for name in (*OUTPUTS, "manifest.json"):
            (generation / name).write_text("asset")
        (generation / "notices").mkdir()
        (generation / "notices/source.txt").write_text("renderer input")
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
            destination = self.repo / "target/help/generation"
            self.assertEqual(
                {path.name for path in destination.iterdir()},
                {*OUTPUTS, "manifest.json"},
            )
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

    def test_concurrent_requests(self):
        script = (
            "import sys; from pathlib import Path; "
            f"sys.path.insert(0, {str(Path(build_worker.__file__).parent)!r}); "
            "from build_worker import worker_snapshot, working_files; "
            "root = Path(sys.argv[1]); "
            "context = worker_snapshot(root, lambda: working_files(root)); "
            "context.__enter__(); context.__exit__(None, None, None)"
        )
        with self.snapshot() as (source, _):
            with self.assertRaises(subprocess.TimeoutExpired):
                subprocess.run(
                    [sys.executable, "-c", script, str(self.other)],
                    env=self.env,
                    check=True,
                    timeout=0.5,
                )
            self.assertIn('"first"', (source / "src/value.rs").read_text())
        subprocess.run(
            [sys.executable, "-c", script, str(self.other)],
            env=self.env,
            check=True,
            timeout=10,
        )

    def test_legacy_cleanup(self):
        cache = build_cache.cache_root(self.env, self.repo)
        leases = cache / "leases"
        leases.mkdir(parents=True)
        build = cache / "build"
        names = ["0" * 16, "1" * 16, "worker"]
        for name in names:
            (build / name).mkdir(parents=True)
            (build / name / "artifact").write_bytes(b"keep")
        with build_cache.locked(leases / names[0], fcntl.LOCK_SH):
            build_worker.clean_old(self.repo)
        self.assertTrue((build / names[0] / "artifact").exists())
        self.assertFalse((build / names[1]).exists())
        self.assertTrue((build / "worker/artifact").exists())

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
                build_worker.__file__,
                "--make",
                "-j32",
                "-f",
                makefile,
            ],
            env=self.env | {"CARGO_BUILD_JOBS": "2"},
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertEqual(json.loads(state.read_text()), [0, 2])

    def test_inherited_lease(self):
        ready = self.root / "ready"
        child = (
            "import os, time; from pathlib import Path; "
            f"Path({str(ready)!r}).write_text(str(os.getpid())); "
            "time.sleep(30)"
        )
        script = (
            "import sys; from pathlib import Path; "
            f"sys.path.insert(0, {str(Path(build_worker.__file__).parent)!r}); "
            "from build_worker import worker_snapshot, working_files, invoke; "
            f"root = Path({str(self.repo)!r}); "
            "context = worker_snapshot(root, lambda: working_files(root)); "
            "source, env = context.__enter__(); "
            f"invoke([sys.executable, '-c', {child!r}], env=env)"
        )
        wrapper = subprocess.Popen([sys.executable, "-c", script], env=self.env)
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
            cache = build_cache.cache_root(self.env, self.repo)
            with self.assertRaises(BlockingIOError):
                with build_cache.locked(
                    cache / "leases/worker", fcntl.LOCK_EX | fcntl.LOCK_NB
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
