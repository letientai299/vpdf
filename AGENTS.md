# Agent rules

Read applicable nested `AGENTS.md` files and adjacent `agents.local.md`
overrides before editing. Use the [development guide][dev] for setup and
validation.

## Changes and Git

- Fix verified issues with minimal changes. Preserve unrelated edits and reuse
  existing code; avoid unrelated refactors or lint suppressions.
- Inspect diffs and stage explicit task paths only. Commit, push, publish, or
  rewrite history only when requested. Never merge pull requests.
- Do not use `git stash` in worktrees; the stash is shared.
- Use lowercase Conventional Commit subjects without scopes.
- Never expose credentials or commit private PDFs, raw captures, or scratch
  notes. Keep lightweight working notes in gitignored `.ai/` topic folders.

## Implementation and checks

- Preserve source PDFs; keep reading state outside them. Respect native-handle
  thread ownership and keep expensive work off the interaction thread.
- Use tasks from [mise.toml][tasks]. For code changes, run `mise run lint` and
  `mise run test`; inspect automatic fixes. Do not bypass commit hooks.
- Add meaningful tests for behavior changes. Use the relevant corpus/OCR checks
  for extraction, rendering, search, or worker changes.
- Live-test terminal windows must never take keyboard focus or be raised, and
  stay visible. Launch them in the background without activating the app.
- For Linux checks, build in a container and copy only the binary and test PDFs
  to the remote host. Do not sync the source tree or install toolchains there.
- Check Markdown with the configured Rumdl and validate links after moves.
- Distinguish local checks from CI results. Report verification gaps; never
  invent measurements.

## Benchmarks and profiling

- Reuse existing mise benchmarks, builds, and inputs. State the hypothesis and
  metric; bound runs and capture duration. Start with representative cases and
  broaden only when results justify it.
- Prefer a microbenchmark of the changed function (`mise run bench:micro`) over
  whole-reader runs; put counts such as renders or cache misses in tests.
- The machine is shared with other agents and builds. Do not build during a
  run, check for orphaned `vpdf` processes first, and repeat paired runs before
  calling a regression.
- Cap size and wait time on GPU render experiments.
- Use hash-verified real PDFs and optimized builds. Record revisions, commands,
  machine, terminal geometry, and cache conditions; separate cold and warm runs.
- Profile the relevant phase and confirm the capture includes its work before
  ranking bottlenecks. Separate setup from steady-state work; treat incomplete
  traces and suspected causes as hypotheses.
- Measure gains with repeated, unprofiled baseline/candidate runs under matching
  conditions. Warm up when appropriate and alternate order. Report sample
  counts, medians, tails, and variability; do not claim gains within noise.
- Verify output fidelity and memory behavior alongside speed. A faster stage
  does not establish a faster reader.
- Distinguish CPU work, render readiness, output flush, and observed display
  latency. Keep raw profiles outside git and save a small reproducible report.

## Documentation

- Keep the README concise; categorize user guidance in [the docs index][docs].
  Keep maintainer material under `docs/dev/`, demos in [demos.md][demos], and
  published media on the dedicated closed GitHub issue. Keep app icons,
  source artwork, capture metadata, and publication records in `docs/assets/`.
  Keep raw and encoded demo files in host-local storage outside git. Reuse
  individual published images where useful.
- Keep interface inventories in executable help and changing versions, limits,
  and defaults in their owning sources. Update affected prose and diagrams.
- When adding an external tool, document when it is needed and how to install
  or configure it. Prefer upstream setup links over copied instructions.
- Keep demo build identity and attribution; screenshots do not prove
  performance.

## Writing

- Be terse and factual in docs, comments, commits, and replies. Lead with the
  result; use active voice and common words. Omit filler, preambles, repeated
  summaries, and unnecessary implementation detail.
- Use Markdown tables for numerical data and comparisons. Keep tables narrow;
  split wide ones. Never align prose columns with spaces. - Use Mermaid for
  diagrams, with `TD` direction.
- Use reference-style links in Markdown and verify targets. Put paths, commands,
  flags, environment variables, and code identifiers in backticks.
- Separate headings and lists from surrounding text with blank lines.

[dev]: docs/dev/dev.md
[tasks]: .config/mise.toml
[docs]: docs/readme.md
[demos]: docs/demos.md
