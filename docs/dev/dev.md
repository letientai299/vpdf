# Development

[Mise tasks][tasks] define the supported build and validation workflow. Keep
task names, tool versions, platform targets, and numerical budgets there or in
their owning source files.

## Build from source

The [embedded help build][help-build] prepares the manual before compilation.

Install [mise][mise] and the native build dependencies: a C/C++ compiler, Make,
and libclang. Development tasks also use [Git][git], `curl` for corpus
downloads, and [Python 3][python] for script tests, benchmarks, and demo
capture. Mise installs the Rust toolchain and the tools pinned in the
[task configuration][tasks].

On macOS, install the [Xcode command line tools][xcode] with
`xcode-select --install` and [pkgconf][pkgconf] with `brew install pkgconf`.
If bindgen cannot find libclang, install [Homebrew LLVM][llvm] and set
`LIBCLANG_PATH` to its `lib` directory. On x86-64 Ubuntu:

```sh
sudo apt-get install build-essential libclang-dev musl-tools git curl python3
```

From a checkout:

```sh
mise trust
mise install
mise run install
```

The [build script][build] stages the optimized executable at
`target/release/vpdf`, or under `CARGO_TARGET_DIR` when set. The
[install script][install] atomically replaces `~/.local/bin/vpdf`; put that
directory on `PATH`. `install:cli` builds and installs only the terminal
reader. `install` also installs the macOS app on macOS; it builds both in one
shared-worker request and skips the disk image.

On macOS, build, install, and release run `--version` at the final executable
path. This pays first-launch security checks during the task before interactive
use; macOS can repeat checks after security updates.

Build on the target platform. Linux builds use musl and reject shared-library
dependencies. macOS builds embed MuPDF and its base14 fonts, link the system
zlib, and reject non-system dynamic libraries. These checks cover the
executable; optional helpers remain separate programs. [Build scripts][build]
and the [CI matrix][ci] define supported hosts.

## Build cache

Build and validation tasks share compilation by default. Worktrees keep
independent source edits; the [shared worker][build-worker] serializes
compilation and tests in one repository snapshot. It compares file contents,
includes nonignored untracked inputs, and preserves unchanged source timestamps.
Hooks use the same snapshot and artifacts, but validate index contents or pushed
commits.
Private Git metadata preserves commit, tag, and patch identity while reusing the
repository's objects. Snapshot builds do not change the caller's index or refs.

```sh
mise run validate
mise run check
mise run build
```

Successful reader builds copy their executable into the calling worktree's
`target/debug/vpdf` or `target/release/vpdf`, respecting `CARGO_TARGET_DIR`. GUI
bundles, release packages, and generated help assets also copy back when their
build tasks finish.
Copies finish before the worker accepts another request. Compiler diagnostics
refer to the shared snapshot; its sources reflect the most recent request.
Changed inputs and different profiles, features, or toolchains can still require
recompilation. Editing remains parallel; shared validation waits for the worker.

The default remains one worker. Experiment with a bounded pool using
`VPDF_BUILD_WORKERS=2 mise run validate`. Each worker owns its source snapshot,
Git metadata, build state, and outputs. Requests prefer their worktree's
previous worker and use another idle worker when available. Successful receipts
remain in the repository cache. Worker-specific paths can still prevent receipt
reuse across workers.

The pool splits the configured compiler and test concurrency totals between
workers; it validates those totals before queuing. Single-worker requests
reserve both resource slots. Pool requests also hold shared admission leases,
so older launchers using the original exclusive worker lease serialize with
the pool. Pool snapshots must contain updated validation scripts so child
processes retain every lease; use the default mode for older snapshots.
Direct Cargo and benchmark commands
also do not participate in this budget. All worker caches share the disk budget,
and active worker leases prevent pruning.

Run `mise run validate` for final code validation. It captures the working
inputs before waiting, then runs read-only lint, the full test suite, and a
debug terminal build under one worker lease. The debug executable copies back
to the caller. Queue, synchronization, lint, test, build, and worker durations
appear in command output. These timings include command and cache overhead;
they are not reader performance measurements.
Request IDs identify the selected worker and validation scope. Synchronization
also reports pruning, file synchronization, and Git setup separately. Receipt
diagnostics report hits, misses, forced checks, disabled-reuse reasons, and
changed fingerprint categories. They store category hashes outside the worker
snapshot and never print environment values. A category change explains a key
change; it does not establish that the changed input can safely be ignored.

