# Installation and terminal setup

Prebuilt builds embed MuPDF and fallback fonts. Python is not a runtime
dependency. OCR and desktop integrations use optional external programs.

## Distribution

Install a snapshot with [Homebrew][homebrew]:

```sh
brew install letientai299/tap/vpdf
vpdf document.pdf
```

The tap follows published snapshots from successful `main` builds. Use
`brew update` and `brew upgrade vpdf` to refresh an installation.

With [mise][mise], install the rolling development prerelease:

```sh
mise i github:letientai299/vpdf@dev
mise exec github:letientai299/vpdf@dev -- vpdf document.pdf
```

Use `mise use -g github:letientai299/vpdf@dev` to activate it globally, or
`mise i --force github:letientai299/vpdf@dev` to fetch a newer rolling build.
Development snapshots can introduce breaking changes.

The [release workflow][release] defines the published platform matrix and update
policy. For other build environments, see [building from source][development].

## Terminal setup

The reader requires the [Kitty graphics][kitty] it draws with, Kitty keyboard
disambiguation, and the cell size in pixels; it asks the terminal at start and
exits naming each one missing. `--text` shows extracted text instead of page
images on such a terminal; other terminals cannot use it either. `vpdf
preview`, which the file picker runs under fzf, cannot ask: it prints an image
with Unicode placeholders where `TERM` or `TERM_PROGRAM` names a terminal that
draws them, else text. The reader tells the picker's previews what it found;
elsewhere, such as under tmux or with an overridden `TERM`, pass
`--graphics kitty`. The [CLI][cli] owns launch options; use `vpdf --help` for
the installed interface.

The reader, in either presentation, requires the [Kitty keyboard
protocol][keyboard]'s disambiguation, so Tab and Ctrl-i stay distinct and Esc
needs no timeout. Without it the reader exits and names what is missing.
WezTerm needs [`enable_kitty_keyboard = true`][wezterm-keyboard]. tmux ignores
the protocol's query, so the reader exits under tmux.

Image placement depends on the terminal's cell metrics. Reader panes work inside
one terminal; a multiplexer is optional. Actual painting cadence depends on the
terminal's rendering settings and display, even when the reader schedules frames
more frequently. See [rendering][rendering] for transport and scheduling
choices.

## Optional helpers

Install Tesseract and the requested language data to recognize scanned pages.
Existing PDF text, including an embedded OCR layer, does not need Tesseract.

The file picker uses `fzf` when it is on `PATH`. Native clipboard delivery can
use the platform's clipboard helper; OSC 52 remains subject to terminal policy.
See `:help` for commands available in the current environment and
[integration behavior][integrations] for clipboard and link boundaries.

## Licensing

vpdf's original source code uses the [MIT license][license]. Dependencies
retain their own licenses.

MuPDF and its Rust bindings, `mupdf` and `mupdf-sys`, use AGPLv3 in the current
dependency stack. Distributed executables that link them remain subject to AGPL
obligations, including corresponding-source requirements. Licensing vpdf's code
under MIT does not relicense these dependencies. The [AGPL text][agpl] is
retained for them; see also [MuPDF licensing][mupdf-license].

Bundled fallback fonts retain upstream licenses in the dependency source
distributions. Consult [the manifest][manifest] and [lockfile][lockfile] when
identifying the dependencies used in a particular build.

[homebrew]: https://brew.sh/
[mise]: https://mise.jdx.dev/
[release]: ../.github/workflows/release.yml
[development]: development.md#build-from-source
[kitty]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
[keyboard]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
[wezterm-keyboard]: https://wezterm.org/config/lua/config/enable_kitty_keyboard.html
[cli]: ../src/main.rs
[rendering]: rendering.md
[integrations]: sessions.md#external-actions
[license]: ../LICENSE
[agpl]: ../LICENSE-AGPL
[mupdf-license]: https://mupdf.readthedocs.io/en/latest/license.html
[manifest]: ../Cargo.toml
[lockfile]: ../Cargo.lock
