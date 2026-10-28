import argparse
import contextlib
import io
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from tools.build.cache import cache_root, invoke, locked
from tools.build.receipts import run_check
from tools.build.worker import replace_file, sync_files, worker_snapshot


def execute(command, root, env=None):
    status = invoke(command, cwd=root, env=env)
    if status:
        raise subprocess.CalledProcessError(status, command)


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root)


def changed_paths(root):
    data = git(root, "diff", "--cached", "--name-only", "--no-renames", "-z")
    return [os.fsdecode(name) for name in data.split(b"\0") if name]


def tree_files(root, tree):
    entries = []
    for record in git(root, "ls-tree", "-rz", "--full-tree", tree).split(b"\0"):
        if not record:
            continue
        header, path = record.split(b"\t", 1)
        mode, kind, identity = header.split()
        if kind != b"blob":
            raise RuntimeError(
                "Submodule validation requires a separate checkout"
            )
        entries.append((os.fsdecode(path), mode, identity))
    blobs = subprocess.check_output(
        ["git", "cat-file", "--batch"],
        cwd=root,
        input=b"".join(identity + b"\n" for _, _, identity in entries),
    )
    stream = io.BytesIO(blobs)
    files = {}
    for path, mode, _ in entries:
        _, kind, size = stream.readline().split()
        if kind != b"blob":
            raise RuntimeError("Snapshot blob missing")
        files[path] = (mode, stream.read(int(size)))
        if stream.read(1) != b"\n":
            raise RuntimeError("Snapshot blob truncated")
    return files


@contextlib.contextmanager
def snapshot(root, tree):
    env = dict(os.environ)
    for name in git(root, "rev-parse", "--local-env-vars").decode().split():
        env.pop(name, None)
    # Hooks see Git helpers first; shells do not.
    helpers = env.pop("GIT_EXEC_PATH", None)
    env["PATH"] = os.pathsep.join(
        part
        for part in env.get("PATH", "").split(os.pathsep)
        if part != helpers
    )
    env["VPDF_CACHE"] = "off"
    kind = git(root, "cat-file", "-t", tree).strip()
    revision = tree if kind == b"commit" else None
    with worker_snapshot(
        root, lambda: tree_files(root, tree), env, revision
    ) as state:
        yield state


def format_commands(paths):
    commands = []
    groups = {}
    for name in paths:
        groups.setdefault(Path(name).suffix, []).append("./" + name)
    for suffix, names in groups.items():
        if suffix == ".rs":
            commands.append(
                ["rustfmt", "--unstable-features", "--skip-children", *names]
            )
        elif suffix == ".py":
            commands.extend(
                [
                    ["ruff", "check", "--fix", *names],
                    ["ruff", "format", *names],
                    ["ruff", "check", *names],
                ]
            )
        elif suffix == ".toml":
            commands.append(["taplo", "fmt", *names])
        elif suffix == ".md":
            commands.append(["rumdl", "check", "--fix", *names])
    return commands


def format_staged(root, source, paths, env):
    originals = {}
    for name in paths:
        path = source / name
        if path.is_file() and not path.is_symlink():
            originals[name] = path.read_bytes()
    commands = format_commands(originals)
    passed = True
    try:
        for command in commands:
            execute(command, source, env)
    except subprocess.CalledProcessError:
        passed = False
    for name, original in originals.items():
        formatted = (source / name).read_bytes()
        path = root / name
        if formatted != original:
            passed = False
            if (
                path.is_file()
                and not path.is_symlink()
                and path.resolve() == root.resolve() / name
                and path.read_bytes() == original
                and (path.stat().st_mode & 0o111)
                == ((source / name).stat().st_mode & 0o111)
            ):
                replace_file(path, formatted)
                print(f"Restage formatted file: {name}", flush=True)
            else:
                print(f"Staged formatting differs: {name}", flush=True)
                for command in format_commands([name]):
                    print(
                        shlex.join(["mise", "exec", "--", *command]), flush=True
                    )
    rust = ["./" + name for name in originals if name.endswith(".rs")]
    if rust:
        try:
            execute(["sh", "tools/check/width.sh", *rust], source, env)
        except subprocess.CalledProcessError:
            passed = False
    return passed


