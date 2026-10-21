"""Summarize independent overhead pairs."""

import argparse
import json
import statistics
from pathlib import Path


def quantile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * fraction))]


def metrics(row):
    return dict(
        fps=row["frames"] / 3,
        gap_p50_ms=quantile(row["gaps_ms"], 0.5),
        gap_p99_ms=quantile(row["gaps_ms"], 0.99),
        gap_max_ms=max(row["gaps_ms"]),
        rss_mib=row["process_rss_peak"] / 2**20,
        trace_rss_mib=row["trace_peak_rss"] / 2**20
        if row["trace_peak_rss"]
        else None,
        controller_cpu_ms=row["controller_cpu_ms"],
        process_cpu_ms=row["process_cpu_ms"],
        empty_frames=row["empty_frames"],
        bytes=row["bytes"],
        input_lateness_p99_ms=quantile(row["input_lateness_ms"], 0.99),
        input_lateness_max_ms=max(row["input_lateness_ms"]),
        blank_frames=row["trace_active_blank"],
    )


def summarize(root):
    rows = json.loads((root / "results.json").read_text())
    out = dict(
        root=str(root),
        manifest=json.loads((root / "manifest.json").read_text()),
    )
    for prefix in ["aa", "trace", "driver"]:
        labels = {}
        for row in rows:
            kind, round_, label = row["name"].split("-")
            if kind == prefix:
                labels.setdefault(int(round_), {})[label] = metrics(row)
        pairs = [v for v in labels.values() if set(v) == {"a", "b"}]
        data = {}
        for metric in pairs[0]["a"] if pairs else ():
            a = [pair["a"][metric] for pair in pairs]
            b = [pair["b"][metric] for pair in pairs]
            if None in a or None in b:
                continue
            delta = [y - x for x, y in zip(a, b)]
            data[metric] = dict(
                a_median=statistics.median(a),
                b_median=statistics.median(b),
                median_delta=statistics.median(delta),
                min_delta=min(delta),
                max_delta=max(delta),
                deltas=delta,
            )
        out[prefix] = dict(pairs=len(pairs), metrics=data)
    relay = [r for r in rows if r["relay"]]
    out["relay"] = {
        key: statistics.median(r["relay_overhead"][key] for r in relay)
        for key in (relay[0]["relay_overhead"] if relay else {})
    }
    out["monitor"] = {
        key: statistics.median(r[key] for r in relay)
        for key in (["monitor_cpu_ms", "monitor_rss_peak"] if relay else [])
    }
    out["all_final_pages"] = sorted(set(r["last_status"] for r in rows))
    out["all_traced_final_ready"] = all(
        r["trace_final"]["complete"]
        and not r["trace_final"]["blank"]
        and not r["trace_final"]["busy"]
        for r in rows
        if r["traced"]
    )
    out["all_inputs"] = sorted(set(r["inputs"] for r in rows))
    return out


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    out = summarize(args.root)
    args.json.write_text(json.dumps(out, indent=2) + "\n")
    for kind in ["aa", "trace", "driver"]:
        print(kind, out[kind]["pairs"], "pairs")
        for key in [
            "fps",
            "gap_p50_ms",
            "gap_p99_ms",
            "gap_max_ms",
            "rss_mib",
            "controller_cpu_ms",
        ]:
            print(key, out[kind]["metrics"][key])
    print("relay", out["relay"], "monitor", out["monitor"])


if __name__ == "__main__":
    main()
