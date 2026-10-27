# Embedded help assets

The base manual comes from [authored Markdown][users]. Both frontends embed the
same PDF, topic manifest, and font notices through the shared reader library.
[Runtime composition][runtime] adds built-in references on a PDF worker. Both
frontends route help through the shared session.

## Build and check

Run `mise install` for the pinned [Typst compiler][typst]. Builds use its
bundled fonts, disable system fonts, clear Typst environment overrides, and need
no font downloads. The [renderer pin][renderer] records the compiler identity
and its bundled font source. [Font notices][notices] accompany the embedded PDF
and the app's resources.

```sh
mise run help:build
mise run help:check
mise run test:help
```

`test:help` also needs [QPDF][qpdf] and [Poppler][poppler] for independent
structure, font, and text checks. On macOS, use `brew install qpdf poppler`. The
native tests check outlines, internal destinations, reference placeholders, and
extracted text directly from the embedded bytes.

Terminal builds, native app builds, tests, and checks prepare assets before
compiling. Direct Cargo commands only consume matching prepared assets. When
assets are missing, stale, incomplete, or corrupt, Cargo fails with
`mise run help:build`. Run that command after editing manual inputs. Remove a
corrupt generation directory before rebuilding it; valid generations remain
immutable.

Outputs live in fingerprint directories under `target/help`, respecting
`CARGO_TARGET_DIR`. Use the same `CARGO_TARGET_DIR` for generation and Cargo,
including when passing Cargo's `--target-dir`. A clean source build generates
assets. Application execution needs neither source files nor generation tools.
`--version` verifies the bundled hashes and reports their shared identity.

## Delivery checks

After changing manual inputs or help navigation, run `mise run lint`,
`mise run test`, and `mise run test:help`. Check relative links after moving
guides. All pages under `docs/users/` belong to the chapter order; keep setup
and workflow guidance available in both the book and bundled help.

Build both packages with `mise run release` and `mise run build:app`. Copy them
and a verified corpus PDF to a directory outside the checkout. Open help without
source files, Typst, fonts, or Python on `PATH`. Verify rendering, topic
completion, internal links, selection/copy, search, annotations, and
close/return behavior in both frontends. Keep test windows visible without
activating or raising them; use the [background event workflow][events].

Observe repeated opens and configuration refreshes for memory and temporary-file
growth. Record cold asset generation, warm reuse, packaged sizes, binary
identity, and runtime context. Separate local results from CI and name platform
gaps. Build Linux in a container; copy only binaries and test PDFs to the host.

## Authoring

[Chapter ordering][book] names the Markdown files. Each file starts with its
title and a stable topic comment:

```markdown
# Navigation

<!-- help-topic:navigation -->
```

The comment stays hidden on GitHub. IDs and
titles must match the [topic registry][topics]; aliases stay with that registry.
Pagination comes from [Typst introspection][metadata], with zero-based pages and
positions in PDF points measured from the upper left.

The converter accepts paragraphs, second-level headings, numbered and bullet
lists, inline code, fenced code blocks, reference-style links, and footnotes.
Manual chapters contain no graphics. It rejects unsupported markup and renderer
diagnostics, including font warnings. Indent wrapped footnotes by four spaces
and list continuations to the item's text. Use reference-style relative Markdown
links between chapters and their sections; the converter resolves them to PDF
destinations. HTTPS links remain external. For a generated reference, link to
the relevant instructions on GitHub and add `<!-- help:name=topic_id -->` beside
its definition. The converter redirects that reference to the generated entry in
the reader. Direct `[name]: help:topic_id` links also work in bundled-only
inputs. Dynamic links retain `vpdf-help:topic_id` placeholders until runtime
composition resolves them.

Use `{{keys:action_id:mode}}` to insert the owning dispatcher's preferred
default shortcut. Label it as a default and link to the built-in reference. The
native-free [metadata exporter][exporter] reads the registry and binding
snapshot. The generator runs it explicitly for the host architecture, even when
Cargo targets another platform. The reader's build script uses the same metadata
function as a host build dependency. Neither route executes a target binary or
compiles the reader to export facts.

## Invalidation and packaging

