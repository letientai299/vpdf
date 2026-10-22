#!/usr/bin/env python3
"""Times vpdf startup in the terminal that runs this script.

Each launch runs a binary on a pty that relays the real terminal's queries
and replies. Pairs alternate which binary starts first. A launch ends once
its frames stay quiet for --quiet seconds after the first one, then quits
with `:q`; frames after the quit do not count. Complete is the first frame
placing as many base tiles as the settled view. With --trace, complete is
the frame the reader itself first marks complete, which also waits for
requested tiles that place nothing on screen; a marker before the placed
tiles settle fails the launch. Timeouts, nonzero
exits, PDF launches without a tile, and terminal geometry changes during a
launch count as failures: they stay in every median as infinitely slow.
A pair whose two launches saw different geometry leaves the paired figures.

Usage: startup.py [options] BASE [CANDIDATE]
With one binary, the run compares it with itself (A/A)."""

import argparse
import fcntl
import hashlib
import json
import math
import os
import platform
import pty
import re
import select
import shutil
import statistics
import struct
import subprocess
import sys
import termios
import time
import tty
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FRAME_END = b"\x1b[?2026l"
QUERY = b"\x1b[c"
# Primary device attributes replies end each query round; 2048 reports
# carry rows, columns, pixel height, and pixel width.
ANSWER = re.compile(rb"\x1b\[\?[0-9;]*c|\x1b\[48;(\d+);(\d+);(\d+);(\d+)t")
CONTROL = re.compile(rb"\x1b_G([^;\x1b]*)")
METRICS = ("first_ui_ms", "first_tile_ms", "complete_ms", "picture_ms")
INF = float("inf")


def winsize():
    rows, cols, xpix, ypix = struct.unpack(
        "HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
    )
    return [rows, cols, xpix, ypix]


def write_all(fd, data):
    view = memoryview(data)
    while view:
        view = view[os.write(fd, view) :]


def drain():
    """Drops terminal input a finished launch left unread."""
    while select.select([0], [], [], 0.05)[0]:
        os.read(0, 65536)


def placements(frame):
    """Base tiles and pictures placed by one frame."""
    base = pictures = 0
    for control in CONTROL.findall(frame):
        keys = dict(
            kv.split(b"=", 1) for kv in control.split(b",") if b"=" in kv
        )
        if keys.get(b"a") != b"p":
            continue
        base += keys.get(b"z") == b"-2"
        pictures += keys.get(b"z") == b"2"
    return base, pictures


def child_setup():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def launch(binary, args, env, size, quiet, timeout):
    """One relayed launch; the row its milestones come from."""
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", *size))
    started = time.perf_counter()

    def ms():
        return (time.perf_counter() - started) * 1000

    child = subprocess.Popen(
        [str(binary), "--no-config", *args],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=env,
        preexec_fn=child_setup,
    )
    os.close(slave)
    frames, queries, replies, sizes = [], [], [], []
    pending, tail, answers = b"", b"", b""
    quit_ms, failure = None, None
    try:
        while True:
            now = ms()
            if now > timeout * 1000:
                failure = "timeout"
                break
            if (
                quit_ms is None
                and frames
                and now - frames[-1]["ms"] >= quiet * 1000
            ):
                os.write(master, b":q\r")
                quit_ms = now
            ready, _, _ = select.select([master, 0], [], [], 0.005)
            if 0 in ready:
                data = os.read(0, 65536)
                write_all(master, data)
                at = ms()
                answers += data
                end = 0
                for match in ANSWER.finditer(answers):
                    end = match.end()
                    if match.group(1) is None:
                        replies.append(at)
                    else:
                        sizes.append([int(n) for n in match.groups()])
                answers = answers[end:][-256:]
            if master in ready:
                try:
                    data = os.read(master, 1 << 16)
                except OSError:
                    data = b""
                if not data:
                    break
                at = ms()
                write_all(1, data)
                joined = tail + data
                queries += [at] * joined.count(QUERY)
                tail = joined[-(len(QUERY) - 1) :]
                start = max(0, len(pending) - len(FRAME_END) + 1)
                pending += data
                while (index := pending.find(FRAME_END, start)) >= 0:
                    frame, pending = (
                        pending[:index],
                        pending[index + len(FRAME_END) :],
                    )
                    start = 0
                    if quit_ms is None:
                        base, pictures = placements(frame)
                        frames.append(
                            {"ms": at, "base": base, "pictures": pictures}
                        )
            if child.poll() is not None and not ready:
                break
    finally:
        os.close(master)
    try:
        code = child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.kill()
        code = child.wait()
        failure = failure or "no-exit"
    if failure is None and code != 0:
        failure = f"exit-{code}"
    if failure is None and quit_ms is None:
        failure = "ended-early"
    return {
        "frames": frames,
        "queries": queries,
        "replies": replies,
        "resizes": sizes,
        "quit_ms": quit_ms,
        "exit_code": code,
        "failure": failure,
    }


