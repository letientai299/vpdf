"""Run one unittest suite in parallel shards."""

import argparse
import os
import signal
import subprocess
import sys
import unittest
from pathlib import Path

# Git-heavy suites mostly wait on process launches.
SHARDS = 4
ROOT = Path(__file__).resolve().parents[2]


def flatten(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from flatten(test)
        else:
            yield test


def discover(start, pattern):
    return list(flatten(unittest.TestLoader().discover(start, pattern, ".")))


def run_shard(start, pattern, shard):
    # Skipped modules have no loadable ID.
    tests = discover(start, pattern)[shard::SHARDS]
    result = unittest.TextTestRunner().run(unittest.TestSuite(tests))
    return 0 if result.wasSuccessful() else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-s", "--start", required=True)
    parser.add_argument("-p", "--pattern", required=True)
    parser.add_argument("--shard", type=int)
    args = parser.parse_args()
    if args.shard is not None:
        return run_shard(args.start, args.pattern, args.shard)
    count = len(discover(args.start, args.pattern))
    if not count:
        print("No tests found", file=sys.stderr)
        return 1
    command = [sys.executable, "-B", "-m", "tools.check.units"]
    command += ["-s", args.start, "-p", args.pattern]
    path = os.pathsep.join(
        filter(None, [str(ROOT), os.environ.get("PYTHONPATH")])
    )
    env = os.environ | {"PYTHONPATH": path}
    # Mise stops tasks with SIGTERM; shards must not outlive it.
    handler = signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    shards = []
    try:
        for index in range(min(SHARDS, count)):
            shards.append(
                subprocess.Popen(
                    [*command, "--shard", str(index)],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    env=env,
                )
            )
        failed = 0
        for shard in shards:
            output, _ = shard.communicate()
            print(output, end="", flush=True)
            failed += shard.returncode != 0
    finally:
        signal.signal(signal.SIGTERM, handler)
        for shard in shards:
            if shard.poll() is None:
                shard.kill()
                shard.wait()
    if failed:
        print(f"Failed shards: {failed} of {len(shards)}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