PYTHON_TASKS = {
    "tools/bench/startup": "test:startup",
    "tools/demo/capture": "test:demo",
    "tools/demo/media": "test:demo",
    "tools/demo/video": "test:demo",
    "tools/demo/window": "test:demo",
    "tools/check/packages": "test:packages",
    "tools/build/cache": "test:build-cache",
    "tools/build/cleanup": "test:build-cache",
    "tools/build/worker": "test:build-worker",
    "tools/build/features": "test:build-features",
    "tools/check/ci": "test:ci",
    "tools/check/merge": "test:hooks",
}
PACKAGING = {
    "tools/package/app.sh",
    "tools/package/install-app.sh",
    "tools/package/replace_app.py",
    "tools/package/test_app.py",
    "tools/package/macos-minimum.sh",
}
GLOBAL_FILES = {
    "Cargo.lock",
    "prek.toml",
    "rustfmt.toml",
    "ruff.toml",
    "taplo.toml",
    ".config/mise.toml",
    ".config/rumdl.toml",
    "tools/check/hooks.py",
    "tools/check/test_hooks.py",
    "tools/build/cache.py",
    "tools/build/worker.py",
    "tools/build/receipts.py",
    "tools/build/test_receipts.py",
    "tools/shims/cargo",
    "tools/shims/native/make",
    "tools/check/packages.py",
}


FULL_TEST_TASKS = {
    "test:startup",
    "test:demo",
    "test:packages",
    "test:bench-report",
    "test:build-features",
    "test:build-cache",
    "test:build-worker",
    "test:validation",
    "test:ci",
}


def python_task(name):
    if name.startswith("tools/bench/live/") and name.endswith(".py"):
        return "test:bench-report"
    if name == "tools/demo/optimize.sh":
        return "test:demo"
    path = Path(name)
    if path.suffix != ".py":
        return None
    module = path.parent / path.stem.removeprefix("test_")
    return PYTHON_TASKS.get(module.as_posix())


def prose_path(name):
    return name in {"README.md", "AGENTS.md"} or (
        name.startswith("docs/") and name.endswith(".md")
    )


def select_tests(paths, packages):
    owners, tasks, reasons = set(), set(), []
    folders = sorted(packages, key=len, reverse=True)
    for name in paths:
        if name.startswith("docs/assets/"):
            tasks.add("test:help")
        if (
            name.startswith(("docs/users/", "docs/licenses/", "tools/help/"))
            or name == "tools/build/help.rs"
        ):
            tasks.add("test:help")
        elif name in PACKAGING:
            tasks.add("test:app")
            reasons.append(name)
        elif (
            name in GLOBAL_FILES
            or Path(name).name == "Cargo.toml"
            or name.startswith(
                ("tests/support/", "crates/vpdf-dev/support/", ".cargo/")
            )
            or Path(name).name.startswith(("rust-toolchain", ".mise"))
        ):
            reasons.append(name)
        elif task := python_task(name):
            tasks.add(task)
        elif name.startswith("tools/build/") and name.endswith(".rs"):
            owners.add(packages.get(".", "vpdf"))
        elif name == "docs/assets/vpdf.png":
            owners.add("vpdf-gui")
        elif prose_path(name):
            continue
        else:
            owner = next(
                (
                    packages[folder]
                    for folder in folders
                    if name.startswith(folder + "/")
                ),
                None,
            )
            if owner:
                owners.add(owner)
            elif name.startswith(("src/", "tests/it/")):
                owners.add(packages.get(".", "vpdf"))
            else:
                reasons.append(name)
    return owners, tasks, reasons


