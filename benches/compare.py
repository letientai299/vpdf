#!/usr/bin/env python3
"""Compare prebuilt benchmark snapshots on Linux or macOS.

Prepare both snapshots with the same harness, compiler, and build flags.
--case is deliberate: first-use runs isolate every successful preparation in
a fresh process and are slower than microbenchmarks. --samples/--interaction
bound first-use work. PDF hashes, snapshot hashes, commands, machine, affinity,
cache policy, process resource usage, and raw output remain in a new bundle.

Pairs alternate order within each case. No resume or output-directory reuse:
stale runs must not silently enter a comparison. Failed, timed-out, malformed,
or mismatched runs stop the campaign and preserve evidence without a CSV.
Timeouts kill the process group, including first-use children. wait4 accounts
for the measured child, not differences of cumulative RSS high-water marks.
Linux reports ru_maxrss in KiB; macOS reports bytes. RSS is a process maximum,
not a sum of concurrent resident allocations. Process resources include setup
and reaped child processes; they are not input-handler CPU measurements.

OS caches are warm after hashing inputs; VPDF_CACHE is off. Cases using a
live worker measure submitted rendering, not guaranteed native overlap.
Frame bytes/readiness do not measure terminal flush or observed painting.
Use startup.py or a live trace for terminal output. Profile slow phases only
after unprofiled comparisons; confirm the capture window includes that work.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import threading
import time

from metrics import read, write_csv
from prepare import digest

CASES = {
    "first-idle": ("first_use", []),
    "first-active": ("first_use", ["--rendering"]),
    "latency": ("latency", []),
    "crop": ("latency", []),
    "stages": ("stages", []),
    "native": ("pdf-micro", ["cold page"]),
    "ui": ("ui-micro", ["page tiles"]),
}


def usage_values(usage, system):
    return {
        "cpu_user_s": usage.ru_utime, "cpu_system_s": usage.ru_stime,
        "cpu_total_s": usage.ru_utime + usage.ru_stime,
        "peak_rss_kib": usage.ru_maxrss / (1024 if system == "Darwin" else 1),
        "voluntary_switches": usage.ru_nvcsw, "involuntary_switches": usage.ru_nivcsw,
    }


def wait(child, timeout):
    expired = threading.Event()

    def terminate():
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def deadline():
        expired.set()
        terminate()

    timer = threading.Timer(timeout, deadline)
    timer.daemon = True
    try:
        timer.start()
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    finally:
        timer.cancel()
        if timer.ident is not None:
            timer.join()
        if child.returncode is None:
            terminate()
            _, status, _ = os.wait4(child.pid, 0)
            child.returncode = os.waitstatus_to_exitcode(status)
    if expired.is_set():
        raise TimeoutError("Benchmark deadline exceeded")
    return usage


def signature(case, text):
    if case.startswith("first-"):
        rows = list(map(json.loads, text.splitlines()))
        return [(row["document"], row["case"], row["prepared"], row["screen"], row["rendering"],
                 [(s["page"], s["destination"], s["milliseconds"] is None) for s in row["samples"]])
                for row in rows]
    if case in ("latency", "crop"):
        return [(row["document"], row["pages"], row["screen"]) for row in json.loads(text)]
    if case == "stages":
        return [(row["document"], row["attempt"], row["page"]) for row in json.loads(text)]
    return None


def snapshot(folder, cases):
    folder = folder.resolve()
    manifest = json.loads((folder / "manifest.json").read_text())
    for case in cases:
        name = CASES[case][0]
        if digest(folder / name) != manifest["binaries"][name]:
            raise ValueError(f"Snapshot binary changed: {folder / name}")
    return folder, manifest


def positive(text):
    value = float(text)
    if not 0 < value < float("inf"):
        raise argparse.ArgumentTypeError("Expected positive finite value")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.ArgumentDefaultsHelpFormatter)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--pdf", type=Path, action="append", required=True)
    parser.add_argument("--case", choices=CASES, action="append", required=True)
    parser.add_argument("--runs", type=int, choices=range(1, 101), default=3, metavar="N",
                        help="Alternating pairs per case")
    parser.add_argument("--samples", type=int, choices=range(1, 1001), default=31, metavar="N",
                        help="First-use attempts per interaction and preparation")
    parser.add_argument("--interaction", choices=["selection", "cursor", "hover", "figures", "preview"],
                        action="append")
    parser.add_argument("--timeout", type=positive, default=420, help="Process deadline in seconds")
    parser.add_argument("--fixtures", type=Path)
    parser.add_argument("--cpus", help="Linux CPU list, e.g. 8,9,10,11; omit on macOS")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not hasattr(os, "wait4"):
        parser.error("Process accounting requires Linux or macOS")
    cpus = {int(cpu) for cpu in args.cpus.split(",")} if args.cpus else None
    if cpus is not None and (not hasattr(os, "sched_getaffinity")
                             or not cpus or not cpus <= os.sched_getaffinity(0)):
        parser.error("CPU selection unavailable")
    cases = list(dict.fromkeys(args.case))
    snapshots = {label: snapshot(folder, cases)
                 for label, folder in [("baseline", args.baseline), ("candidate", args.candidate)]}
    for field in ["harness_sha256", "rustc", "flags"]:
        if snapshots["baseline"][1][field] != snapshots["candidate"][1][field]:
            parser.error(f"Snapshot setup differs: {field}")
    pdfs = [path.resolve(strict=True) for path in args.pdf]
    if len(set(pdfs)) != len(pdfs):
        parser.error("Duplicate PDF input")
    if "ui" in cases and not args.fixtures:
        parser.error("UI case requires --fixtures")
    args.output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, VPDF_EXTRA=os.pathsep.join(map(str, pdfs)), VPDF_CACHE="off")
    for name in ["VPDF_FIRST_USE_SAMPLE", "VPDF_FIRST_USE_RENDER", "VPDF_CROP_BENCH",
                 "VPDF_CROP_SPACE", "VPDF_BENCH_OUTPUT", "VPDF_COLUMNS"]:
        env.pop(name, None)
    if args.fixtures:
        env["VPDF_BENCH_FIXTURES"] = str(args.fixtures.resolve(strict=True))
    manifest = {
        "snapshots": {label: dict(path=str(folder), **info) for label, (folder, info) in snapshots.items()},
        "inputs": {str(path): digest(path) for path in pdfs}, "system": platform.platform(),
        "machine": {"hostname": platform.node(), "architecture": platform.machine(),
                    "processor": platform.processor(), "logical_cpus": os.cpu_count()},
        "cpus": sorted(cpus) if cpus else sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "cache": "VPDF_CACHE=off; OS cache warm after hashing inputs",
        "cases": cases, "runs": args.runs, "samples": args.samples,
        "interactions": args.interaction,
        "fixtures": {"path": str(args.fixtures.resolve()),
                     "manifest_sha256": digest(args.fixtures / "manifest.json")} if args.fixtures else None,
    }
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    records, signatures = {}, {}
    for case in cases:
        for repeat in range(args.runs):
            order = ["baseline", "candidate"] if repeat % 2 == 0 else ["candidate", "baseline"]
            for label in order:
                name, extra = CASES[case]
                binary = snapshots[label][0] / name
                command = [str(binary), *extra]
                if case.startswith("first-"):
                    command += ["--samples", str(args.samples)]
                    for interaction in args.interaction or []:
                        command += ["--case", interaction]
                runenv = dict(env)
                if case == "crop":
                    runenv["VPDF_CROP_BENCH"] = "1"
                dest = args.output / f"{label}-{case}-{repeat + 1}"
                row = {"label": label, "case": case, "repeat": repeat + 1, "command": command}
                started = time.monotonic()
                try:
                    with dest.with_suffix(".out").open("w") as stdout, dest.with_suffix(".err").open("w") as stderr:
                        child = subprocess.Popen(command, env=runenv, stdout=stdout, stderr=stderr,
                                                 start_new_session=True,
                                                 preexec_fn=(lambda: os.sched_setaffinity(0, cpus)) if cpus else None)
                        usage = wait(child, args.timeout)
                    row.update(status=child.returncode, seconds=time.monotonic()-started,
                               resources=usage_values(usage, platform.system()))
                    if child.returncode:
                        raise RuntimeError(f"Benchmark exited {child.returncode}")
                    text = dest.with_suffix(".out").read_text()
                    values = read(case, text)
                    current = signature(case, text)
                    if case in signatures and signatures[case] != current:
                        raise ValueError("Benchmark workload differs")
                    signatures[case] = current
                    values.update({f"process.{key}": value for key, value in row["resources"].items()})
                    values["process.wall_seconds"] = row["seconds"]
                    for metric, value in values.items():
                        records.setdefault((case, metric), {}).setdefault(label, {})[repeat] = value
                    if digest(binary) != snapshots[label][1]["binaries"][name]:
                        raise ValueError("Snapshot changed during measurement")
                except Exception as error:
                    row.update(error=str(error), seconds=time.monotonic()-started)
                    raise
                finally:
                    with (args.output / "runs.jsonl").open("a") as file:
                        file.write(json.dumps(row) + "\n")
                print(f"{label} {case} {repeat + 1}: {row['seconds']:.3f}s", flush=True)
    write_csv(args.output / "metrics.csv", records)


if __name__ == "__main__":
    main()
