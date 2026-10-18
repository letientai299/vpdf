# Development

[Mise tasks][tasks] define the supported build and validation workflow. Keep
task names, tool versions, platform targets, and numerical budgets there or in
their owning source files.

## Build from source

Install [mise][mise] and the native build dependencies: a C/C++ compiler, Make,
and libclang. On macOS, install the Xcode command line tools and
[pkgconf][pkgconf], which locates the system zlib, and provide libclang if the
selected toolchain does not expose it to bindgen. On x86-64
Ubuntu:

```sh
sudo apt-get install build-essential libclang-dev musl-tools
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

Build on the target platform. Linux builds use musl and reject shared-library
dependencies. macOS builds embed MuPDF and its base14 fonts, link the system
zlib, and reject non-system dynamic libraries. These checks cover the
executable; optional helpers remain separate programs. [Build scripts][build]
and the [CI matrix][ci] define supported hosts.

## Validation

```sh
mise run hooks
mise run lint
mise run test
```

The lint task applies automatic fixes before checking for remaining issues;
inspect its diff before staging. [Prek hooks][hooks] run lint and ordinary tests
before a commit. If lint changes tracked files, stage the intended fixes and
retry.

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
hashes in the corpus script and [corpus assertions][corpus-tests].

Terminal behavior needs its own interactive checks. Native rendering or state
tests do not verify Kitty placement, tmux passthrough, clipboard policy, or
behavior after a child program uses the screen.

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

The [latency benchmark][latency] uses a live render worker to measure cold
opens, jumps, zooms, and paced scrolling, including tile readiness and blank
frames. The [stage benchmark][stages] separates page loading, text extraction,
display list construction, rasterization, and encoding.

The [startup benchmark][startup] runs inside the terminal under test. It
launches one or two executables on a relay of that terminal, alternating their
order per pair, and reports first frame, first page tile, and complete initial
view, with paired differences. Pass one executable for an A/A noise check and
`--trace` to compare the reader's own completion marker. Results, hashes, and
geometry go under `target/startup`.

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

[Tracing][trace] overwrites that file and records frame construction, flush
time, consumed-input-to-flush time, and base-image transfer accounting. It
records no document text, but includes page and scroll position. Timing starts
when the loop consumes input and ends at output flush; terminal painting and
physical input latency are outside that measurement.

Publish measurements with repeated runs, sample counts, and percentile
summaries. Label CPU frame construction, render readiness,
input-consumption-to-flush, and observed display behavior separately. A speedup
needs a comparable baseline under the same conditions. Keep benchmark JSON,
input hashes, cache conditions, and build identity in an immutable,
commit-identified evidence bundle rather than a moving development release.

## Gallery capture

The [capture script][demo-script] drives the real reader inside an isolated
[Kitty][kitty-capture] instance. It captures the terminal's rendered graphics,
including PDF images, through Kitty's screenshot API. It needs Python 3,
Kitty with that API, and [ImageMagick][imagemagick] with WebP support, plus a
desktop graphics session. It does not require screen-recording permission.
Captures that run OCR also need Tesseract with English language data and
Poppler's `pdftotext` to verify that the scanned input has no embedded text.

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
recipes send terminal pixel reports to the reader. The default capture uses QHD
dimensions and a font scaled to 200% of its base size. It rejects other pixel
dimensions instead of resizing the screenshot. User terminal configuration and
saved reader state stay separate from the capture instance.

The [optimizer][demo-optimize] encodes lossless WebP and checks decoded pixels
against the original PNG. Tracked images and their generated capture manifest
live in `docs/assets/`; raw PNGs, screen text, and terminal logs stay in
gitignored `tmp/demo/`. The manifest identifies the executable hash, commit,
build profile, machine, terminal, font settings, document hashes, and scene
inputs. Inspect every image for complete rendering and readable content before
publishing. See the [gallery][demo] for document attribution and capture scope.

Run `mise run test:demo` to check dependency selection, executable identity,
and publication failure handling without opening Kitty.

Refresh images when the showcased workflow changes. Keep PDFs, raw traces,
and large recordings outside git. For future feature and performance videos,
record actual terminal graphics at normal speed and disclose edits or time
compression. Record the viewport, capture rate, cold and revisited pages,
cache conditions, and build identity with the demonstration. A video's capture
rate does not establish application frame rate or physical input latency.

## Release decisions

[Packaging][package] stages a separate executable under `target/dist`,
respecting `CARGO_TARGET_DIR`. Linux packaging compresses it with UPX, verifies
the packed binary, and checks startup. macOS packaging retains the stripped
executable. Packaging leaves the Cargo build uncompressed.

The [release workflow][release] publishes a rolling development prerelease and
commit-specific snapshots after CI. It checks installation through mise before
updating the Homebrew tap. The rolling tag moves; use the snapshot identity when
a result needs a reproducible release reference.

[tasks]: ../mise.toml
[mise]: https://mise.jdx.dev/
[pkgconf]: https://formulae.brew.sh/formula/pkgconf
[build]: ../scripts/build.sh
[install]: ../scripts/install.sh
[ci]: ../.github/workflows/ci.yml
[hooks]: ../prek.toml
[rumdl]: ../rumdl.toml
[corpus]: ../scripts/corpus.sh
[corpus-tests]: ../tests/it/corpus.rs
[reader-bench]: ../benches/reader.rs
[latency]: ../benches/latency.rs
[stages]: ../benches/stages.rs
[startup]: ../scripts/startup.py
[trace]: ../src/trace.rs
[package]: ../scripts/release.sh
[release]: ../.github/workflows/release.yml
[demo-script]: ../scripts/demo.py
[demo-optimize]: ../scripts/demo-optimize.sh
[demo]: demo.md
[kitty-capture]: https://sw.kovidgoyal.net/kitty/remote-control/#kitten-screenshot
[imagemagick]: https://imagemagick.org/