Successful lint and test checks write local receipts outside the worker's
mutable directory. Validation and hooks reuse them only for matching source
contents and modes, Git revision, tags and configuration, environment, tools,
and corpus contents. A full test receipt can cover the affected Rust test
subset; an affected subset cannot cover the full suite. The CI recipe records
feature and packaging checks separately; standard-test receipts cannot cover
them. Changed provenance invalidates receipts even
when source contents match. Receipts are local evidence, not CI results.

Set `VPDF_VALIDATE_FORCE=1` to rerun checks. CI, isolated execution, source or
corpus symlinks, unavailable tools, and custom compiler wrappers or flags
disable receipt reuse. Selected external compiler and OCR configuration also
disables reuse. Custom Cargo or nextest configuration and enabled test retries
also disable reuse; the [receipt manager][validation] owns those exclusions.
Relative input paths resolve against the checked source; home defaults use its
environment. Fresh full-suite checks and forced test checks invalidate previous
test receipts for matching inputs, including subsets. Lint receipts remain
independent. Input changes during a check prevent a success receipt from being
saved. Failed or interrupted checks never write a success receipt. Debug builds
always consult Cargo, so missing or replaced binaries cannot be restored from a
test receipt.

Commit hooks format a private staged snapshot before requesting the worker.
Formatting fixes require restaging and preserve unstaged edits. Prose-only
commits that do not affect generated help skip the Rust worker entirely.

Run `mise run lint` in the worktree when fixes are needed. Formatters edit the
worktree; Rust lint checks run through the shared worker. Clippy findings
require manual fixes. Interactive tasks build through the worker, then launch
the copied executable from the caller. Installation also uses caller outputs.
Direct Cargo and benchmark tasks retain their existing local behavior. Corpus
inputs stay in the caller's `tmp/` directory and are linked into the snapshot
instead of copied. Shared tests can fetch their existing corpus inputs there.

`CARGO_BUILD_JOBS` and `NEXTEST_TEST_THREADS` override the worker's concurrency
limits. For a single worker, an opt-in test limit can be set per invocation:

```sh
VPDF_BUILD_WORKERS=1 NEXTEST_TEST_THREADS=8 mise run test
```

Choose the limit for available CPU and memory; this does not change the default
or relax the worker pool's total budget checks. The worker also applies the
compiler limit to MuPDF's Make invocation, which otherwise selects all available
CPUs. Defaults live in the worker. Development builds retain line tables and
disable incremental compilation to limit disk use.

The [cache manager][build-cache] bounds intermediate storage under one
repository cache in the user cache directory. The worker's source, intermediate
artifacts, and final outputs share that budget with direct Cargo's isolated
caches. Active commands protect their caches until exit and can temporarily
exceed the budget. Set `VPDF_BUILD_CACHE_GIB` to override the budget. Worktree
removal preserves the warm worker cache for reuse.

Run `mise run cache:prune` to remove idle legacy isolated build caches and
legacy hook sources belonging to currently registered worktrees. It preserves
the shared worker, active legacy builds, caller outputs, and unrelated
repositories. Legacy hook sources from already removed worktrees cannot be
identified safely and are left in the operating system's temporary directory.
Existing unmanaged build directories remain until cleaned separately.

Set `VPDF_BUILD_MODE=isolated` to run a task in its calling worktree without
the shared worker. An explicit `CARGO_BUILD_BUILD_DIR` also opts out and stays
caller-owned. CI retains its existing Cargo cache layout. Cargo outside Mise
also keeps its normal behavior; use `mise exec -- cargo ...` for managed direct
builds.
Explicit manifest, build-directory, target, profile, or artifact-selection flags
also run in the caller so their paths and output selection keep their meaning.

## Native window

Build the macOS app and disk image:

```sh
mise build:app
mise install:app
mise test:app
```

The [app packager][build-app] creates `target/dist/Vpdf.app` and
`target/dist/Vpdf.dmg`, respecting `CARGO_TARGET_DIR`. It builds for the host
architecture with the `app` Cargo profile and uses Python 3 and the macOS
command line tools. The DMG uses LZFSE compression and includes an Applications
shortcut. `install:app` builds the app without the DMG and atomically replaces
`~/Applications/Vpdf.app` without launching it. `test:app` checks the mounted
DMG and installation in a temporary home directory.

In an interactive macOS session, run `mise run test:app:open` to check Finder
document events at launch and while running, multiple files, Unicode paths,
and normal quit. It uses a temporary app and state in a background window.
The macOS-only [document event adapter][document-events] preserves Winit's
application delegate and keeps native interoperation outside the reader.

