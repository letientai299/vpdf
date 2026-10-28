"""Merge worktrees into main one at a time."""

import fcntl
import os
import shutil
import sys
from pathlib import Path

from tools.build.cache import invoke
from tools.check.hooks import check_path


def merge(root, args):
    wt = shutil.which("wt")
    if not wt:
        raise RuntimeError("Worktrunk executable missing")
    with check_path(root, "merge", "lock").open("a") as stream:
        # Queued merges rebase once instead of racing.
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("Waiting for another merge", flush=True)
            fcntl.flock(stream, fcntl.LOCK_EX)
        return invoke([wt, "merge", *args], env=dict(os.environ), cwd=root)


if __name__ == "__main__":
    try:
        status = merge(Path.cwd(), sys.argv[1:])
        sys.exit(status if status >= 0 else 128 - status)
    except (OSError, RuntimeError, ValueError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
