"""Bound isolated worktree caches with one budget."""

import concurrent.futures
import contextlib
import fcntl
import hashlib
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

DEFAULT_GIB = 20
WORKER_PROTOCOL = 3
MAX_WORKERS = 8
DEFAULT_WORKERS = 4
# Per worker; also caps MuPDF's Make.
WORKER_JOBS = 16
# Recent measurements with headroom skip rescans.
USAGE_SECONDS = 300
USAGE_SHARE = 0.75
SHIMS = Path(__file__).resolve().parents[1] / "shims"


def cache_identity(root):
    result = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=root,
        capture_output=True,
        check=False,
    )
    if result.returncode == 0:
        return result.stdout.strip()
    return os.fsencode(root.resolve())


def cache_key(root):
    return hashlib.sha256(cache_identity(root)).hexdigest()[:16]


def mark_origin(cache, root):
    # Cleanup removes caches whose repository is gone.
    marker = cache / "origin"
    identity = cache_identity(root)
    if not marker.exists() or marker.read_bytes() != identity:
        marker.write_bytes(identity)


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


def pool_size(env):
    count = int(env.get("VPDF_BUILD_WORKERS", DEFAULT_WORKERS))
    if not 1 <= count <= MAX_WORKERS:
        raise ValueError(f"Worker count must be between 1 and {MAX_WORKERS}")
    return count


def cache_limit(env):
    gib = env.get("VPDF_BUILD_CACHE_GIB")
    limit = (float(gib) if gib else DEFAULT_GIB * pool_size(env)) * 1024**3
    if not 0 < limit < float("inf"):
        raise ValueError("Build cache budget must be positive")
    return limit


def cache_bytes(path):
    if path.is_symlink():
        raise ValueError("Build cache directory is a symlink")
    if not path.exists():
        return 0
    # Its own session keeps Ctrl-C for the task.
    result = subprocess.run(
        ["du", "-sk", str(path)],
        capture_output=True,
        text=True,
        start_new_session=True,
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


def usage_record(path):
    return path.parent / "usage"


def read_usage(record):
    try:
        return int(record.read_text()), record.stat().st_mtime
    except (OSError, ValueError):
        return None


def write_usage(record, size, measured=None):
    staged = record.with_name(f".usage.{os.getpid()}")
    # The record only skips scans; never fail builds.
    with contextlib.suppress(OSError):
        try:
            staged.write_text(f"{size}\n")
            if measured is not None:
                os.utime(staged, (measured, measured))
            os.replace(staged, record)
        finally:
            staged.unlink(missing_ok=True)


def prune(path, limit, leases=None):
    measured = size = cache_bytes(path)
    if size > limit:
        size = evict(path, size, limit, leases)
    write_usage(usage_record(path), size)
    return measured - size


def prune_recorded(path, limit, leases=None):
    usage = read_usage(usage_record(path))
    if (
        usage
        and 0 <= time.time() - usage[1] < USAGE_SECONDS
        and usage[0] <= limit * USAGE_SHARE
    ):
        return 0
    return prune(path, limit, leases)


def add_growth(grown):
    def check(path, limit, leases):
        # Estimates keep the last measurement's age.
        record = usage_record(path)
        usage = read_usage(record)
        if usage is None or usage[0] + grown > limit * USAGE_SHARE:
            return prune(path, limit, leases)
        write_usage(record, usage[0] + grown, usage[1])
        return 0

    return check


def locked_prune(cache, limit, leases, check):
    start = time.monotonic()
    with locked(cache / "lock"):
        freed = check(cache / "build", limit, leases)
    return freed, time.monotonic() - start


@contextlib.contextmanager
def budget(cache, limit, leases, log=None):
    """Yield a scan starter; account for the task's growth after."""
    free = shutil.disk_usage(cache).free
    start = time.monotonic()
    failed = False
    with concurrent.futures.ThreadPoolExecutor(1) as pool:
        scans = []

        def begin():
            # Callers start it once their lease protects them.
            scans.append(
                pool.submit(locked_prune, cache, limit, leases, prune_recorded)
            )

        try:
            yield begin
        except BaseException:
            failed = True
            raise
        finally:
            try:
                freed = 0
                for scan in scans:
                    freed, elapsed = scan.result()
                    if log:
                        log(f"scan: {elapsed:.3f}s")
                # Volume growth, including other processes' writes.
                grown = max(0, free + freed - shutil.disk_usage(cache).free)
                stale = freed or time.monotonic() - start > USAGE_SECONDS
                check = prune if stale else add_growth(grown)
                _, elapsed = locked_prune(cache, limit, leases, check)
                if log:
                    log(f"settle: {elapsed:.3f}s")
            except (
                OSError,
                ValueError,
                subprocess.CalledProcessError,
            ) as error:
                if not failed:
                    raise
                print(f"Build cache scan failed: {error}", file=sys.stderr)


def evict(path, size, limit, leases):
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
        size = cache_bytes(path)
        if size <= limit:
            break
    return size


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


def worker_descriptors(env):
    if not env or not env.get("VPDF_BUILD_WORKER_FD"):
        return ()
    try:
        source = Path(env["VPDF_BUILD_WORKER"])
        leases = source.parents[2] / "leases"
        expected = [(source.parent.name, int(env["VPDF_BUILD_WORKER_FD"]))]
        for item in filter(
            None, env.get("VPDF_BUILD_RESOURCE_FDS", "").split(",")
        ):
            slot, descriptor = item.split(":")
            if slot != "gate" and not slot.isdigit():
                return ()
            name = "worker" if slot == "gate" else f"budget-{slot}"
            expected.append((name, int(descriptor)))
        for name, descriptor in expected:
            inherited = os.fstat(descriptor)
            actual = (leases / name).stat()
            if (inherited.st_dev, inherited.st_ino) != (
                actual.st_dev,
                actual.st_ino,
            ):
                return ()
        return tuple(descriptor for _, descriptor in expected)
    except (KeyError, IndexError, OSError, ValueError):
        return ()


def invoke(command, env, pass_fds=(), cwd=None):
    pass_fds = tuple(set(pass_fds) | set(worker_descriptors(env)))
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
    limit = cache_limit(env)
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    mark_origin(root, source)
    leases = root / "leases"
    leases.mkdir(exist_ok=True)
    with (
        budget(root, limit, leases) as scan,
        locked(leases / namespace, fcntl.LOCK_SH) as lease,
    ):
        scan()
        intermediates.mkdir(parents=True, exist_ok=True)
        os.utime(intermediates, None)
        try:
            return invoke([cargo, *args], env=env, pass_fds=(lease.fileno(),))
        finally:
            try:
                os.utime(intermediates, None)
            except FileNotFoundError:
                pass


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