def milestones(row, pdf):
    frames = row["frames"]
    if row["failure"] is None and not frames:
        row["failure"] = "no-frame"
    if row["failure"]:
        return row
    settled = frames[-1]["base"]
    if pdf and settled == 0:
        row["failure"] = "no-tile"
        return row
    row["first_ui_ms"] = frames[0]["ms"]
    row["first_tile_ms"] = next((f["ms"] for f in frames if f["base"]), None)
    row["complete_ms"] = next(f["ms"] for f in frames if f["base"] == settled)
    row["picture_ms"] = next((f["ms"] for f in frames if f["pictures"]), None)
    row["settled_tiles"] = settled
    return row


def traced(row, path):
    """Takes complete from the reader's own marker, matched by order."""
    if row["failure"] or not path.exists() or not row.get("settled_tiles"):
        return
    lines = path.read_text().splitlines()
    rows = [json.loads(line) for line in lines]
    flags = [
        row.get("complete")
        for row in rows
        if row.get("kind", "frame") == "frame"
    ]
    if all(flag is None for flag in flags):
        return
    index = next((i for i, flag in enumerate(flags) if flag), None)
    frames = row["frames"]
    if (
        index is None
        or index >= len(frames)
        or frames[index]["ms"] < row["complete_ms"]
    ):
        row["failure"] = "complete-mismatch"
        return
    row["placed_ms"] = row["complete_ms"]
    row["complete_ms"] = frames[index]["ms"]


def drifted(row):
    rows, cols, xpix, ypix = row["size"]
    return any(r != [rows, cols, ypix, xpix] for r in row["resizes"])


def stats(values):
    """Median over every value, failures (inf) included; range of the rest."""
    values = sorted(values)
    if not values:
        return None
    finite = [v for v in values if math.isfinite(v)]
    median = statistics.median(values)
    return {
        "n": len(values),
        "failed": len(values) - len(finite),
        "median": median if math.isfinite(median) else None,
        "min": min(finite, default=None),
        "max": max(finite, default=None),
    }


def value(row, metric):
    """A launch's metric, inf when it failed, None when it does not apply."""
    return INF if row["failure"] else row.get(metric)


def summarize(rows, labels):
    cases = list(dict.fromkeys(row["case"] for row in rows))
    report = {}
    for case in cases:
        mine = [row for row in rows if row["case"] == case]
        entry = {}
        for label in labels:
            runs = [row for row in mine if row["label"] == label]
            failures = {}
            for row in runs:
                if row["failure"]:
                    failures[row["failure"]] = (
                        failures.get(row["failure"], 0) + 1
                    )
            entry[label] = {"launches": len(runs), "failures": failures}
            for m in METRICS:
                values = [value(row, m) for row in runs]
                applies = any(v not in (None, INF) for v in values)
                entry[label][m] = (
                    stats([v for v in values if v is not None])
                    if applies
                    else None
                )
        if len(set(labels)) == 2:
            pairs = {}
            for row in mine:
                pairs.setdefault(row["pair"], {})[row["label"]] = row
            diffs = {m: [] for m in METRICS}
            excluded = {"both-failed": 0, "geometry": 0}
            for pair in pairs.values():
                a, b = (pair.get(label) for label in labels)
                if not a or not b or a["failure"] and b["failure"]:
                    excluded["both-failed"] += 1
                    continue
                if a["size"] != b["size"]:
                    excluded["geometry"] += 1
                    continue
                for m in METRICS:
                    first, second = value(a, m), value(b, m)
                    if first is not None and second is not None:
                        diffs[m].append(second - first)
            entry["paired"] = {"excluded": excluded}
            for m in METRICS:
                applies = entry[labels[0]][m] or entry[labels[1]][m]
                entry["paired"][m] = stats(diffs[m]) if applies else None
        report[case] = entry
    return report


