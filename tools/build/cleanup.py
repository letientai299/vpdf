"""Reclaim build state of removed worktrees and repositories."""

import argparse
import contextlib
import fcntl
import hashlib
import os
import shutil
import sys
import time
from pathlib import Path

from tools.build.cache import (
    cache_bytes,
    cache_limit,
    cache_root,
    locked,
    mark_origin,
    prune,
)

RECORD_DAYS = 14


def idle(leases):
    with contextlib.ExitStack() as stack:
        try:
            for lease in leases.glob("*"):
                stack.enter_context(
                    locked(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
                )
        except BlockingIOError:
            return False
    return True


def remove_worktree(cache, worktree):
    path = os.fsencode(worktree)
    namespace = hashlib.sha256(path).hexdigest()[:16]
    leases = cache / "leases"
    if (cache / "build" / namespace).exists():
        try:
            with locked(leases / namespace, fcntl.LOCK_EX | fcntl.LOCK_NB):
                shutil.rmtree(cache / "build" / namespace)
        except BlockingIOError:
            print(f"Cache busy: {namespace}", flush=True)
    (cache / "affinity" / hashlib.sha256(path).hexdigest()).unlink(
        missing_ok=True
    )


def expire_records(cache):
    # Records are small; age bounds their count.
    deadline = time.time() - RECORD_DAYS * 86400
    for name in ["receipts", "diagnostics", "checks"]:
        for path in (cache / name).glob("*"):
            with contextlib.suppress(FileNotFoundError, BlockingIOError):
                if (
                    path.name == "merge.lock"
                    or path.stat().st_mtime >= deadline
                ):
                    continue
                if path.suffix != ".lock":
                    path.unlink()
                    continue
                # Waiters on a removed lock would miss later holders.
                with locked(path, fcntl.LOCK_EX | fcntl.LOCK_NB):
                    path.unlink()


def remove_orphans(cache):
    for other in cache.parent.iterdir():
        marker = other / "origin"
        if other == cache or not marker.exists():
            continue
        if Path(os.fsdecode(marker.read_bytes())).exists():
            continue
        if idle(other / "leases"):
            shutil.rmtree(other)
            print(f"Removed orphaned cache: {other}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("worktrees", nargs="*", type=Path)
    args = parser.parse_args()
    root = Path.cwd()
    env = dict(os.environ)
    cache = cache_root(env, root)
    if not cache.exists():
        return 0
    before = cache_bytes(cache.parent)
    mark_origin(cache, root)
    for worktree in args.worktrees:
        remove_worktree(cache, worktree.resolve())
    expire_records(cache)
    with locked(cache / "lock"):
        prune(cache / "build", cache_limit(env), cache / "leases")
    remove_orphans(cache)
    freed = (before - cache_bytes(cache.parent)) / 1024**3
    print(f"Build cache reclaimed: {freed:.1f} GiB", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