def package_folders(source, env):
    data = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=source,
        env=env,
    )
    metadata = json.loads(data)
    return {
        str(Path(package["manifest_path"]).parent.relative_to(source)): package[
            "name"
        ]
        for package in metadata["packages"]
        if package["id"] in metadata["workspace_members"]
    }


def affected_tests(source, paths, env):
    packages = {}
    metadata_failed = False
    if any(
        not python_task(name)
        and name.startswith(("crates/", "src/", "tests/it/"))
        for name in paths
    ):
        try:
            packages = package_folders(source, env)
        except (subprocess.CalledProcessError, ValueError, KeyError):
            metadata_failed = True
    owners, tasks, reasons = select_tests(paths, packages)
    if metadata_failed:
        reasons.append("Package metadata unavailable")
    if reasons:
        print("Tests: full; changed shared or unknown inputs", flush=True)
        for name in sorted(reasons):
            print(f"  {name}", flush=True)
        run_check(
            source,
            env,
            "test",
            lambda: execute(["mise", "run", "test"], source, env),
        )
        tasks.difference_update(FULL_TEST_TASKS)
    for task in sorted(tasks):
        print(f"Tests: {task}", flush=True)
        if task in FULL_TEST_TASKS:
            run_check(
                source,
                env,
                task,
                lambda task=task: execute(["mise", "run", task], source, env),
                covered_by="test",
            )
        else:
            execute(["mise", "run", task], source, env)
    # It has no tests; nextest fails empty selections.
    owners.discard("vpdf-dev")
    if owners and not reasons:
        print("Tests: affected " + ", ".join(sorted(owners)), flush=True)
        expression = " | ".join(f"rdeps(={name})" for name in sorted(owners))
        execute(["mise", "run", "help:build"], source, env)
        command = [
            "cargo",
            "nextest",
            "run",
            "--locked",
            "--workspace",
            "--test-threads",
            env.get("NEXTEST_TEST_THREADS", "4"),
            "--no-fail-fast",
            "--no-tests=fail",
            "-E",
            expression,
        ]
        run_check(
            source,
            env,
            "affected:" + expression,
            lambda: execute(command, source, env),
            covered_by="test",
        )
    if not owners and not tasks and not reasons:
        print("Tests: no affected code", flush=True)


def pre_commit(root):
    paths = changed_paths(root)
    if not paths:
        print("No staged changes", flush=True)
        return 0
    tree = git(root, "write-tree").decode().strip()
    files = tree_files(root, tree)
    env = dict(os.environ)
    with tempfile.TemporaryDirectory(prefix="vpdf-staged-") as directory:
        source = Path(directory).resolve()
        sync_files(source, files)
        if not format_staged(root, source, paths, env):
            return 1
        if all(
            prose_path(name) and not name.startswith("docs/users/")
            for name in paths
        ):
            print("Tests: no affected code", flush=True)
            return 0
    with snapshot(root, tree) as (source, env):
        affected_tests(source, paths, env)
    return 0


