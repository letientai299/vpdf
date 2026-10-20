# Benchmarking

Choose the measurement that can answer the question. Use the [mise tasks][tasks]
to build and run the owning [harness][benches]. Case-specific setup, controls,
and limitations live in the harness and its executable help.

## Method

State the hypothesis and metric before running. Use optimized builds and
hash-verified PDFs; [corpus preparation][corpus] verifies the public inputs.
Prepare binaries and fixtures before timing. Keep the harness, compiler,
geometry, cache policy, and transport the same between revisions.

Repeat pairs in alternating order. Retain raw samples, input and executable
hashes, revision, machine, commands, and cache conditions. Report sample counts,
medians, tails, ranges, total CPU, and peak RSS. Preserve failures and skipped
scenes. A median of per-run p95 values is not a pooled p95. Shared-host noise
and overlapping ranges limit improvement claims.

Separate input handling, frame CPU, render readiness, output flush, and observed
painting. Headless output ends in memory. Terminal output and flush markers do
not establish physical input latency or display latency. Profile only a slow
phase confirmed by unprofiled runs; verify that the capture includes that phase
before ranking contributors. Keep raw profiles and private PDFs outside git.

## First-use interactions

The [first-use harness][first-use] measures selection, cursor movement, hover,
figures, and preview opening. Each observation uses a fresh process to isolate
native workers. Scene discovery happens before application-cache reset; these
are application-cache first-use costs with warm native internals.

Compare cold, list-prepared, and text-prepared input separately from preparation
cost. Preview preparation includes its known destination. Cached-text variants
bound lookup savings; they do not validate an asynchronous worker or idle
scheduling. Text-cache weight excludes display lists and native allocations.

Rendering-active cases submit foreground work before the action and report
base-view completion separately. Pending tiles do not prove native execution
overlaps every action. Inspect successful and skipped counts before interpreting
tails, especially on pages without usable links.

## Scrolling, crop, and contention

The [latency harness][latency] drives a live worker and tracks input, frame
construction, view readiness, incomplete frames, gaps, and transferred bytes.
Use it to test whether a focused gain survives scrolling or active search. Reset
reversals inside document bounds; end-of-document clamping changes the workload.
Crop detection and cached crop toggles answer different questions.

Repeat standalone document opens when a sequential-document result changes.
Native work can outlive a dropped reader; teardown and allocator state can
affect the next case. Preserve the sequential result while investigating it.

## Focused costs and fidelity

Use [microbenchmarks][micro] for a changed function and [stages][stages] to
separate loading, interpretation, extraction, rasterization, and encoding.
Native-free packages consume [verified fixtures][fixtures] prepared outside
timing. Keep counts and byte output alongside timings. Run the relevant corpus
and OCR checks for native behavior changes. A faster stage does not establish a
faster reader or equivalent pixels.

## Reproducible comparisons

Run [snapshot preparation][prepare] in each checkout with the same harness, then
give both snapshots to the [paired runner][compare]:

```sh
mise run bench:prepare -- /tmp/base-benches --bench first_use
mise run bench:compare -- --baseline /tmp/base-benches \
  --candidate /tmp/candidate-benches --pdf /path/manual.pdf \
  --case first-active --output /tmp/bench-results
```

The candidate snapshot needs the corresponding preparation in its checkout. Use
the scripts' help for targeted cases, bounds, and platform controls. The runner
records per-child resources and writes paired metrics only after all runs
succeed. Commit clean source before publishing performance evidence.

For terminal output, use the [startup relay][startup] or [interactive
tracing][trace]. Startup compares binaries inside the terminal under test;
tracing separates frame construction and flush. Live checks still establish
visible behavior.

[tasks]: ../../.config/mise.toml
[benches]: ../../benches
[corpus]: ../../scripts/corpus.sh
[first-use]: ../../benches/first_use.rs
[latency]: ../../benches/latency.rs
[micro]: ../../scripts/bench-micro.sh
[stages]: ../../benches/stages.rs
[fixtures]: ../../examples/bench-fixtures.rs
[prepare]: ../../benches/prepare.py
[compare]: ../../benches/compare.py
[startup]: ../../scripts/startup.py
[trace]: ../../src/trace.rs