The PNG and ICNS icons derive from the [source SVG][icon]. After editing it,
run `mise build:icon` on macOS. It uses the native SVG renderer through `sips`
and packages the sizes with `iconutil`; app builds use the checked-in icons.

The app uses an ad hoc signature for local use. Distribution with Gatekeeper
approval requires Developer ID signing and notarization; see [Apple's
distribution guide][apple-distribution]. Linux and Windows app packaging are
not supported.

Run the experimental native reader on macOS:

```sh
mise try:gui -- path/to/file.pdf
```

Like `mise try`, this builds a debug executable and forwards arguments. Omit the
path for the start screen. Normal launches activate the window. The window
uses the shared reader keys, tabs, splits, search, text view, and reading-state
storage. PDF jobs run on a session thread;
the window thread uploads RGB tiles and pictures and paints the fixed cell
grid. Both frontends share pane composition, including previews, recent cards,
menus, clues, region outlines, and scrollbar drawing.

Native pointer events use the session's shared hit testing, link handling,
selection, context menus, pane dragging, and hover-preview deadlines. Cursor
shapes follow the same shared policy; focus loss and exit release held gestures.
Wheel points, lines, and pages become shared mouse reports, retaining pointer
position and modifiers for panel scrolling, previews, pan, and Ctrl-zoom.
Prompts accept IME composition through the shared editor. Marked text stays
uncommitted until confirmation; candidate windows follow the prompt caret. Alt
chords retain the terminal's word-editing behavior. Region captures use the
shared jobs and clipboard helpers. Completed text from background panes reaches
the GUI clipboard when focus moves elsewhere. On macOS, UI text uses installed
system fonts for missing CJK glyphs. Protected documents use the shared masked
password prompt. Cancel keeps the previous view; retry resumes the original
open, split, or mark command. Switching windows preserves committed passwords
and cancels IME composition. On macOS, `:fzf` uses the system file chooser
through the built-in `osascript`; it starts in the current directory and
replaces the focused pane. File drops open tabs. Command-W closes the active tab
and its splits; the last tab returns Home. Closing empty Home exits. The
terminal retains its existing close behavior. External prompt editing requires
the terminal reader.

On macOS, `--background` disables automatic activation; combine it with
`open -g` when launching the app bundle. macOS delivers posted keys to a
background app but drops posted clicks and wheel turns. For live checks without
focus, the hidden `--events PATH` option reads keys, window geometry, marks,
pointer, wheel, modifier, focus, IME, paste, and file-drop lines from a file or
FIFO. Production builds reject the option and omit the reader and injection
hooks. The non-default `bench-events` Cargo feature enables them:

```sh
VPDF_BENCH_EVENTS=1 mise run install
```

Install tasks build production binaries when `VPDF_BENCH_EVENTS` is unset
or `0`; CI explicitly uses `0`. Feature-enabled binaries report `[bench-events]`
in `--version`; retain this marker in benchmark provenance.
`mise run test:events` checks a separate feature-enabled build; CI runs it
before packaging production binaries. It lints the workspace and tests the
reader and GUI packages that enable the feature. The default workspace pass
covers the remaining packages. Reinstall without
the variable after live checks. The [injection module][inject] documents FIFO
primitives. The [shared parser][scenario] also accepts driver-side waits,
repeats, and seeded key mixes. Injected events skip OS translation, so real
pointer devices, OS drags, and IME candidate windows still need manual checks.

The native `:perf` overlay samples process CPU and RSS. FPS counts content
frames received and painted by the window; Upload counts RGBA texture bytes
submitted to egui. Overlay-only redraws do not count as content frames. These
counters do not measure GPU completion or observed display latency.
The painter performance budget has not been measured. Use the terminal reader
for complete feature support. Release and install tasks still target it.

## Validation

```sh
mise run hooks
mise run check
mise run lint
mise run test
```

The default test task includes Rust documentation examples. Run
`mise run test:docs` to check those examples separately.

Checks cover every workspace member. Support crates own their unit tests;
package selection avoids compiling the application and its native dependencies
when the selected crate does not need them:

```sh
mise run test:crate -- vpdf-ui
mise run test:crate -- vpdf-model text::tests
mise run test:crate -- vpdf-search search::tests
```

The [package guide][packages] describes ownership and permitted dependencies.
`check`, `lint`, and `test` also run `check:packages`, which validates all
dependency kinds and transitive native isolation. Run `test:packages` for the
verifier's regression tests.

PDF tests require MuPDF; ignored corpus tests also require the verified
public PDFs from `mise run corpus`. Run them independently:

```sh
mise run test:crate -- vpdf-pdf --test corpus --run-ignored only
```

Root modules retain the application compilation unit and support test filters:

```sh
mise run test:crate -- vpdf --lib session::tests
mise run test:crate -- vpdf-ui --lib layout::tests
```

The lint task applies automatic fixes before checking for remaining issues;
inspect its diff before staging. `lint:check` checks without applying fixes.

Validation tasks and Git hooks disable disk caching. Ordinary reader launches
follow `VPDF_CACHE`. Set `VPDF_CACHE=off` for direct `cargo` tests to keep test
data out of the user cache.

`mise run hooks` installs both Git hooks. The [hook dispatcher][hook-script]
formats staged Rust, Python, Markdown, and TOML files, then runs tests for
changed crates and their reverse dependents. Known Python changes run their
own test groups. Prose changes skip tests; shared fixtures, build inputs, and
unknown impact select full tests. [Nextest filtersets][nextest-filters] preserve
downstream unit and integration coverage.

Formatting fixes require restaging. Partially staged files are checked from
the index without changing their working copies. Formatting and tests use a
stable snapshot outside the checkout; build artifacts stay outside that
snapshot. `mise run hook:commit` repeats this staged check.
Compilation and shared-code changes can lengthen a commit.

Before a push, every distinct pushed commit tip runs `ci:check` in its own
snapshot, including tips outside the current branch. The current platform
runs the same non-publishing recipe as [CI][ci]: verified corpus preparation,
read-only lint, standard and benchmark-feature tests, release packing, and
archive verification. Archive checks validate its checksum, single executable,
and startup after unpacking. CI also runs read-only lint on macOS.

Run `mise run ci:check` to check the working snapshot before committing. The
[shared recipe][ci-script] checks that validation leaves source inputs unchanged
and reuses matching successful lint, test, feature, and packaging receipts.
Failed stages rerun without discarding earlier successful stages. Packaging
reuse also requires matching executable/archive contents and permissions;
missing or changed outputs rerun packaging. Help assets are prepared even
when lint is reused. CI always runs fresh; `VPDF_VALIDATE_FORCE=1` forces local
checks. The packaging key also includes UPX on Linux.
It shares Cargo compilation caches and copies release outputs into
`target/dist`, respecting `CARGO_TARGET_DIR`. Its compiler and benchmark-input
settings live in the task configuration and match CI. It requires the build
and development tools described above, including UPX for Linux packing.

A passing push check covers the current platform. CI checks both supported
platforms. Hosted artifact uploads, publication, published-binary installation,
and Homebrew updates remain separate post-push checks. Ignored corpus/OCR
and app/DMG checks still run separately when applicable.

Branch deletions need no tests. The shared worker serializes validation across
worktrees and reuses unchanged files and Cargo artifacts across runs.

