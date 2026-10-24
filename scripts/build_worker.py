"""Serialize builds in a stable repository snapshot."""

import contextlib
import fcntl
import hashlib
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from build_cache import (
    DEFAULT_GIB,
    cache_key,
    cache_root,
    cargo_env,
    invoke,
    locked,
    prune,
)


def replace_file(path, content, mode=None):
    if mode is None:
        mode = stat.S_IMODE(path.stat().st_mode)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(content)
            stream.flush()
            temporary.chmod(mode)
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def sync_files(source, files):
    source.mkdir(parents=True, exist_ok=True)
    for directory, dirs, names in os.walk(source, topdown=False):
        for name in names + dirs:
            path = Path(directory) / name
            relative = path.relative_to(source).as_posix()
            if relative == ".git" or relative.startswith(".git/"):
                continue
            if path.is_symlink() or path.is_file():
                if relative not in files or path.is_symlink():
                    path.unlink()
            elif not any(path.iterdir()):
                path.rmdir()
    for name, (mode, content) in files.items():
        path = source / name
        if not path.is_relative_to(source) or ".." in Path(name).parts:
            raise RuntimeError("Snapshot path escapes source")
        if path.is_dir():
            shutil.rmtree(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        if mode == b"120000":
            target = (path.parent / os.fsdecode(content)).resolve()
            if not target.is_relative_to(source):
                raise RuntimeError("Snapshot symlink escapes source")
            path.unlink(missing_ok=True)
            path.symlink_to(os.fsdecode(content))
        else:
            permissions = 0o755 if mode == b"100755" else 0o644
            if not path.exists() or path.read_bytes() != content:
                replace_file(path, content, permissions)
            else:
                path.chmod(permissions)


def working_files(root, env=None):
    names = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=root,
        env=env,
    )
    files = {}
    for name in names.split(b"\0"):
        if not name:
            continue
        path = root / os.fsdecode(name)
        if path.is_symlink():
            files[os.fsdecode(name)] = (
                b"120000",
                os.fsencode(os.readlink(path)),
            )
        elif path.is_file():
            if not path.resolve().is_relative_to(root):
                raise RuntimeError("Source path escapes worktree")
            mode = b"100755" if path.stat().st_mode & 0o111 else b"100644"
            files[os.fsdecode(name)] = (mode, path.read_bytes())
        elif path.is_dir() and (path / ".git").exists():
            raise RuntimeError("Submodule builds require a separate checkout")
    return files


def snapshot_git(root, source, revision, descriptor):
    env = dict(os.environ)
    names = (
        subprocess.check_output(
            ["git", "rev-parse", "--local-env-vars"],
            cwd=root,
            pass_fds=(descriptor,),
        )
        .decode()
        .split()
    )
    for name in names:
        env.pop(name, None)

    def git(*args, directory=root):
        return subprocess.check_output(
            ["git", *args], cwd=directory, env=env, pass_fds=(descriptor,)
        ).strip()

    metadata = source / ".git"
    if not metadata.exists():
        git("init", "-q", "--template=", str(source))
        git("config", "core.hooksPath", os.devnull, directory=source)
    (metadata / "index.lock").unlink(missing_ok=True)
    exclude = metadata / "info/exclude"
    if not exclude.exists():
        exclude.parent.mkdir(parents=True, exist_ok=True)
        replace_file(exclude, b"/tmp\n", 0o644)
    objects = git(
        "rev-parse", "--path-format=absolute", "--git-path", "objects"
    )
    alternates = metadata / "objects/info/alternates"
    content = objects + b"\n"
    if not alternates.exists() or alternates.read_bytes() != content:
        alternates.parent.mkdir(parents=True, exist_ok=True)
        replace_file(alternates, content, 0o644)
    if revision is None:
        result = subprocess.run(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=root,
            env=env,
            pass_fds=(descriptor,),
            capture_output=True,
            check=False,
        )
        revision = result.stdout.strip() if result.returncode == 0 else None
    if revision:
        content = os.fsencode(revision) + b"\n"
        if (metadata / "HEAD").read_bytes() != content:
            replace_file(metadata / "HEAD", content, 0o644)
        git("read-tree", os.fsdecode(revision), directory=source)
    else:
        content = b"ref: refs/heads/worker-unborn\n"
        if (metadata / "HEAD").read_bytes() != content:
            replace_file(metadata / "HEAD", content, 0o644)
        git("read-tree", "--empty", directory=source)
    tags = {}
    for line in git(
        "for-each-ref", "--format=%(objectname) %(refname)", "refs/tags"
    ).splitlines():
        identity, name = line.split(b" ", 1)
        tags[os.fsdecode(name.removeprefix(b"refs/tags/"))] = (
            b"100644",
            identity + b"\n",
        )
    sync_files(metadata / "refs/tags", tags)