def cell(s, sign=""):
    def number(v, width=0):
        return "inf" if v is None else f"{v:{sign}{width}.1f}"

    failed = f" failed={s['failed']}" if s["failed"] else ""
    return f"{number(s['median'], 7)} [{number(s['min'])},{number(s['max'])}] n={s['n']}{failed}"


def table(report, labels):
    lines = []
    for case, entry in report.items():
        for m in METRICS:
            cells = []
            for label in labels:
                s = entry[label][m]
                cells.append(f"{label} {cell(s)}" if s else f"{label} -")
            paired = entry.get("paired", {}).get(m)
            if paired:
                cells.append(f"diff {cell(paired, '+')}")
            if any(entry[label][m] for label in labels):
                lines.append(f"{case:10} {m:14} " + "  ".join(cells))
        for label in labels:
            if entry[label]["failures"]:
                lines.append(
                    f"{case:10} failures {label}: {entry[label]['failures']}"
                )
        excluded = entry.get("paired", {}).get("excluded", {})
        if any(excluded.values()):
            lines.append(f"{case:10} excluded pairs: {excluded}")
    return "\n".join(lines)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def resolve(case, recent):
    if case == "home":
        return []
    if case == "recent":
        if not recent:
            sys.exit("startup.py: case recent needs --recent STATE")
        return []
    corpus = Path(os.environ.get("VPDF_CORPUS", ROOT / "tmp/corpus"))
    for folder in (corpus, ROOT / "tmp/huge"):
        if (folder / f"{case}.pdf").exists():
            return [str(folder / f"{case}.pdf")]
    sys.exit(f"startup.py: no {case}.pdf; run mise run corpus or corpus:huge")


def environment(folder, recent, trace):
    env = {
        k: v
        for k, v in os.environ.items()
        if k not in ("TMUX", "STY", "VPDF_PREVIEW", "VPDF_TRACE")
    }
    state = folder / "state"
    (state / "vpdf").mkdir(parents=True, exist_ok=True)
    if recent:
        shutil.copyfile(recent, state / "vpdf/state.json")
    env.update(XDG_STATE_HOME=str(state), XDG_CACHE_HOME=str(folder / "cache"))
    if trace:
        env["VPDF_TRACE"] = str(folder / "frames.jsonl")
    return env