Install with `mise run hooks` rather than `prek install`:
[prek's shim][prek-run] hides unstaged edits before dispatch, preventing
check-only handling of partially staged files. The [prek configuration][hooks]
retains manual stage entries; use `mise run hook:commit` for the staged-file
preservation behavior.

Python uses [Ruff][ruff] for lint fixes, import sorting, and formatting through
the lint task and commit hook. Run `mise run lint:python` for Python alone.

Markdown uses [Rumdl configuration][rumdl] for formatting and checks. Keep links
relative within the repository, use Mermaid for diagrams, and put changing
interface details in executable help.

Ordinary tests run without external OCR or downloaded corpus files. For changes
to native extraction, rendering, search, or workers, also run the relevant
real-document checks:

```sh
mise run test:corpus
mise run test:ocr
```

Both tasks fetch the public corpus and verify hashes through the
[corpus script][corpus]. OCR integration tests require Tesseract and its
language data. Large stress documents have a separate fetch task and are opt-in
inputs; standard tasks do not consume them automatically.
Keep download URLs and hashes in the corpus script.
[Application corpus tests][corpus-tests] cover integrated behavior;
[PDF corpus tests][pdf-corpus-tests] cover native extraction and rasterization.

Terminal behavior needs its own interactive checks. Native rendering or state
tests do not verify Kitty placement, tmux passthrough, clipboard policy, or
behavior after a child program uses the screen. [Preview composition
checks][preview] cover centering and failures on later pages; live checks must
still verify placement and terminal graphics.

## Performance validation

Measure CPU work separately from terminal painting and end-to-end input latency.
The [warm reader benchmark][reader-bench] covers extraction, navigation, search,
and serialized frames. `VPDF_PERF_CHECK=1` enforces CPU frame and motion
limits and base-image reuse; the thresholds live with the assertions.

```sh
VPDF_PERF_CHECK=1 mise run bench
mise run bench:latency
mise run bench:stages
```

The [scenario driver][scenario-driver] runs the real session without a window.
Scenario headers pin PDF hashes, layout, and crop mode. Run
`mise run bench:drive -- benches/scenarios/keys.txt`; results and traces stay in
the host cache. The catalog defaults to `sdm.pdf`; `mise run corpus:huge -- sdm`
fetches and verifies it. Use `--huge DIRECTORY` for an additional large PDF with
its own hash-pinned scenario. Headless delivery measures frame collection,
excluding GPU painting and terminal output. Each headless scenario replicate
runs in a fresh process; its populate/hide/restore sequence shares that process.
Reports separate embedded build provenance from the runtime checkout, whose
status stays unknown when Git fails. Patch identity includes tracked changes and
nonignored untracked files. Executable hashes identify copied builds. The
general latency cases use this bridge; crop measurements retain their component
loop. Enqueue time measures input submission, rather than the former direct
action CPU time.

Summarize marks with `mise run bench:report -- TRACE --json REPORT`. CPU means
use the sampler's rolling percentages. A `clock` row anchors reader timestamps
to Unix time for launch timing. Sharp-after measures the first complete frame
without stand-ins or active jobs after a mark. It does not measure observed
display latency. Missing fidelity, idle, or build-time evidence makes sharp
timing unknown; readiness-only timing remains separate. Mixed-schema coverage
durations also stay unknown. Pixel-area durations measure renderer coverage.
Named marks capture post-command checkpoints. Motion and search-navigation
assertions compare position or selection with the named checkpoint, retaining
pane/document/query identity. Live and headless waits reject application errors.
Native extended observations require an enabled trace or assertion consumer;
ordinary frames omit their clones and cache scans. Build timing includes that
work when enabled, with observation time reported separately.
Scenario `expect` steps assert outcomes within the run deadline; headless help
lists their fields. Idle alone does not prove search or navigation succeeded.
Resource rows separate text and tile cache bytes from RSS and track owner
search services and active document coordinators/search threads, including
retiring ones. Shared raster-pool threads and allocator retention are outside
those owner counts. Native texture caches release hidden owners during paint.
The [live launcher][live-driver] accepts paired feature-enabled binaries,
alternates order, and keeps individual reports. Use
`mise run bench:drive -- native --help` for arguments; use `kitty` for the
terminal backend. Override the scenario window with
`--window W H` when needed; reports retain the requested size. Choose a
placement where the entire window fits visibly on one display; the default
chooses a corner outside the foreground window. The launcher rejects focus
changes, occlusion, a locked session, and active builds. Battery runs require
`--allow-battery`; reports retain power state. `wait idle` requires completed
rendering and finished session jobs. Native waits reject paints of frames built
before the marker. Kitty acknowledges pty geometry before later wheel input.
Each run copies its executable before launch and records that copy's hash;
native reports also identify the signed executable. Kitty relays input and
output through bounded nonblocking queues. Failed runs retain their error and
cleanup records. Retain one real macOS resize check per release; FIFO geometry
bypasses OS input paths.

Use `benches/scenarios/keys-120.txt` and `keys-240.txt` to stress cadence;
`keys-1000.txt` stresses input throughput. Reports separate requested input
rates, reader input cadence, and frame cadence. The report's frame budget comes
from its application FPS target. Input throughput does not establish rendering
FPS. The persistent window monitor and pty relay keep subprocess creation and
disk writes outside paced key delivery. Controller, monitor, and relay CPU/RSS
accounting stays separate from app samples; include tracing overhead in matched
checks. The Kitty backend relays raw keys and terminal replies through a child
pty. Its `startup` scenario reports reader readiness separately from output
delivery. It shares transport helpers with the existing startup task.

The [microbenchmarks][micro] accept case filters and `--package` selection.
Model, search, reader, terminal, and UI cases use prepared real-PDF fixtures.
UI cases measure scene construction without terminal serialization. Combined
UI/terminal workloads run under the application `presentation` benchmark,
included by `bench:micro -- --package vpdf`; their workload names are unchanged.
Fixture hashes are checked and source identity is recorded outside the timed
loops. Preparation requires MuPDF; these packages and protocol execute their
benchmarks without it. PDF benchmarks open native documents; application cases
retain integrated frames, routing, and latency. Run `mise run corpus` before
native cases to verify their inputs.

The PDF `crop worker` case measures a sampled batch with interpreted display
lists and a subsequent page with a cached list. It opens the document and worker
handle before timing. `VPDF_CROP_INPUT` selects another real PDF; otherwise it
uses the corpus book. Verify the input hash before comparing runs. These cases
measure crop detection with warm file access, not reader display latency.

Microbenchmarks report individual-call median, p95, p99, and sample count.
They print timer overhead separately; short calls can approach that floor.
Calibrated batch throughput stays separate from latency percentiles. Sampling
uses the limits in the [shared helper][bench-support]. Sparse tails report
`n/a`; JSON reports use `null`. Repeat matched runs before comparing tails.

Use `mise run corpus:huge -- sdm postgres` before `mise run bench:manuals`.
The [manual benchmark][manuals] separates document opening, retained memory,
search indexing and matching, geometry, navigation, and paced overview frames.
Pass a case name to select a phase, or set `VPDF_EXTRA` to a platform-separated
list of verified PDF paths. Keep source hashes and index hashes with results.
Compare built executables in alternating order after warmup; keep compilation
outside measurement windows. Report OS file-cache conditions separately from
fresh document handles. Overview frames go to memory and do not measure terminal
painting or SSH delivery.

The public corpus includes Vietnamese and Chinese reports for search and disk
cache checks; the corpus script owns their URLs and hashes. Select those PDFs
through `VPDF_EXTRA` for manual benchmarks. `VPDF_QUERIES` supplies
comma-separated queries, and `VPDF_WHOLE_WORD` selects whole-word matching.
Matching timings follow an untimed lookup, so CJK dictionary boundaries are
warm. Compare hit counts as well as time when normalization changes the result
set.

Set `VPDF_BENCH_FIXTURES` to share a prepared fixture directory between
checkouts. Case filters apply within the selected package; omit `--package` to
run matching cases across the workspace.

```sh
mise run bench:fixtures
mise run bench:micro -- --package vpdf-model
mise run bench:micro -- --package vpdf-search index
mise run bench:micro -- --package vpdf-reader
mise run bench:micro -- --package vpdf-terminal
mise run bench:micro -- --package vpdf-ui
mise run bench:micro -- --package vpdf-gui
mise run bench:micro -- --package vpdf-protocol decode
mise run corpus
mise run bench:micro -- --package vpdf-pdf
```

The [latency benchmark][latency] uses a live render worker to measure cold
opens, page and history jumps, zooms, section and search navigation, jumps
during a search scan, and paced scrolling, including tile readiness and blank
frames. The [stage benchmark][stages] separates page loading, text extraction,
display list construction, rasterization, and encoding.

The [raster pool][render-worker] caps concurrency separately from cache
decoding. Native drawing shares locks; extra raster threads can increase
contention instead of throughput.

The [startup benchmark][startup] runs inside the terminal under test. It
launches one or two executables on a relay of that terminal, alternating their
order per pair, and reports first frame, first page tile, and complete initial
view, with paired differences. Completion normally comes from emitted tile
placements; `--trace` uses the reader's own completion marker. These timings end
at terminal output, not observed display painting. Pass one executable for an
A/A noise check. Results, hashes, and geometry go under `target/startup`;
see the script's `--help` for options.

```sh
mise run bench:startup -- BASE CANDIDATE
```

The latency and stage benchmarks accept `VPDF_EXTRA` as a platform path-list of
PDFs. The latency benchmark also accepts `VPDF_COLUMNS` to vary its simulated
screen. The warm reader and latency benchmarks can save JSON with
`VPDF_BENCH_OUTPUT`. Keep input provenance, hashes, screen geometry, and build
identity with results used to justify a change.

For an interactive trace:

```sh
VPDF_TRACE=/path/frames.jsonl vpdf document.pdf
```

[Tracing][trace] overwrites the selected file. Both frontends write JSONL rows
selected by `kind`. Every row has `t_ms`, milliseconds since its writer opened,
and the compatibility alias `elapsed_ms`. Consumers must filter
`kind == "frame"` before computing frame counts or matching terminal output.
Older frame-only traces lack `kind`.

| Kind     | Fields and meaning                                                   |
| -------- | -------------------------------------------------------------------- |
| `frame`  | Frontend-specific frame fields below.                                |
| `input`  | `step`: one-based frontend-local input counter.                      |
| `mark`   | `name`: escaped phase label supplied to the writer.                  |
| `sample` | `cpu`: process percent, or `null` before a CPU window; `rss`: bytes. |

Resource rows use the existing process sampler and continue while the reader
is idle. CPU counts all cores; values can exceed 100. Tracing starts a sampler
without enabling the `:perf` overlay. Native FIFO marks reach the session
thread before the writer records them. Scenario step identity remains separate
from frontend input counters.

Terminal `frame` timestamps end at output flush. Existing fields remain:
`build_ms`, `flush_ms`, `input_to_flush_ms`, `bytes`, `base_upload_bytes`,
`page`, `scroll`, and `complete`. Input-to-flush starts at the first consumed
input that changes a frame, and excludes terminal painting and physical input
latency. Terminal `input` rows currently count those changed typed inputs.
`missing` counts unavailable tile placements intersecting visible panes;
`blank` means `missing > 0`. These counters reset each frame and exclude
offscreen target tiles. `stand_ins` is zero because terminal tiles have no
scale fallback. They describe emitted content, excluding terminal painting.

Native `frame` timestamps start at CPU paint. Existing `paint_ms`, `fresh`,
`upload_bytes`, `stand_ins`, and `missing` remain. `blank` means `missing > 0`.
`build_ms` measures session construction, and `built_ms` uses the trace clock.
`busy` covers animation and active search, crop, outline, and OCR jobs.
`complete` comes from the session frame and measures render readiness. It does
not imply that no scale stand-ins remain. Native `input` rows currently count
adapted window events and file drops. Paint timing ends before GPU submission;
it does not measure GPU completion or observed display latency. An unwritable
path disables native tracing with a warning rather than stopping the app.

Publish measurements with repeated runs, sample counts, and percentile
summaries. Label CPU frame construction, render readiness,
input-consumption-to-flush, and observed display behavior separately. A speedup
needs a comparable baseline under the same conditions. Keep benchmark JSON,
input hashes, cache conditions, and build identity in an immutable,
commit-identified evidence bundle rather than a moving development release.

## Gallery capture

The [demo backlog][demo-backlog] lists workflows awaiting new captures.

The [capture script][demo-script] drives the real reader inside an isolated
[Kitty][kitty-capture] instance. It captures the terminal's rendered graphics,
including PDF images, through Kitty's screenshot API. Install Kitty using its
[upstream instructions][kitty-install]; both `kitty` and `kitten` must be on
`PATH`, and `kitten @ screenshot --help` must succeed. Also install Python 3
and [ImageMagick][imagemagick] with JPEG support and the `magick` command.
The background window runner currently requires a macOS desktop session. Kitty's
screenshot API does not require screen-recording permission; interaction video
capture does. See [Demo media publication][demo-media] for video setup.

OCR scenes also need [Tesseract and English language data][ocr-setup] and
[Poppler][poppler]'s `pdftotext` to verify that the scanned input has no
embedded text. On macOS, install the capture helpers with
`brew install python imagemagick poppler tesseract`. For Linux corpus and OCR
checks, install `python3 poppler-utils tesseract-ocr tesseract-ocr-eng` with
`apt-get`; obtain ImageMagick from its installation guide. These capture helpers
need separate setup from `mise install`.

```sh
mise run demo
```

List scene names without opening a terminal, or repeat `--scene` to refresh a
subset:

```sh
python3 scripts/demo.py --list-scenes
mise run demo -- --scene mouse-menu
mise run demo -- --scene mouse-hints --scene mouse-region
```

Selected runs replace only those images and merge their records into the capture
manifest. Each scene references its own capture identity, so refreshing one
scene on a newer build preserves the provenance of the others. Completed scenes
save immediately; a later failure keeps their records. Failed input validation
or publication preserves the previous image and record. Omit `--scene` to
regenerate the full gallery.

The task builds the reader and fetches or verifies only the required
[public examples][corpus] before capturing. A custom `--corpus` directory uses
its existing PDFs. Use the script's `--help` for font, executable, output, and
individual scene overrides. For a custom executable, `--build-profile` records
its compiler profile; omitting it records an unknown profile. The manifest reads
the executable's version rather than attributing it to the checkout. Mouse
recipes send terminal pixel reports to the reader. Use the script's geometry and
placement options when a smaller visible window is needed. The script checks the
requested pixel dimensions without resizing screenshots. User terminal
configuration and saved reader state stay separate from the capture instance.

Screenshots and video posters share the [JPEG optimizer][demo-optimize]. It
verifies decoding at the original dimensions. JPEG compression is lossy; full
color resolution keeps colored text readable. The capture manifest stays in
`docs/assets/`; new JPEGs, raw PNGs, screen text, and terminal logs default to
host-local `~/.cache/vpdf/demo-media/<checkout>/` directories. Capture and
publication are separate; see [Demo media publication][demo-media]. The manifest
identifies the executable hash, commit, build profile, machine, terminal, font
settings, document hashes, and scene inputs. Inspect every image for complete
rendering and readable content before publishing. See the [gallery][demo] for
document attribution and capture scope.

The `test` task checks dependency selection, executable identity, and
publication failure handling without opening Kitty.

Keep published demonstrations in the [demo catalog][demo], with media on the
[closed hosting issue][demo-host]. The [publication workflow][demo-media]
separates offline capture from reviewed uploads, verifies anonymous bytes, and
retains superseded URLs. App icons and provenance stay in `docs/assets/`.
Individual images may also be reused in other documentation. Refresh images
when the showcased workflow changes. Keep PDFs, raw traces, and unedited
recordings outside git. For feature and performance videos, record actual
terminal graphics at normal speed and disclose
edits or time compression. Record the viewport, capture rate, cold and revisited
pages, cache conditions, and build identity with the demonstration. A video's
capture rate does not establish application frame rate or physical input
latency.

## Release decisions

[Packaging][package] stages a separate executable under `target/dist`,
respecting `CARGO_TARGET_DIR`. Linux packaging compresses it with UPX, verifies
the packed binary, and checks startup. macOS packaging retains the stripped
executable. Packaging leaves the Cargo build uncompressed.

The [release workflow][release] publishes a rolling development prerelease and
commit-specific snapshots after CI. It checks installation through mise before
updating the Homebrew tap. The rolling tag moves; use the snapshot identity when
a result needs a reproducible release reference.

[tasks]: ../../.config/mise.toml
[help-build]: help.md
[packages]: packages.md
[mise]: https://mise.jdx.dev/
[pkgconf]: https://formulae.brew.sh/formula/pkgconf
[build]: ../../scripts/build.sh
[build-cache]: ../../scripts/build_cache.py
[validation]: ../../scripts/validation.py
[build-worker]: ../../scripts/build_worker.py
[build-app]: ../../scripts/build-app.sh
[document-events]: https://docs.rs/fmv-macos-events/latest/fmv_macos_events/
[icon]: ../assets/vpdf.svg
[inject]: ../../crates/vpdf-gui/src/inject.rs
[scenario]: ../../src/scenario.rs
[apple-distribution]: https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution
[install]: ../../scripts/install.sh
[ci]: ../../.github/workflows/ci.yml
[hooks]: ../../prek.toml
[ruff]: https://docs.astral.sh/ruff/
[rumdl]: ../../.config/rumdl.toml
[corpus]: ../../scripts/corpus.sh
[corpus-tests]: ../../tests/it/corpus.rs
[pdf-corpus-tests]: ../../crates/vpdf-pdf/tests/corpus.rs
[micro]: ../../scripts/bench-micro.sh
[reader-bench]: ../../benches/reader.rs
[latency]: ../../benches/latency.rs
[stages]: ../../benches/stages.rs
[render-worker]: ../../crates/vpdf-pdf/src/jobs/render.rs
[startup]: ../../scripts/startup.py
[trace]: ../../src/trace.rs
[package]: ../../scripts/release.sh
[release]: ../../.github/workflows/release.yml
[demo-script]: ../../scripts/demo.py
[demo-optimize]: ../../scripts/demo-optimize.sh
[demo]: ../demos.md
[demo-backlog]: demo-backlog.md
[kitty-capture]: https://sw.kovidgoyal.net/kitty/remote-control/#kitten-screenshot
[imagemagick]: https://imagemagick.org/script/download.php
[git]: https://git-scm.com/install
[python]: https://www.python.org/downloads/
[xcode]: https://developer.apple.com/xcode/resources/
[llvm]: https://formulae.brew.sh/formula/llvm
[kitty-install]: https://sw.kovidgoyal.net/kitty/binary/
[poppler]: https://poppler.freedesktop.org/
[ocr-setup]: ../users/install.md#optional-helpers
[preview]: ../../crates/ui/src/preview.rs
[manuals]: ../../benches/manuals.rs
[bench-support]: ../../benches/support/mod.rs
[scenario-driver]: ../../benches/drive.rs
[live-driver]: ../../benches/live/drive.py
[ci-script]: ../../scripts/ci.py
[hook-script]: ../../scripts/hooks.py
[nextest-filters]: https://nexte.st/docs/filtersets/reference/
[prek-run]: https://prek.j178.dev/running-hooks/
[demo-media]: demo-media.md
[demo-host]: https://github.com/letientai299/vpdf/issues/9