@contextlib.contextmanager
def worker_snapshot(root, files, env=None, revision=None):
    env = cargo_env(os.environ if env is None else env)
    env["VPDF_BUILD_CACHE_ID"] = cache_key(root)
    cache = cache_root(env, root)
    build = cache / "build"
    worker = build / "worker"
    source = worker / "source"
    limit = float(env.get("VPDF_BUILD_CACHE_GIB", DEFAULT_GIB)) * 1024**3
    if not 0 < limit < float("inf"):
        raise ValueError("Build cache budget must be positive")
    leases = cache / "leases"
    leases.mkdir(mode=0o700, parents=True, exist_ok=True)
    captured = files()
    if revision is None:
        head = subprocess.run(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=root,
            capture_output=True,
            check=False,
        )
        revision = head.stdout.strip() if head.returncode == 0 else ""
    queued = time.monotonic()
    try:
        with locked(leases / "worker") as lease:
            acquired = time.monotonic()
            print(f"Worker queue: {acquired - queued:.3f}s", flush=True)
            with locked(cache / "lock"):
                prune(build, limit, leases)
            worker.mkdir(parents=True, exist_ok=True)
            os.utime(worker, None)
            sync_files(source, captured)
            snapshot_git(root, source, revision, lease.fileno())
            env.update(
                CARGO_BUILD_BUILD_DIR=str(worker / "intermediates"),
                CARGO_TARGET_DIR=str(worker / "target"),
                VPDF_BUILD_CACHE_ID=cache.name,
                VPDF_BUILD_WORKER=str(source),
                VPDF_BUILD_WORKER_FD=str(lease.fileno()),
                MISE_TRUSTED_CONFIG_PATHS=str(source),
                PYTHONDONTWRITEBYTECODE="1",
            )
            env["PATH"] = (
                str(source / "scripts/tools/native") + os.pathsep + env["PATH"]
            )
            if env.get("VPDF_CORPUS") == str(root / "tmp/corpus"):
                env["VPDF_CORPUS"] = str(source / "tmp/corpus")
            env.setdefault("CARGO_BUILD_JOBS", "4")
            env.setdefault("NEXTEST_TEST_THREADS", "4")
            # Reuse corpus inputs without copying PDFs.
            (root / "tmp").mkdir(exist_ok=True)
            (source / "tmp").symlink_to(root / "tmp", target_is_directory=True)
            ready = time.monotonic()
            print(f"Worker sync: {ready - acquired:.3f}s", flush=True)
            try:
                yield source, env
            finally:
                print(
                    f"Worker task: {time.monotonic() - ready:.3f}s", flush=True
                )
                os.utime(worker, None)
    finally:
        with locked(cache / "lock"):
            prune(build, limit, leases)


def copy_binary(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    replace_file(
        destination, source.read_bytes(), source.stat().st_mode & 0o777
    )


def copy_outputs(task, root, target, env):
    output = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    if not output.is_absolute():
        output = root / output
    if task == "validate":
        task = "build:debug"
    if task == "help:build" or (
        task in {"build", "build:debug", "build:gui", "build:app", "release"}
        and env.get("VPDF_BUILD_WORKER")
    ):
        copy_help(output, target, env)
    if task == "build":
        binary = Path("release/vpdf")
        copy_binary(target / binary, output / binary)
    elif task in {"build:debug", "build:gui"}:
        name = "vpdf-gui" if task == "build:gui" else "vpdf"
        triple = env.get("CARGO_BUILD_TARGET")
        binary = Path(triple or "") / "debug" / name
        copy_binary(target / binary, output / binary)
        if triple:
            copy_binary(target / binary, output / "debug" / name)
    elif task == "build:app":
        from replace_app import replace

        destination = output / "dist"
        destination.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=destination) as temporary:
            app = Path(temporary) / "Vpdf.app"
            shutil.copytree(target / "dist/Vpdf.app", app, symlinks=True)
            replace(app, destination / "Vpdf.app")
        copy_binary(target / "dist/Vpdf.dmg", destination / "Vpdf.dmg")
        print(f"App: {destination / 'Vpdf.app'}")
        print(f"DMG: {destination / 'Vpdf.dmg'}")
    elif task == "release":
        for binary in [Path("release/vpdf"), Path("dist/vpdf")]:
            copy_binary(target / binary, output / binary)
        print(f"Release executable: {output / 'dist/vpdf'}")


def copy_help(output, target, env):
    from help_assets import OUTPUTS

    directory = Path(
        subprocess.check_output(
            [sys.executable, "-B", "scripts/help_assets.py", "--check"],
            cwd=target.parent / "source",
            env=env,
            pass_fds=(int(env["VPDF_BUILD_WORKER_FD"]),),
            text=True,
        ).strip()
    )
    destination = output / "help" / directory.name
    destination.mkdir(parents=True, exist_ok=True)
    for name in (*OUTPUTS, "manifest.json"):
        copy_binary(directory / name, destination / name)


