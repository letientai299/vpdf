"""Normalize benchmark output without pooling run tails.

First-use quantiles use nearest rank (p50 uses the ordinary median).
Other suites retain their producer's statistics. Report the median and range
of each run statistic, plus paired differences. A median of p95s is not a
pooled p95. Skipped scenes retain counts; a suite without timings is invalid.
Full document paths avoid collisions between equal basenames.
"""
import csv
import json
import math
import re
import statistics


def flatten(value, prefix=""):
    if isinstance(value, dict):
        for key, item in value.items():
            yield from flatten(item, f"{prefix}.{key}" if prefix else key)
    elif isinstance(value, list):
        for index, item in enumerate(value):
            label = item.get("name", index) if isinstance(item, dict) else index
            yield from flatten(item, f"{prefix}.{label}")
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        if not math.isfinite(value):
            raise ValueError(f"Nonfinite metric: {prefix}")
        yield prefix, value


def quantiles(values):
    values = sorted(values)
    if not values:
        return {}
    if not all(math.isfinite(v) and v >= 0 for v in values):
        raise ValueError("Invalid timing samples")
    return {"p50": statistics.median(values), "p95": values[math.ceil(len(values) * .95) - 1],
            "max": values[-1], "n": len(values)}


def add(values, items):
    for key, value in items:
        if key in values:
            raise ValueError(f"Duplicate metric: {key}")
        values[key] = value


def read(case, text):
    values = {}
    if case.startswith("first-"):
        for row in map(json.loads, text.splitlines()):
            key = f'{row["document"]}.{row["case"]}.{row["prepared"]}'
            if key + ".attempts" in values:
                raise ValueError(f"Duplicate interaction: {key}")
            samples = row["samples"]
            if len(samples) != row["attempts"]:
                raise ValueError("First-use attempt count differs")
            valid = [sample for sample in samples if sample["milliseconds"] is not None]
            values[key + ".skipped"] = len(samples) - len(valid)
            values[key + ".attempts"] = len(samples)
            for name in ["milliseconds", "preparation_ms", "text_cache_bytes", "ready_ms"]:
                for stat, value in quantiles([sample[name] for sample in valid
                                             if sample[name] is not None]).items():
                    values[f"{key}.{name}.{stat}"] = value
    elif case in ("latency", "crop", "stages"):
        for row in json.loads(text):
            key = row["document"]
            if case == "stages":
                key += f'.sample{row["attempt"]}.p{row["page"]}'
            add(values, flatten(row, key))
    else:
        pattern = r"(.+?)\s+([0-9.]+) ns\s+p10 ([0-9.]+)\s+p90 ([0-9.]+)"
        for match in re.finditer(pattern, text):
            name, p50, p10, p90 = match.groups()
            add(values, ((f"{name.strip()}.{label}_ns", float(value))
                         for label, value in [("p50", p50), ("p10", p10), ("p90", p90)]))
    if not values or not any(".p50" in key or ".p95" in key or key.endswith("_ms")
                             or key.endswith(".load") for key in values):
        raise ValueError("Benchmark output contains no timings")
    if not all(math.isfinite(value) for value in values.values()):
        raise ValueError("Benchmark output contains nonfinite values")
    return values


def write_csv(path, records):
    rows = []
    for (case, metric), versions in sorted(records.items()):
        if set(versions) != {"baseline", "candidate"}:
            raise ValueError(f"Unpaired metric: {case} {metric}")
        base, cand = versions["baseline"], versions["candidate"]
        if base.keys() != cand.keys():
            raise ValueError(f"Unpaired runs: {case} {metric}")
        pairs = sorted(base)
        b = list(base.values())
        c = list(cand.values())
        bm, cm = statistics.median(b), statistics.median(c)
        deltas = [cand[i] - base[i] for i in pairs]
        rows.append([case, metric, len(pairs), bm, cm, cm-bm,
                     (cm/bm-1)*100 if bm else "", min(b), max(b), min(c), max(c),
                     statistics.median(deltas), min(deltas), max(deltas)])
    with path.open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(["case", "metric", "pairs", "baseline_median", "candidate_median",
                         "delta", "delta_percent", "baseline_min", "baseline_max",
                         "candidate_min", "candidate_max", "paired_median_delta",
                         "paired_min_delta", "paired_max_delta"])
        writer.writerows(rows)
