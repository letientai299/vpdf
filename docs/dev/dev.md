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

Use [Benchmarking][bench] for measurement boundaries, targeted harnesses,
fixture preparation, and comparable runs. Keep setup controls and case details
in the harness scripts and their executable help.

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
[install]: ../../scripts/install.sh
[ci]: ../../.github/workflows/ci.yml
[hooks]: ../../prek.toml
[rumdl]: ../../.config/rumdl.toml
[corpus]: ../../scripts/corpus.sh
[corpus-tests]: ../../tests/it/corpus.rs
[pdf-corpus-tests]: ../../crates/vpdf-pdf/tests/corpus.rs
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
[bench]: bench.md
