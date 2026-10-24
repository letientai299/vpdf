"""Bound isolated worktree caches with one budget."""

import contextlib
import fcntl
import hashlib
import os
import shutil
import signal
import subprocess
import sys
from pathlib import Path

DEFAULT_GIB = 20
SHIMS = Path(__file__).resolve().parent / "tools"


def cache_key(root):
    result = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=root,
        capture_output=True,
        check=False,
    )
    identity = (
        result.stdout.strip()
        if result.returncode == 0
        else os.fsencode(root.resolve())
    )
    return hashlib.sha256(identity).hexdigest()[:16]


def cache_root(env, source=None):
    key = env.get("VPDF_BUILD_CACHE_ID") or cache_key(source or Path.cwd())
    if len(key) != 16 or any(c not in "0123456789abcdef" for c in key):
        raise ValueError("Invalid build cache identity")
    base = env.get("XDG_CACHE_HOME")
    if not base:
        base = Path.home() / (
            "Library/Caches" if sys.platform == "darwin" else ".cache"
        )
    return Path(base).resolve() / "vpdf-build" / key


def cache_bytes(path):
    if path.is_symlink():
        raise ValueError("Build cache directory is a symlink")
    if not path.exists():
        return 0
    result = subprocess.run(
        ["du", "-sk", str(path)], capture_output=True, text=True
    )
    # Active compilers rename files during scans.
    if not result.stdout:
        if not path.exists():
            return 0
        result.check_returncode()
    return int(result.stdout.split()[0]) * 1024


@contextlib.contextmanager
def locked(path, mode=fcntl.LOCK_EX):
    with path.open("a") as lock:
        fcntl.flock(lock, mode)
        yield lock


def prune(path, limit, leases=None):
    if cache_bytes(path) <= limit:
        return
    print(
        "Vpdf build cache exceeded budget; pruning intermediates",
        file=sys.stderr,
    )
    entries = []
    for entry in path.iterdir():
        try:
            entries.append((entry.lstat().st_mtime_ns, entry))
        except FileNotFoundError:
            continue
    for _, entry in sorted(entries):
        if entry.is_symlink():
            raise ValueError("Build cache entry is a symlink")
        with contextlib.ExitStack() as stack:
            if leases:
                try:
                    stack.enter_context(
                        locked(
                            leases / entry.name, fcntl.LOCK_EX | fcntl.LOCK_NB
                        )
                    )
                except BlockingIOError:
                    continue
            if not entry.exists():
                continue
            if entry.is_dir():
                shutil.rmtree(entry)
            else:
                entry.unlink()
        if cache_bytes(path) <= limit:
            break


def source_root(args):
    directory = Path.cwd()
    for index, arg in enumerate(args):
        if arg == "-C" and index + 1 < len(args):
            directory = (directory / args[index + 1]).resolve()
        elif arg.startswith("-C") and len(arg) > 2:
            directory = (directory / arg[2:]).resolve()
    for index, arg in enumerate(args):
        if arg.startswith("--manifest-path="):
            return (directory / arg.split("=", 1)[1]).resolve().parent
        if arg == "--manifest-path" and index + 1 < len(args):
            return (directory / args[index + 1]).resolve().parent
    return directory.resolve()


def cargo_env(env):
    env = env.copy()
    # Child Cargo calls retain our lease.
    env["PATH"] = os.pathsep.join(
        part
        for part in env.get("PATH", "").split(os.pathsep)
        if Path(part).resolve() != SHIMS
    )
    return env


def invoke(command, env, pass_fds=(), cwd=None):
    if env and env.get("VPDF_BUILD_WORKER_FD"):
        descriptor = int(env["VPDF_BUILD_WORKER_FD"])
        lease = Path(env["VPDF_BUILD_WORKER"]).parents[2] / "leases/worker"
        try:
            inherited = os.fstat(descriptor)
            expected = lease.stat()
            if (inherited.st_dev, inherited.st_ino) == (
                expected.st_dev,
                expected.st_ino,
            ):
                pass_fds = (*pass_fds, descriptor)
        except OSError:
            pass
    with subprocess.Popen(
        command, env=env, pass_fds=pass_fds, cwd=cwd, start_new_session=True
    ) as child:
        previous = {}

        def forward(signum, _frame):
            try:
                os.killpg(child.pid, signum)
            except ProcessLookupError:
                pass

        try:
            for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
                previous[signum] = signal.signal(signum, forward)
            return child.wait()
        finally:
            for signum, handler in previous.items():
                signal.signal(signum, handler)


def run(args):
    env = cargo_env(os.environ)
    cargo = shutil.which("cargo", path=env["PATH"])
    if not cargo:
        raise RuntimeError("Cargo executable missing")
    options = args[: args.index("--")] if "--" in args else args
    # Explicit build directories remain caller-owned.
    if (
        env.get("CI") == "true"
        or env.get("CARGO_BUILD_BUILD_DIR")
        or any(
            arg == "--build-dir" or arg.startswith("--build-dir=")
            for arg in options
        )
    ):
        return invoke([cargo, *args], env=env)
    source = source_root(options)
    root = cache_root(env, source)
    build = root / "build"
    namespace = hashlib.sha256(os.fsencode(source)).hexdigest()[:16]
    intermediates = build / namespace
    env["CARGO_BUILD_BUILD_DIR"] = str(intermediates)
    env["VPDF_BUILD_CACHE_ID"] = root.name
    command = next((arg for arg in options if not arg.startswith("+")), "")
    if command in {"metadata", "fmt", "locate-project", "help"} or any(
        arg in {"--version", "-V", "--help", "-h", "--list"} for arg in options
    ):
        return invoke([cargo, *args], env=env)
    limit = float(env.get("VPDF_BUILD_CACHE_GIB", DEFAULT_GIB)) * 1024**3
    if not 0 < limit < float("inf"):
        raise ValueError("Build cache budget must be positive")
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    leases = root / "leases"
    leases.mkdir(exist_ok=True)
    try:
        with locked(leases / namespace, fcntl.LOCK_SH) as lease:
            with locked(root / "lock"):
                prune(build, limit, leases)
            intermediates.mkdir(parents=True, exist_ok=True)
            os.utime(intermediates, None)
            try:
                return invoke(
                    [cargo, *args], env=env, pass_fds=(lease.fileno(),)
                )
            finally:
                try:
                    os.utime(intermediates, None)
                except FileNotFoundError:
                    pass
    finally:
        with locked(root / "lock"):
            prune(build, limit, leases)


def main():
    try:
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
