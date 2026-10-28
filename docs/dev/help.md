# Embedded help assets

The base manual comes from [authored Markdown][users]. Both frontends embed the
same PDF, topic manifest, and font notices through the shared reader library.
[Runtime composition][runtime] adds built-in references on a PDF worker. Both
frontends route help through the shared session.

## Build and check

Run `mise install` for the pinned [Pandoc parser][pandoc], [Typst
compiler][typst], and [QPDF][qpdf] for lossless PDF compression. Builds use
vendored fonts, disable system fonts, clear Typst environment overrides, and
need no font downloads. The [renderer pin][renderer] records tool identities and
font hashes. Short legal notices appear in the PDF. Full [font notices][notices]
accompany the binary and app resources.

```sh
mise run help:build
mise run help:check
mise run test:help
```

`test:help` also needs [Poppler][poppler] for independent font and text checks.
On macOS, use `brew install poppler`. The
native tests check outlines, internal destinations, reference placeholders, and
extracted text directly from the embedded bytes.

Terminal builds, native app builds, tests, and checks prepare assets before
compiling. Direct Cargo commands only consume matching prepared assets. When
assets are missing, stale, incomplete, or corrupt, Cargo fails with
`mise run help:build`. Run that command after editing manual inputs. Remove a
corrupt generation directory before rebuilding it; valid generations remain
immutable.

Outputs live in fingerprint directories under `target/help`, respecting
`CARGO_TARGET_DIR`. Builds export the readable, composed manual to
`target/help/help.pdf`, including built-in references and resolved links. The
fingerprinted `base.pdf` contains placeholder colors for runtime composition.
Use the same `CARGO_TARGET_DIR` for generation and Cargo,
including when passing Cargo's `--target-dir`. A clean source build generates
assets. Application execution needs neither source files nor generation tools.
`--version` verifies the bundled hashes and reports their shared identity.

## Delivery checks

After changing manual inputs or help navigation, run `mise run lint`,
`mise run test`, and `mise run test:help`. Check relative links after moving
guides. All pages under `docs/users/` belong to the section order; keep setup
and workflow guidance available in both the manual and bundled help.

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

[Section ordering][contents] names the Markdown files. Each file starts with its
title and a stable topic comment:

```markdown
# Navigation

<!-- help-topic:navigation -->
```

The comment stays hidden on GitHub. IDs and
titles must match the [topic registry][topics]; aliases stay with that registry.
Pagination comes from [Typst introspection][metadata], with zero-based pages and
positions in PDF points measured from the upper left.

Pandoc parses [GitHub Flavored Markdown][gfm], including tables, nested lists,
task lists, strikethrough, autolinks, images, HTML, and footnotes. Its Typst
writer preserves heading levels and fenced-code languages. HTML becomes static
content; vendor images under `docs/users/` or `docs/assets/` so their bytes
participate in the build fingerprint. Typst highlights TOML
and flows sections continuously with justified prose. Tables and code retain
their alignment. Diagnostics, including font warnings, fail generation.
Indent wrapped footnotes by four spaces
and list continuations to the item's text. Use reference-style relative Markdown
links between sections and their sections; the converter resolves them to PDF
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

The [generator][generator] fingerprints authored files, section ordering,
figures, topic and binding facts, templates, font and renderer pins, notices,
generator code, the lockfile, and the task configuration. Its receipt hashes
every required output. Generation uses a locked staging directory, checks inputs
again, and publishes the complete directory with one rename. Failure leaves
previous generations intact; they cannot satisfy a changed fingerprint.

The build script checks the receipt and facts without invoking Typst. Release
packaging also compares the staged executable's identity with current assets.
The PDF links to license texts. The binary embeds full notices; app packages
also carry a plain text copy.

## Runtime snapshots

Pass the running settings' `help_reference` snapshot and a `manual::Location` to
`manual::Runtime::request`. Topic lookups use `Location::topic`; key lookups
keep their action, context, and sequence and use `Location::anchored` with
`topics::key_anchor`. Setting lookups anchor within `config`. Capture the
invoking pane before changing focus. `poll` returns a composed document and its
resolved destination. After normal PDF navigation, capture
`manual::Location::at` and pass it to `locate`. `refresh` resolves that logical
topic and anchor when pagination changes. If an anchor disappears, open and
refresh use its topic's start. If the topic also disappears, refresh uses
`config`. `Ready::fallback` identifies the removed location. Replaced jobs are
canceled; their channels cannot deliver into the replacement request.

The PDF cache uses manual identity. Generated entries
use built-in bindings and setting defaults, with no custom mappings, winning
sources, warnings, or captured pane values. Built-in keys, configuration
actions, settings, and Ex commands have separate sections. Key entries use the
dispatcher's preferred shortcut, omit encoding variants, and combine modes with
the same behavior. One entry holds each key; differing effects carry mode
labels. Operator and object continuations fold into their full sequence. Every
key row has an anchor per context and sequence, including hidden aliases.
Parameter families have one template row; their keys land on its topic.

