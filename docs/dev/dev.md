# Development

[Mise tasks][tasks] define the supported build and validation workflow. Keep
task names, tool versions, platform targets, and numerical budgets there or in
their owning source files.

## Build from source

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
mise run build
mise run install
```

The [build script][build] stages the optimized executable at
`target/release/vpdf`, or under `CARGO_TARGET_DIR` when set. The
[install script][install] atomically replaces `~/.local/bin/vpdf`; put that
directory on `PATH`. The install task depends on the build task, so it can also
be run directly after setup.

On macOS, build, install, and release run `--version` at the final executable
path. This pays first-launch security checks during the task before interactive
use; macOS can repeat checks after security updates.

Build on the target platform. Linux builds use musl and reject shared-library
dependencies. macOS builds embed MuPDF and its base14 fonts, link the system
zlib, and reject non-system dynamic libraries. These checks cover the
executable; optional helpers remain separate programs. [Build scripts][build]
and the [CI matrix][ci] define supported hosts.

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
command line tools. The DMG uses LZFSE compression and includes an
Applications shortcut. `install:app` builds the package and atomically replaces
`~/Applications/Vpdf.app` without launching it. `test:app` checks the mounted
DMG and installation in a temporary home directory.

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
uncommitted until confirmation; candidate windows follow the prompt caret.
Alt chords retain the terminal's word-editing behavior.
Region captures use the shared jobs and clipboard helpers. Completed text from
background panes reaches the GUI clipboard when focus moves elsewhere.
On macOS, UI text uses installed system fonts for missing CJK glyphs.
Protected documents use the shared masked password prompt. Cancel keeps the
previous view; retry resumes the original open, split, or mark command.
Switching windows preserves committed passwords and cancels IME composition.
On macOS, `:fzf` uses the system file chooser through the built-in `osascript`;
it starts in the current directory and replaces the focused pane. File drops
open tabs. Closing the last tab exits, as in the terminal. External prompt
editing requires the terminal reader.

On macOS, `--background` disables automatic activation; combine it with
`open -g` when launching the app bundle. macOS delivers posted keys to a
background app but drops posted clicks and wheel turns. For live checks without
focus, the hidden `--events PATH` option reads keys, window geometry, marks,
pointer, wheel, modifier, focus, IME, paste, and file-drop lines from a file or
FIFO. Production builds reject the option and omit the reader and injection
hooks. The non-default `bench-events` Cargo feature enables them:

```sh
VPDF_BENCH_EVENTS=1 mise run install
VPDF_BENCH_EVENTS=1 mise run install:app
```

Both install tasks build production binaries when `VPDF_BENCH_EVENTS` is unset
or `0`; CI explicitly uses `0`. Feature-enabled binaries report `[bench-events]`
in `--version`; retain this marker in benchmark provenance.
`mise run test:events` checks a separate feature-enabled build; CI runs it
before packaging production binaries. Reinstall without
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

Checks cover every workspace member. Support crates own their unit tests;
package selection avoids compiling the application and its native dependencies
when the selected crate does not need them:

```sh
mise run test:model
mise run test:search
mise run test:pdf
mise run test:protocol
mise run test:terminal
mise run test:ui
mise run test:reader
mise exec -- cargo nextest run --locked -p vpdf-model text::tests
mise exec -- cargo nextest run --locked -p vpdf-search search::tests
```

The [package guide][packages] describes ownership and permitted dependencies.
`check`, `lint`, and `test` also run `check:packages`, which validates all
dependency kinds and transitive native isolation. Run `test:packages` for the
verifier's regression tests.

PDF tests require MuPDF; ignored corpus tests also require the verified
public PDFs from `mise run corpus`. Run them independently:

```sh
mise exec -- cargo nextest run --locked -p vpdf-pdf --test corpus --run-ignored only
```

Root modules retain the application compilation unit and support test filters:

```sh
mise exec -- cargo nextest run --locked -p vpdf --lib session::tests
mise exec -- cargo nextest run --locked -p vpdf-ui --lib layout::tests
```

The lint task applies automatic fixes before checking for remaining issues;
inspect its diff before staging. [Prek hooks][hooks] run lint and ordinary tests
before a commit. If lint changes tracked files, stage the intended fixes and
retry.

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
inputs; standard tasks do not consume them automatically. Keep download URLs and
hashes in the corpus script. [Application corpus tests][corpus-tests] cover
integrated behavior; [PDF corpus tests][pdf-corpus-tests] cover native
extraction and rasterization.

Terminal behavior needs its own interactive checks. Native rendering or state
tests do not verify Kitty placement, tmux passthrough, clipboard policy, or
behavior after a child program uses the screen. [Preview composition
checks][preview] cover centering and failures on later pages; live checks must
still verify placement and terminal graphics.

## Performance validation

Measure CPU work separately from terminal painting and end-to-end input latency.
The [warm reader benchmark][reader-bench] covers extraction, navigation, search,
and serialized frames. Its budget task enforces CPU frame and motion limits
and base-image reuse; the thresholds live with the assertions.

```sh
mise run bench:budget
mise run bench:latency
mise run bench:stages
```

The [scenario driver][scenario-driver] runs the real session without a
window. Scenario headers pin PDF hashes, layout, and crop mode. Run
`mise run bench:drive -- benches/scenarios/keys.txt`; results and traces stay
in the host cache. The catalog defaults to `sdm.pdf`; `corpus:sdm` fetches and
verifies it. Use `--huge DIRECTORY` for an additional large PDF with its own
hash-pinned scenario. Headless delivery measures frame collection, excluding
GPU painting and terminal output. The general latency cases use this bridge;
crop measurements retain their component loop. Enqueue time measures input
submission, rather than the former direct action CPU time.

Summarize marks with `mise run bench:report -- TRACE --json REPORT`. CPU means
use the sampler's rolling percentages. A `clock` row anchors reader timestamps
to Unix time for launch timing. Sharp-after measures the first complete frame
without stand-ins or active jobs after a mark. It does not measure observed
display latency. Missing fidelity counters remain unknown.
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
Capture needs a desktop graphics session, without screen-recording permission.

OCR scenes also need [Tesseract and English language data][ocr-setup] and
[Poppler][poppler]'s `pdftotext` to verify that the scanned input has no
embedded text. On macOS, install the capture helpers with
`brew install python imagemagick poppler tesseract`. On Ubuntu, install
`python3 poppler-utils tesseract-ocr tesseract-ocr-eng` with `apt-get`; obtain
ImageMagick with the `magick` command from its linked installation guide. These
capture helpers need separate setup from `mise install`.

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
recipes send terminal pixel reports to the reader. The script defines the
required pixel dimensions and exposes font controls through `--help`. It rejects
other pixel dimensions instead of resizing the screenshot. User terminal
configuration and saved reader state stay separate from the capture instance.

The [optimizer][demo-optimize] writes optimized JPEG and verifies that it
decodes at the original dimensions. JPEG compression is lossy; full color
resolution keeps colored text readable. Tracked images and their generated
capture manifest live in `docs/assets/`; raw PNGs, screen text, and terminal
logs stay in gitignored `tmp/demo/`. The manifest identifies the executable
hash, commit, build profile, machine, terminal, font settings, document hashes,
and scene inputs. Inspect every image for complete rendering and readable
content before publishing. See the [gallery][demo] for document attribution and
capture scope.

Run `mise run test:demo` to check dependency selection, executable identity,
and publication failure handling without opening Kitty.

Keep all published image and future video demos in the [demo catalog][demo],
with their media under `docs/assets/`. Individual images may also be reused in
other documentation. Refresh images when the showcased workflow changes. Keep
PDFs, raw traces, and unedited recordings outside git. For future feature and
performance videos, record actual terminal graphics at normal speed and disclose
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
[packages]: packages.md
[mise]: https://mise.jdx.dev/
[pkgconf]: https://formulae.brew.sh/formula/pkgconf
[build]: ../../scripts/build.sh
[build-app]: ../../scripts/build-app.sh
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
[ocr-setup]: ../install.md#optional-helpers
[preview]: ../../crates/ui/src/preview.rs
[manuals]: ../../benches/manuals.rs
[bench-support]: ../../benches/support/mod.rs

[scenario-driver]: ../../benches/drive.rs
[live-driver]: ../../benches/live/drive.py
