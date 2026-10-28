import contextlib
import fcntl
import hashlib
import io
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.build import cache as build_cache
from tools.build import cleanup


class CacheTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(
                (
                    "VPDF_BUILD_",
                    "CARGO_BUILD_BUILD_DIR",
                    "CARGO_BUILD_TARGET",
                    "CARGO_TARGET_DIR",
                    "CI",
                )
            )
        }
        self.env.update(
            XDG_CACHE_HOME=str(self.root / "cache"),
            CARGO_HOME=str(self.root / "cargo-home"),
            VPDF_BUILD_CACHE_ID="0123456789abcdef",
        )
        self.build = build_cache.cache_root(self.env) / "build"

    def artifact(self):
        self.build.mkdir(parents=True, exist_ok=True)
        artifact = self.build / "artifact"
        artifact.write_bytes(b"x" * 8192)
        return artifact

    def test_prune_preserves_outputs(self):
        artifact = self.artifact()
        binary = self.root / "target/debug/vpdf"
        binary.parent.mkdir(parents=True)
        os.link(artifact, binary)
        build_cache.prune(self.build, 1)
        self.assertFalse(list(self.build.iterdir()))
        self.assertEqual(binary.read_bytes(), b"x" * 8192)

    def test_warm_cache_retained(self):
        artifact = self.artifact()
        build_cache.prune(self.build, 1024**2)
        self.assertTrue(artifact.exists())

    def test_prune_tolerates_cleaned_namespace(self):
        folder = self.build / "removed"
        folder.mkdir(parents=True)
        (folder / "artifact").write_bytes(b"x" * 8192)
        original = Path.lstat

        def cleaned(path, *args, **kwargs):
            if path == folder and folder.exists():
                shutil.rmtree(folder)
            return original(path, *args, **kwargs)

        with patch.object(Path, "lstat", cleaned):
            build_cache.prune(self.build, 1)
        self.assertFalse(folder.exists())

    def test_symlink_rejected(self):
        self.build.parent.mkdir(parents=True)
        self.build.symlink_to(self.root / "missing")
        with self.assertRaisesRegex(ValueError, "symlink"):
            build_cache.prune(self.build, 1)

    def test_cleanup_on_failed_build(self):
        self.env["VPDF_BUILD_CACHE_GIB"] = "0.000000001"

        def compile_artifact(*args, env, **kwargs):
            self.assertFalse((self.build / "artifact").exists())
            self.assertEqual(
                Path(env["CARGO_BUILD_BUILD_DIR"]).parent, self.build
            )
            (Path(env["CARGO_BUILD_BUILD_DIR"]) / "artifact").write_bytes(
                b"x" * 8192
            )
            return 7

        with (
            patch.dict(os.environ, self.env, clear=True),
            patch.object(build_cache, "invoke", compile_artifact),
        ):
            self.artifact()
            self.assertEqual(build_cache.run(["build"]), 7)
        self.assertFalse(list(self.build.iterdir()))

    def test_oldest_namespace_evicted(self):
        for index, name in enumerate(["old", "warm", "active"]):
            folder = self.build / name
            folder.mkdir(parents=True)
            (folder / "artifact").write_bytes(b"x" * 8192)
            os.utime(folder, (index, index))
        limit = build_cache.cache_bytes(self.build) - build_cache.cache_bytes(
            self.build / "old"
        )
        build_cache.prune(self.build, limit)
        self.assertFalse((self.build / "old").exists())
        self.assertTrue((self.build / "warm/artifact").exists())
        self.assertTrue((self.build / "active/artifact").exists())

    def test_explicit_directories_untouched(self):
        for extra, args in [
            ({"CARGO_BUILD_BUILD_DIR": str(self.build)}, ["build"]),
            ({}, ["build", "--build-dir", str(self.build)]),
            ({}, ["build", f"--build-dir={self.build}"]),
            ({"CI": "true"}, ["build"]),
        ]:
            with (
                self.subTest(extra=extra, args=args),
                patch.dict(os.environ, self.env | extra, clear=True),
                patch.object(build_cache, "invoke", return_value=0),
                patch.object(build_cache, "prune") as prune,
            ):
                self.assertEqual(build_cache.run(args), 0)
                prune.assert_not_called()

    def test_invalid_budget_rejected(self):
        for budget in ["0", "-1", "nan", "inf", "invalid"]:
            with (
                self.subTest(budget=budget),
                patch.dict(
                    os.environ,
                    self.env | {"VPDF_BUILD_CACHE_GIB": budget},
                    clear=True,
                ),
                self.assertRaises(ValueError),
            ):
                build_cache.run(["build"])

    def test_informational_commands_skip_housekeeping(self):
        for args in [["metadata"], ["fmt"], ["--version"], ["build", "--help"]]:
            with (
                self.subTest(args=args),
                patch.dict(os.environ, self.env, clear=True),
                patch.object(build_cache, "invoke", return_value=0) as invoke,
                patch.object(build_cache, "prune") as prune,
            ):
                self.assertEqual(build_cache.run(args), 0)
                prune.assert_not_called()
                self.assertEqual(
                    Path(
                        invoke.call_args.kwargs["env"]["CARGO_BUILD_BUILD_DIR"]
                    ).parent,
                    self.build,
                )
                self.assertFalse(self.build.exists())

    def test_manifest_and_directory_options(self):
        for args in [
            ["build", "--manifest-path", str(self.root / "Cargo.toml")],
            ["build", f"--manifest-path={self.root}/Cargo.toml"],
            ["-C", str(self.root), "build", "--manifest-path", "Cargo.toml"],
            [f"-C{self.root}", "build"],
        ]:
            with self.subTest(args=args):
                self.assertEqual(build_cache.source_root(args), self.root)

    def test_worktrees_keep_source_isolation(self):
        env = self.env.copy()
        env.pop("VPDF_BUILD_CACHE_ID")
        repo = self.root / "main"
        repo.mkdir()
        subprocess.run(["git", "init", "-q", repo], check=True)
        (repo / "Cargo.toml").write_text(
            '[package]\nname="cache-fixture"\nversion="0.1.0"\nedition="2024"\n'
        )
        (repo / "src").mkdir()
        (repo / "src/main.rs").write_text('fn main() { println!("first"); }\n')
        subprocess.run(
            ["git", "-C", repo, "add", "Cargo.toml", "src"], check=True
        )
        subprocess.run(
            [
                "git",
                "-C",
                repo,
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-qm",
                "fixture",
            ],
            check=True,
        )
        other = self.root / "branch"
        subprocess.run(
            ["git", "-C", repo, "worktree", "add", "-qb", "test", other],
            check=True,
        )
        self.assertEqual(
            build_cache.cache_key(repo), build_cache.cache_key(other)
        )
        (other / "src/main.rs").write_text(
            'fn main() { println!("second"); }\n'
        )
        script = Path(build_cache.__file__).resolve()
        for tree in [repo, other, repo]:
            subprocess.run(
                [sys.executable, "-B", script, "build", "--offline"],
                cwd=tree,
                env=env,
                check=True,
                capture_output=True,
            )
        build_cache.prune(build_cache.cache_root(env, repo) / "build", 1)
        for tree, expected in [(repo, "first"), (other, "second")]:
            binary = tree / "target/debug/cache-fixture"
            self.assertEqual(
                subprocess.check_output([binary], text=True).strip(), expected
            )
            self.assertFalse((tree / "target/debug/deps").exists())

    def cargo_stub(self, body):
        tools = self.root / "tools"
        tools.mkdir()
        cargo = tools / "cargo"
        cargo.write_text(
            f"#!{sys.executable}\n"
            "import os, sys, time\nfrom pathlib import Path\n"
            "root = Path(os.environ['TEST_CACHE_ROOT'])\n" + body
        )
        cargo.chmod(0o755)
        return self.env | {
            "PATH": str(tools) + os.pathsep + self.env["PATH"],
            "TEST_CACHE_ROOT": str(self.root),
            "VPDF_BUILD_CACHE_GIB": "0.000000001",
        }

    def wait_file(self, name):
        deadline = time.monotonic() + 10
        while not (self.root / name).exists():
            if time.monotonic() > deadline:
                self.fail(f"Timed out waiting for {name}")
            time.sleep(0.01)

    def test_cancellation_reaches_cargo(self):
        env = self.cargo_stub(
            "build = Path(os.environ['CARGO_BUILD_BUILD_DIR'])\n"
            "(build / 'artifact').write_bytes(b'x' * 8192)\n"
            "(root / 'pid').write_text(str(os.getpid()))\n"
            "(root / 'ready').touch()\n"
            "while True: time.sleep(0.01)\n"
        )
        script = Path(build_cache.__file__).resolve()
        wrapper = subprocess.Popen([sys.executable, script, "build"], env=env)
        reaped = False
        try:
            self.wait_file("ready")
            wrapper.send_signal(signal.SIGTERM)
            self.assertEqual(wrapper.wait(timeout=10), 128 + signal.SIGTERM)
            with self.assertRaises(ProcessLookupError):
                os.kill(int((self.root / "pid").read_text()), 0)
            reaped = True
            self.assertFalse(list(self.build.iterdir()))
        finally:
            if wrapper.poll() is None:
                wrapper.kill()
                wrapper.wait()
            if not reaped and (self.root / "pid").exists():
                try:
                    os.kill(
                        int((self.root / "pid").read_text()), signal.SIGTERM
                    )
                except ProcessLookupError:
                    pass

    def test_parallel_builds_preserve_active_cache(self):
        env = self.cargo_stub(
            "name = sys.argv[1]\n"
            "build = Path(os.environ['CARGO_BUILD_BUILD_DIR'])\n"
            "(build / name).write_bytes(b'x' * 8192)\n"
            "(root / name).touch()\n"
            "if name == 'first':\n"
            "    while not (root / 'release').exists(): time.sleep(0.01)\n"
        )
        script = Path(build_cache.__file__).resolve()
        first = subprocess.Popen([sys.executable, script, "first"], env=env)
        second = None
        try:
            self.wait_file("first")
            second = subprocess.Popen(
                [sys.executable, script, "second"], env=env
            )
            self.assertEqual(second.wait(timeout=10), 0)
            self.assertTrue(list(self.build.glob("*/first")))
            (self.root / "release").touch()
            self.assertEqual(first.wait(timeout=10), 0)
            self.assertTrue((self.root / "second").exists())
            self.assertFalse(list(self.build.iterdir()))
        finally:
            for process in [first, second]:
                if process and process.poll() is None:
                    process.kill()
                    process.wait()

    def test_pool_size(self):
        self.assertEqual(build_cache.pool_size({}), 4)
        self.assertEqual(build_cache.pool_size({"VPDF_BUILD_WORKERS": "3"}), 3)

    def test_budget_scales_with_pool(self):
        env = {"VPDF_BUILD_WORKERS": "3"}
        self.assertEqual(
            build_cache.cache_limit(env),
            3 * build_cache.DEFAULT_GIB * 1024**3,
        )
        env["VPDF_BUILD_CACHE_GIB"] = "2"
        self.assertEqual(build_cache.cache_limit(env), 2 * 1024**3)

    def test_cleanup_reclaims_removed_state(self):
        cache = self.build.parent
        removed, busy = self.root / "removed", self.root / "busy"
        names = {
            path: hashlib.sha256(os.fsencode(path)).hexdigest()
            for path in [removed, busy]
        }
        for name in names.values():
            (self.build / name[:16]).mkdir(parents=True)
        (self.build / "worker").mkdir()
        affinity = cache / "affinity" / names[removed]
        affinity.parent.mkdir()
        affinity.write_text("worker")
        records = cache / "receipts"
        records.mkdir()
        checks = cache / "checks"
        checks.mkdir()
        for path in [
            records / "old.json",
            checks / "old.log",
            checks / "old.lock",
            checks / "held.lock",
            checks / "merge.lock",
        ]:
            path.touch()
            os.utime(path, (0, 0))
        (records / "new.json").touch()
        orphan, live = cache.parent / "orphan", cache.parent / "live"
        for other, origin in [(orphan, self.root / "gone"), (live, self.root)]:
            other.mkdir()
            (other / "origin").write_bytes(os.fsencode(origin))
        leases = cache / "leases"
        leases.mkdir()
        with (
            build_cache.locked(checks / "held.lock", fcntl.LOCK_SH),
            build_cache.locked(leases / names[busy][:16], fcntl.LOCK_SH),
            patch.dict(os.environ, self.env, clear=True),
            patch.object(sys, "argv", ["cleanup", str(removed), str(busy)]),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(cleanup.main(), 0)
        self.assertFalse((self.build / names[removed][:16]).exists())
        self.assertTrue((self.build / names[busy][:16]).exists())
        self.assertTrue((self.build / "worker").exists())
        self.assertFalse(affinity.exists())
        self.assertEqual(
            [path.name for path in records.iterdir()], ["new.json"]
        )
        self.assertEqual(
            sorted(path.name for path in checks.iterdir()),
            ["held.lock", "merge.lock"],
        )
        self.assertTrue((leases / names[busy][:16]).exists())
        self.assertFalse(orphan.exists())
        self.assertTrue(live.exists())


if __name__ == "__main__":
    unittest.main()
