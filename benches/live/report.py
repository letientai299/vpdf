#!/usr/bin/env python3
"""Summarize reader traces by marks."""

import argparse
import json
import math
import statistics
from pathlib import Path

TARGET_FPS = 120


def quantile(values, fraction):
    ordered = sorted(values)
    return (
        ordered[min(len(ordered) - 1, int(fraction * len(ordered)))]
        if ordered
        else None
    )


def summarize(rows, start, end, include_end=False):
    stop = math.nextafter(end, math.inf) if include_end else end
    frames = [
        r
        for r in rows
        if r.get("kind", "frame") == "frame" and start <= r["t_ms"] < stop
    ]
    fresh = [r["t_ms"] for r in frames if r.get("fresh", True)]
    gaps = [b - a for a, b in zip(fresh, fresh[1:])]
    inputs = [
        r["t_ms"]
        for r in rows
        if r.get("kind") == "input" and start <= r["t_ms"] < stop
    ]
    input_gaps = [b - a for a, b in zip(inputs, inputs[1:])]
    samples = [
        r
        for r in rows
        if r.get("kind") == "sample" and start <= r["t_ms"] < stop
    ]
    cpu_samples = [r for r in samples if r.get("cpu") is not None]
    cpu = [r["cpu"] for r in cpu_samples]
    previous = next(
        (
            r
            for r in reversed(rows)
            if r.get("kind", "frame") == "frame" and r["t_ms"] < start
        ),
        None,
    )
    timeline = ([dict(previous, t_ms=start)] if previous else []) + frames

    def known_blank(row):
        return isinstance(row.get("blank"), bool) or isinstance(
            row.get("missing"), (int, float)
        )

    def is_blank(row):
        return row.get("blank", row.get("missing", 0) > 0)

    blank = 0.0
    stand = 0.0
    if not timeline or timeline[0]["t_ms"] > start:
        blank = stand = None
    for index, frame in enumerate(timeline):
        stop = timeline[index + 1]["t_ms"] if index + 1 < len(timeline) else end
        duration = max(0, stop - frame["t_ms"])
        if duration == 0:
            continue
        if not known_blank(frame):
            blank = None
        if not isinstance(frame.get("stand_ins"), (int, float)):
            stand = None
        if blank is not None and is_blank(frame):
            blank += duration
        if stand is not None and frame["stand_ins"]:
            stand += duration
    readiness = next(
        (r["t_ms"] - start for r in frames if r.get("complete") is True),
        None,
    )
    sharp = None
    reasons = set()
    for row in frames:
        if not known_blank(row) or not isinstance(
            row.get("stand_ins"), (int, float)
        ):
            reasons.add("fidelity unavailable")
            continue
        if not isinstance(row.get("busy"), bool):
            reasons.add("idle state unavailable")
            continue
        built = row.get("built_ms")
        if not isinstance(built, (int, float)) or not math.isfinite(built):
            reasons.add("build provenance unavailable")
            continue
        if built < start:
            reasons.add("frame predates phase")
            continue
        if (
            row.get("complete") is True
            and not row["busy"]
            and not (row["stand_ins"] or is_blank(row))
        ):
            sharp = row["t_ms"] - start
            break
    sharp_reason = (
        None
        if sharp is not None
        else ("; ".join(sorted(reasons)) or "no sharp frame")
    )
    result = dict(
        inputs=len(inputs),
        input_hz=len(inputs) * 1000 / (end - start) if end > start else None,
        input_gap_p99_ms=quantile(input_gaps, 0.99),
        input_gap_max_ms=max(input_gaps, default=None),
        target_fps=TARGET_FPS,
        frame_budget_ms=1000 / TARGET_FPS,
        over_budget_gaps=sum(gap > 1000 / TARGET_FPS for gap in gaps),
        duration_ms=end - start,
        frames=len(frames),
        fresh=len(fresh),
        fresh_fps=(
            (len(fresh) - 1) * 1000 / (fresh[-1] - fresh[0])
            if len(fresh) > 1 and fresh[-1] > fresh[0]
            else None
        ),
        gap_p50_ms=quantile(gaps, 0.5),
        gap_p99_ms=quantile(gaps, 0.99),
        gap_max_ms=max(gaps, default=None),
        blank_ms=blank,
        stand_in_ms=stand,
        sharp_after_ms=sharp,
        sharp_after_unknown=sharp_reason,
        readiness_after_ms=readiness,
        cpu_mean=statistics.mean(cpu) if cpu else None,
        cpu_samples=len(cpu),
        cpu_sample_span_ms=(
            cpu_samples[-1]["t_ms"] - cpu_samples[0]["t_ms"]
            if len(cpu_samples) > 1
            else 0
        ),
        rss_peak=max((r["rss"] for r in samples), default=None),
    )
    prior_samples = [
        r for r in rows if r.get("kind") == "sample" and r["t_ms"] <= start
    ]
    before = prior_samples[-1] if prior_samples else None
    after = samples[-1] if samples and samples[-1]["t_ms"] > start else None
    result["rss_before"] = before.get("rss") if before else None
    result["rss_before_ms"] = before["t_ms"] if before else None
    result["rss_after"] = after.get("rss") if after else None
    result["rss_after_ms"] = after["t_ms"] if after else None
    result["rss_before_estimated"] = before["t_ms"] < start if before else None
    result["rss_after_estimated"] = after["t_ms"] < end if after else None
    result["rss_delta"] = (
        result["rss_after"] - result["rss_before"]
        if result["rss_before"] is not None and result["rss_after"] is not None
        else None
    )
    result["rss_delta_estimated"] = (
        result["rss_before_estimated"] or result["rss_after_estimated"]
        if result["rss_delta"] is not None
        else None
    )
    result["resource_release"] = None
    if timeline and "resources" in timeline[0] and "resources" in timeline[-1]:
        before = {r["owner"]: r for r in timeline[0]["resources"]}
        after = {r["owner"]: r for r in timeline[-1]["resources"]}
        result["resource_release"] = [
            dict(
                owner=owner,
                before=before[owner],
                after=after[owner],
                released={
                    key: before[owner][key] - after[owner][key]
                    for key in (
                        "text_bytes",
                        "tile_bytes",
                        "search_workers",
                        "active_workers",
                    )
                    if key in before[owner] and key in after[owner]
                },
            )
            for owner in sorted(before.keys() & after.keys())
        ]
    for field, metric in (
        ("missing_pixels", "missing_pixel_ms"),
        ("stand_in_pixels", "stand_in_pixel_ms"),
    ):
        area_time = 0.0 if timeline and timeline[0]["t_ms"] <= start else None
        for index, row in enumerate(timeline):
            until = (
                timeline[index + 1]["t_ms"]
                if index + 1 < len(timeline)
                else end
            )
            duration = max(0, until - row["t_ms"])
            if duration == 0:
                continue
            if not isinstance(row.get(field), (int, float)):
                area_time = None
            if area_time is not None:
                area_time += row[field] * duration
        result[metric] = area_time
    for field in (
        "paint_ms",
        "build_ms",
        "delivery_ms",
        "flush_ms",
        "input_to_flush_ms",
    ):
        values = [
            r[field]
            for r in frames
            if r.get(field) is not None
            and (field != "build_ms" or r.get("fresh", True))
        ]
        for name, fraction in (("p50", 0.5), ("p99", 0.99)):
            result[f"{field}_{name}"] = quantile(values, fraction)
        result[f"{field}_max"] = max(values, default=None)
    for field in ("bytes", "base_upload_bytes", "upload_bytes"):
        values = [r[field] for r in frames if field in r]
        total = sum(values) if values else None
        result[field] = total
        result[f"{field}_per_s"] = (
            total * 1000 / (end - start)
            if total is not None and end > start
            else None
        )
    return result