def run(args):
    if not args:
        raise ValueError("Shared build task missing")
    task = args[0]
    if task not in {
        "build",
        "validate",
        "release",
        "build:debug",
        "build:app",
        "build:gui",
        "help:build",
        "help:check",
        "hook:check",
        "check",
        "lint:check",
        "lint:rust",
        "test",
    } and not task.startswith("test:"):
        raise ValueError("Shared task must build or validate")
    root = Path.cwd().resolve()
    command = ["mise", "run", f"_{task}", "--", *args[1:]]
    outputs = not set(args[1:]).intersection(
        {"--help", "-h", "--version", "-V"}
    )
    custom = any(
        arg.split("=", 1)[0]
        in {
            "--manifest-path",
            "--build-dir",
            "--target-dir",
            "--target",
            "--profile",
            "--release",
            "-r",
            "--lib",
            "--bin",
            "--bins",
            "--example",
            "--examples",
            "--test",
            "--tests",
            "--bench",
            "--benches",
            "--all-targets",
        }
        for arg in args[1:]
    )
    if (
        os.environ.get("CI") == "true"
        or os.environ.get("VPDF_BUILD_MODE") == "isolated"
        or os.environ.get("CARGO_BUILD_BUILD_DIR")
        or custom
        or os.environ.get("VPDF_BUILD_WORKER") == str(root)
    ):
        status = invoke(command, env=dict(os.environ), cwd=root)
        if (
            status == 0
            and outputs
            and not custom
            and task in {"build:debug", "build:gui"}
            and os.environ.get("VPDF_BUILD_WORKER") != str(root)
        ):
            target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
            if not target.is_absolute():
                target = root / target
            copy_outputs(task, root, target, os.environ)
        return status
    with worker_snapshot(root, lambda: working_files(root)) as (source, env):
        status = invoke(command, env=env, cwd=source)
        if status == 0 and outputs:
            copy_outputs(task, root, Path(env["CARGO_TARGET_DIR"]), env)
        return status


def clean_old(root):
    env = dict(os.environ)
    env["VPDF_BUILD_CACHE_ID"] = cache_key(root)
    cache = cache_root(env, root)
    build = cache / "build"
    leases = cache / "leases"
    leases.mkdir(mode=0o700, parents=True, exist_ok=True)
    with locked(cache / "lock"):
        for entry in build.glob("*"):
            if len(entry.name) != 16 or any(
                c not in "0123456789abcdef" for c in entry.name
            ):
                continue
            try:
                with locked(leases / entry.name, fcntl.LOCK_EX | fcntl.LOCK_NB):
                    if entry.is_symlink():
                        raise ValueError("Build cache entry is a symlink")
                    shutil.rmtree(entry)
            except BlockingIOError:
                continue
    worktrees = subprocess.check_output(
        ["git", "worktree", "list", "--porcelain", "-z"], cwd=root
    )
    for record in worktrees.split(b"\0"):
        if not record.startswith(b"worktree "):
            continue
        identity = subprocess.check_output(
            ["git", "rev-parse", "--absolute-git-dir"],
            cwd=os.fsdecode(record[9:]),
        ).strip()
        key = hashlib.sha256(identity).hexdigest()[:16]
        legacy = Path(tempfile.gettempdir()) / f"vpdf-hooks-{os.getuid()}" / key
        if legacy.is_dir():
            try:
                with locked(legacy / "lock", fcntl.LOCK_EX | fcntl.LOCK_NB):
                    shutil.rmtree(legacy / "source", ignore_errors=True)
            except BlockingIOError:
                continue
    return 0


def native_make(args):
    env = dict(os.environ)
    native = Path(__file__).resolve().parent / "tools/native"
    env["PATH"] = os.pathsep.join(
        part
        for part in env["PATH"].split(os.pathsep)
        if Path(part).resolve() != native
    )
    make = shutil.which("make", path=env["PATH"])
    if not make:
        raise RuntimeError("Make executable missing")
    value = env.get("CARGO_BUILD_JOBS", "4")
    jobs = (os.cpu_count() or 1) if value == "default" else int(value)
    if jobs < 0:
        jobs = max(1, (os.cpu_count() or 1) + jobs)
    if jobs == 0:
        raise ValueError("Build job limit must be positive")
    return invoke([make, *args, f"-j{jobs}"], env=env)


def main():
    try:
        if sys.argv[1:] == ["--clean-old"]:
            status = clean_old(Path.cwd())
        elif sys.argv[1:2] == ["--make"]:
            status = native_make(sys.argv[2:])
        else:
            status = run(sys.argv[1:])
        return status if status >= 0 else 128 - status
    except (
        OSError,
        ValueError,
        RuntimeError,
        subprocess.CalledProcessError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
