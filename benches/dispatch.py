"""Dispatch scenarios to headless or live backends."""

import os
import sys


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print("usage: bench:drive [headless|native|kitty] SCENARIO [OPTIONS]")
        print("Default backend: headless")
        print("Use BACKEND --help for backend options.")
        return
    backend = (
        args.pop(0)
        if args[0] in ("headless", "native", "kitty")
        else "headless"
    )
    task = "bench:headless" if backend == "headless" else "bench:live"
    if backend != "headless":
        args.insert(0, backend)
    os.execvp("mise", ["mise", "run", task, "--", *args])


if __name__ == "__main__":
    main()