def analyze(path):
    rows = [
        json.loads(line)
        for line in Path(path).read_text().splitlines()
        if line.strip()
    ]
    for row in rows:
        row["t_ms"] = row.get("t_ms", row.get("elapsed_ms"))
        if not isinstance(row["t_ms"], (float, int)):
            raise ValueError("Trace timestamp missing")
    rows.sort(key=lambda r: r["t_ms"])
    if not rows:
        raise ValueError("Trace is empty")
    marks = [
        (r["t_ms"], r["name"])
        for r in rows
        if r.get("kind") == "mark" and not r["name"].startswith("__bench_wait_")
    ]
    boundaries = [(rows[0]["t_ms"], "startup"), *marks]
    phases = []
    for index, (start, name) in enumerate(boundaries):
        end = (
            boundaries[index + 1][0]
            if index + 1 < len(boundaries)
            else rows[-1]["t_ms"]
        )
        phases.append(
            dict(
                name=name,
                start_ms=start,
                **summarize(rows, start, end, index == len(boundaries) - 1),
            )
        )
    return dict(
        trace=str(Path(path).resolve()),
        phases=phases,
        total=summarize(rows, rows[0]["t_ms"], rows[-1]["t_ms"], True),
    )


def markdown(report):
    lines = [
        "| Phase | Fresh fps | Gap p99 ms | Blank ms | RSS bytes |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for row in report["phases"]:

        def show(key):
            value = row[key]
            return f"{value:.2f}" if value is not None else "—"

        name = row["name"].replace("|", "\\|").replace("\n", " ")
        lines.append(
            f"| {name} | {show('fresh_fps')} | {show('gap_p99_ms')} | "
            f"{show('blank_ms')} | {show('rss_peak')} |"
        )
    return "\n".join(lines)


def paired(reports):
    metrics = (
        "fresh_fps",
        "gap_p99_ms",
        "blank_ms",
        "stand_in_ms",
        "sharp_after_ms",
        "cpu_mean",
        "rss_peak",
    )
    rounds = {}
    for report in reports:
        rounds.setdefault(report["round"], {})[report["label"]] = report
    differences = {}

    def indexed(phases):
        counts = {}
        result = {}
        for phase in phases:
            name = phase["name"]
            counts[name] = counts.get(name, 0) + 1
            result[name, counts[name]] = phase
        return result

    for pair in rounds.values():
        if set(pair) != {"base", "candidate"}:
            continue
        base = indexed(pair["base"]["phases"])
        candidate = indexed(pair["candidate"]["phases"])
        for name, occurrence in base.keys() & candidate.keys():
            for metric in metrics:
                a, b = (
                    base[name, occurrence][metric],
                    candidate[name, occurrence][metric],
                )
                if a is not None and b is not None:
                    differences.setdefault(
                        (name, occurrence, metric), []
                    ).append(b - a)
    return [
        dict(
            phase=name,
            occurrence=occurrence,
            metric=metric,
            samples=len(values),
            median_delta=statistics.median(values),
            min_delta=min(values),
            max_delta=max(values),
        )
        for (name, occurrence, metric), values in sorted(differences.items())
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    report = analyze(args.trace)
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    print(markdown(report))


if __name__ == "__main__":
    main()