[Action metadata][catalog] lists each parser-accepted action family with
fields and a TOML example; tests parse every example against the
configuration parser. [Setting metadata][fields] supplies each setting's
value, precedence, and reload text.

Outline visibility is separate from section placement: all key entries precede
settings and Ex commands. Every key entry with a default binding has an outline
heading, so the status line names the landed entry. Mark setting, mark jumps,
counts, and hint labels use parameter entries; concrete key lookups share those
destinations. There is no dispatch appendix.

The manual, generated reference, and native interface embed Vpdf Mono, derived
from [Iosevka Nerd Font Mono][nerd-font]. The native app requires no font
installation. Iosevka uses the SIL Open Font License; icon notices retain
their upstream attributions and licenses. The terminal interface uses the
terminal's configured font.

The [font preparation script][font-script] removes unused shaping glyphs and
renames the derivative. The note symbol aliases the Nerd Font pencil. It
preserves the full Unicode map for the native UI and authored manual; a text
subset keeps the runtime reference smaller. Normal builds consume the checked-in
files without FontTools. To reproduce them, download the regular, bold, italic,
and bold italic sources recorded in the renderer pin into one directory and run
the script with [uv][uv] and the pinned FontTools version:

```sh
uv run --no-project --with fonttools==4.61.1 python -B -m tools.help.font SOURCE.ttf
```

Reference descriptions align within each section, including wrapped lines.
Blank lines separate entries. Long labels occupy their own line.

Authored and generated pages use the fixed light colors for paper, commands,
parameters, and links. Interface theme changes reuse the help PDF. The view can
apply `:recolor` like other PDFs; its setting stays outside persisted document
state. Short legal notices follow the reference; linked license texts stay
outside the PDF. Unsupported glyphs and controls use reversible Unicode escapes;
literal backslashes double.

The document owns a private temporary PDF because existing workers reopen paths.
Retain its `Arc` until every reader and worker using that path stops. Each
worker opens its own native handle. The final owner removes the backing file.
Search, outline, annotation, figure, and OCR disk caches exclude these files for
their lifetime. The public document identity is `manual::ID`; the temporary path
is internal. Help stays out of recent files, file watching, and saved document
state. Worker wake callbacks retain the backing owner until pending work
finishes.

## Session navigation

`:help [topic]`, `g?`, and F1 replace the focused pane and keep the replaced
document as its alternate. `:help! [topic]` opens beside the invoking pane;
`Ctrl-w p` returns focus. Later lookups prefer an existing help pane in the
current tab, keeping its placement. Key queries accept an explicit context, such
as `search:<Tab>`. Ambiguous topics open a persistent completion list with
descriptions. Plain calls return to the introduction. Lookup and completion use
the invoking mode and applied built-in binding map; failed lookup leaves the
document and position intact. Lookups within help retain bookmarks, search, jump
history, and text preview return behavior. Key lookup uses the current mode.

Splits share the composed document, with independent reading positions. Refresh
resolves logical anchors and clears page-based navigation and crops. Closing
help reopens its alternate. Without one, closing returns to the invoking pane
when it exists; otherwise a lone help pane becomes Recent.

Path copying reports `builtin:help`. Help supports region copying and OCR;
region saving, printing, disk reload, and persistent global marks are
unavailable. Open another document before using those path-based actions.

[users]: ../users/readme.md
[typst]: https://typst.app/open-source/
[pandoc]: https://pandoc.org/installing.html
[gfm]: https://github.github.com/gfm/
[nerd-font]: https://github.com/ryanoasis/nerd-fonts/tree/v3.5.1/patched-fonts/Iosevka
[font-script]: ../../tools/help/font.py
[uv]: https://docs.astral.sh/uv/getting-started/installation/
[renderer]: ../../tools/help/help/renderer.json
[notices]: ../../tools/help/help/notices/
[qpdf]: https://qpdf.readthedocs.io/en/stable/
[poppler]: https://poppler.freedesktop.org/
[contents]: ../../tools/help/help/contents.json
[topics]: ../../crates/ui/src/topics.rs
[metadata]: https://typst.app/docs/reference/introspection/metadata/
[exporter]: ../../crates/ui/examples/help_metadata.rs
[generator]: ../../tools/help/assets.py
[runtime]: ../../src/manual/runtime.rs
[catalog]: ../../crates/ui/src/bindings/catalog.rs
[fields]: ../../src/config/fields.rs
[events]: dev.md#native-window

## Dependency notices

The PDF links to full notices instead of reproducing license texts.
`third-party-notices.txt` embeds those texts in both binaries and accompanies
app resources. It includes font, theme, Rust dependency, and MuPDF notices.

After changing `Cargo.lock`, regenerate notices with [cargo-about][cargo-about]
0.9.3 on the maintainer machine. The tool needs network access to retrieve
licenses omitted from published crates. Run through Mise's optional Cargo tool:

```sh
mise exec cargo:cargo-about@0.9.3 -- python3 -B -m tools.help.licenses
```

The inventory includes all workspace features and platform dependencies.
MuPDF's bundled components retain their native notices separately. Help tests
reject inventories from a different lockfile.

[cargo-about]: https://embarkstudios.github.io/cargo-about/