The [generator][generator] fingerprints authored files, chapter ordering,
figures, topic and binding facts, templates, font and renderer pins, notices,
generator code, the lockfile, and the task configuration. Its receipt hashes
every required output. Generation uses a locked staging directory, checks inputs
again, and publishes the complete directory with one rename. Failure leaves
previous generations intact; they cannot satisfy a changed fingerprint.

The build script checks the receipt and facts without invoking Typst. Release
packaging also compares the staged executable's identity with current assets.
The font licenses stay available in the PDF and the binary; app packages also
carry a plain text copy.

## Runtime snapshots

Pass the running settings' `help_reference` snapshot and a canonical topic ID to
`manual::Runtime::request`. Capture the invoking pane before changing focus.
`poll` returns a composed document and its resolved destination. After normal
PDF navigation, capture `manual::Location::at` and pass it to `locate`.
`refresh` resolves that logical topic and anchor when pagination changes. If an
anchor disappears, refresh uses its topic's start. If the topic also disappears,
refresh uses `config`. `Ready::fallback` identifies the removed location.
Replaced jobs are canceled; their channels cannot deliver into the replacement
request.

The PDF cache uses manual identity. Generated entries
use built-in bindings and setting defaults, with no custom mappings, winning
sources, warnings, or captured pane values. Built-in keys, settings, and Ex
commands have separate sections. Key entries use the dispatcher's preferred
shortcut, omit encoding variants, and combine modes with the same behavior.
Outline visibility is separate from section placement: all key entries precede
settings and Ex commands. Mark setting, mark jumps, counts, and hint labels use
parameter entries; concrete key lookups share those destinations. Other
parameterized actions remain outside the outline. There is no dispatch appendix.

The structured writer embeds DejaVu Sans Mono without external tools. The
renderer pin records its source and hash. Authored and generated pages use the
fixed light colors for paper, commands, parameters, and links. Interface theme
changes reuse the help PDF. The view can apply `:recolor` like other PDFs;
its setting stays outside persisted document state.
Font notices follow the reference. Unsupported glyphs and controls use
reversible Unicode escapes; literal backslashes double.

The document owns a private temporary PDF because existing workers reopen paths.
Retain its `Arc` until every reader and worker using that path stops. Each
worker opens its own native handle. The final owner removes the backing file.
Search, outline, annotation, figure, and OCR disk caches exclude these files for
their lifetime. The public document identity is `manual::ID`; the temporary path
is internal. Help stays out of recent files, file watching, and saved document
state. Worker wake callbacks retain the backing owner until pending work
finishes.

## Session navigation

`:help [topic]`, `g?`, and F1 open a reusable document tab. `:help! [topic]`
opens beside the invoking pane; `Ctrl-w p` returns focus. Later lookups prefer
an existing help pane in the current tab, keeping its placement. Key queries
accept an explicit context, such as `search:<Tab>`. Ambiguous topics open a
persistent completion list with descriptions. Plain calls return to the
introduction. Lookup and completion use the invoking mode and applied
built-in binding map; failed lookup leaves the document and position intact.
Lookups within help retain bookmarks, search, jump history, and text preview
return behavior. Key lookup uses the current mode.

Splits share the composed document, with independent reading positions. Refresh
resolves logical anchors and clears page-based navigation and crops. Closing the
final help pane returns to the invoking pane when it exists, otherwise an
adjacent tab or the start screen.

Path copying reports `builtin:help`. Help supports region copying and OCR;
region saving, printing, disk reload, and persistent global marks are
unavailable. Open another document before using those path-based actions.

[users]: ../users/readme.md
[typst]: https://typst.app/open-source/
[renderer]: ../../scripts/help/renderer.json
[notices]: ../../scripts/help/notices/
[qpdf]: https://qpdf.readthedocs.io/en/stable/
[poppler]: https://poppler.freedesktop.org/
[book]: ../../scripts/help/book.json
[topics]: ../../crates/ui/src/topics.rs
[metadata]: https://typst.app/docs/reference/introspection/metadata/
[exporter]: ../../crates/ui/examples/help_metadata.rs
[generator]: ../../scripts/help_assets.py
[runtime]: ../../src/manual/runtime.rs
[events]: dev.md#native-window