def revision():
    def git(*args):
        return subprocess.run(
            ["git", "-C", str(ROOT), *args], capture_output=True, text=True
        ).stdout.strip()

    return {
        "commit": git("rev-parse", "HEAD"),
        "dirty": bool(git("status", "--porcelain")),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("base", type=Path)
    parser.add_argument("candidate", type=Path, nargs="?")
    parser.add_argument(
        "--cases", help="comma-separated: home, recent, or a corpus PDF name"
    )
    parser.add_argument("--pairs", type=int, default=20)
    parser.add_argument(
        "--quiet",
        type=float,
        default=1.0,
        help="seconds without frames that end a launch",
    )
    parser.add_argument("--timeout", type=float, default=10.0)
    parser.add_argument(
        "--warmup",
        type=int,
        default=1,
        help="unrecorded launches per binary and case",
    )
    parser.add_argument(
        "--recent", type=Path, help="state.json for the recent case"
    )
    parser.add_argument(
        "--trace", action="store_true", help="set VPDF_TRACE per launch"
    )
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    if not os.isatty(0):
        sys.exit("startup.py: run inside the terminal under test")
    binaries = [args.base.resolve(), (args.candidate or args.base).resolve()]
    labels = ["base", "candidate"] if args.candidate else ["a", "b"]
    default = "home,book,paper,form,ocr,postgres,sdm,cjk,topo-nyc"
    cases = (args.cases or ("recent," if args.recent else "") + default).split(
        ","
    )
    inputs = {case: resolve(case, args.recent) for case in cases}
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    out = args.out or target / "startup" / time.strftime("%Y%m%d-%H%M%S")
    out.mkdir(parents=True, exist_ok=True)
    # A newly written executable stalls before main on its first runs.
    for binary in binaries:
        for _ in range(3):
            subprocess.run(
                [str(binary), "--version"],
                stdout=subprocess.DEVNULL,
                check=True,
            )
    meta = {
        "binaries": {
            label: {"path": str(b), "sha256": sha256(b)}
            for label, b in zip(labels, binaries, strict=True)
        },
        "inputs": {
            case: [{"path": p, "sha256": sha256(p)} for p in paths]
            for case, paths in inputs.items()
        },
        "recent": {"path": str(args.recent), "sha256": sha256(args.recent)}
        if args.recent
        else None,
        "source": revision(),
        "host": platform.node(),
        "platform": platform.platform(),
        "terminal": {
            k: os.environ.get(k) for k in ("TERM", "TERM_PROGRAM", "KITTY_PID")
        },
        "settings": {
            "pairs": args.pairs,
            "quiet": args.quiet,
            "timeout": args.timeout,
            "warmup": args.warmup,
            "trace": args.trace,
        },
        "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    rows = []
    saved = termios.tcgetattr(0)
    tty.setraw(0)
    try:
        for case in cases:
            for binary in binaries[: 1 if binaries[0] == binaries[1] else 2]:
                for index in range(args.warmup):
                    folder = out / "warmup" / f"{case}-{binary.name}-{index}"
                    env = environment(
                        folder, args.recent if case == "recent" else None, False
                    )
                    drain()
                    launch(
                        binary,
                        inputs[case],
                        env,
                        winsize(),
                        args.quiet,
                        args.timeout,
                    )
        with (out / "runs.jsonl").open("w") as log:
            for pair in range(args.pairs):
                for case in cases:
                    order = [0, 1] if pair % 2 == 0 else [1, 0]
                    for slot in order:
                        folder = (
                            out / "runs" / f"{pair:03}-{case}-{labels[slot]}"
                        )
                        env = environment(
                            folder,
                            args.recent if case == "recent" else None,
                            args.trace,
                        )
                        drain()
                        size = winsize()
                        row = launch(
                            binaries[slot],
                            inputs[case],
                            env,
                            size,
                            args.quiet,
                            args.timeout,
                        )
                        shutil.rmtree(folder / "cache", ignore_errors=True)
                        row.update(
                            pair=pair,
                            case=case,
                            label=labels[slot],
                            order=order.index(slot),
                            size=size,
                        )
                        if drifted(row):
                            row["failure"] = row["failure"] or "geometry"
                        milestones(row, bool(inputs[case]))
                        traced(row, folder / "frames.jsonl")
                        rows.append(row)
                        log.write(json.dumps(row) + "\n")
                        log.flush()
    finally:
        termios.tcsetattr(0, termios.TCSANOW, saved)
        write_all(1, b"\x1b[H\x1b[2J")
    meta["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    report = summarize(rows, labels)
    (out / "summary.json").write_text(
        json.dumps({"meta": meta, "report": report}, indent=2) + "\n"
    )
    text = table(report, labels)
    (out / "summary.txt").write_text(text + "\n")
    print(text)
    print(out)


if __name__ == "__main__":
    main()