def check_lint(root, env):
    commands = [
        ["python3", "-B", "-m", "tools.help.assets"],
        ["python3", "-B", "-m", "tools.check.packages"],
        ["cargo", "fmt", "--all", "--check"],
        ["taplo", "fmt", "--check"],
        ["rumdl", "check", "."],
        ["ruff", "check", "."],
        ["ruff", "format", "--check", "."],
        [
            "cargo",
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        ["sh", "tools/check/width.sh"],
    ]
    for command in commands:
        execute(command, root, env)


def full_checks(root, env):
    run_check(root, env, "lint", lambda: check_lint(root, env))
    run_check(
        root,
        env,
        "test",
        lambda: execute(["mise", "run", "test"], root, env),
    )


def check_path(root, commit, suffix):
    checks = cache_root(os.environ, root) / "checks"
    checks.mkdir(mode=0o700, parents=True, exist_ok=True)
    return checks / f"{commit}.{suffix}"


def verify(root, commit):
    # A second check of one commit waits, then reuses receipts.
    with locked(check_path(root, commit, "lock")):
        print(f"Push commit: {commit}", flush=True)
        try:
            with snapshot(root, commit) as (source, env):
                execute(["mise", "run", "ci:check"], source, env)
        except subprocess.CalledProcessError:
            print(f"Push check failed: {commit}", flush=True)
            raise
        print(f"Push check passed: {commit}", flush=True)


def pre_push(root, stream):
    commits = set()
    for line in stream:
        fields = line.split()
        if len(fields) != 4:
            raise RuntimeError("Invalid push reference")
        name, local, _, _ = fields
        if local and set(local) == {"0"}:
            continue
        commit = (
            git(root, "rev-parse", "--verify", local + "^{commit}")
            .decode()
            .strip()
        )
        commits.add(commit)
        print(f"Push validation: {name}", flush=True)
    for commit in sorted(commits):
        verify(root, commit)
    return 0


def hook_shim(stage):
    return f"""#!/bin/sh
# Vpdf validation hook.
set -eu
root=$(git rev-parse --show-toplevel)
if [ -f "$root/tools/check/hooks.py" ]; then
  cd "$root"
  exec mise exec -- python3 -B -m tools.check.hooks {stage} "$@"
fi
if [ -f "$root/scripts/hooks.py" ]; then
  exec mise exec -- python3 -B "$root/scripts/hooks.py" {stage} "$@"
fi
exec mise exec -- prek hook-impl --hook-dir "$(dirname "$0")" \\
  --script-version 4 --hook-type {stage} -- "$@"
"""


def install_hooks(root):
    folder = Path(
        os.fsdecode(git(root, "rev-parse", "--git-path", "hooks").strip())
    )
    if not folder.is_absolute():
        folder = root / folder
    for stage in ("pre-commit", "pre-push"):
        path = folder / stage
        if path.exists():
            content = path.read_text()
            if (
                "# Vpdf validation hook." not in content
                and "File generated by prek:" not in content
            ):
                raise RuntimeError(f"Existing hook requires migration: {stage}")
    folder.mkdir(parents=True, exist_ok=True)
    for stage in ("pre-commit", "pre-push"):
        path = folder / stage
        content = hook_shim(stage).encode()
        if path.exists():
            replace_file(path, content)
        else:
            path.write_bytes(content)
        path.chmod(0o755)
        print(f"Installed {stage}", flush=True)


def main():
    parser = argparse.ArgumentParser(
        description="Validate staged and pushed snapshots"
    )
    parser.add_argument(
        "stage",
        choices=[
            "install",
            "pre-commit",
            "pre-push",
            "verify",
            "check",
            "lint",
            "test",
            "validate",
        ],
    )
    parser.add_argument("args", nargs="*")
    args = parser.parse_args()
    root = Path(
        os.fsdecode(git(Path.cwd(), "rev-parse", "--show-toplevel").strip())
    )
    if args.stage == "install":
        install_hooks(root)
    elif args.stage == "pre-commit":
        return pre_commit(root)
    elif args.stage == "pre-push":
        return pre_push(root, sys.stdin)
    elif args.stage == "verify":
        for name in args.args or ["HEAD"]:
            commit = git(root, "rev-parse", "--verify", name + "^{commit}")
            verify(root, commit.decode().strip())
    elif args.stage == "lint":
        check_lint(root, os.environ.copy())
    elif args.stage == "test":
        run_check(
            root,
            os.environ.copy(),
            "test",
            lambda: execute(
                ["mise", "run", "_test:full"], root, os.environ.copy()
            ),
        )
    elif args.stage == "validate":
        env = os.environ.copy()
        full_checks(root, env)
        started = time.monotonic()
        execute(["mise", "run", "build:debug"], root, env)
        print(
            f"Validation build: {time.monotonic() - started:.3f}s", flush=True
        )
    else:
        execute(["mise", "run", "ci:check"], root, os.environ.copy())
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
