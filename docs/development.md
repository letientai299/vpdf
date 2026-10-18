# Development

[Mise tasks][tasks] define the supported build and validation workflow. Keep
task names, tool versions, platform targets, and numerical budgets there or in
their owning source files.

## Build from source

Install [mise][mise] and the native build dependencies: a C/C++ compiler, Make,
and libclang. On macOS, install the Xcode command line tools and provide
libclang if the selected toolchain does not expose it to bindgen. On x86-64
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
dependencies. macOS builds embed MuPDF and its base14 fonts and reject
non-system dynamic libraries. These checks cover the executable; optional
helpers remain separate programs. [Build scripts][build] and the [CI matrix][ci]
define supported hosts.

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
