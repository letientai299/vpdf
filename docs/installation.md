# Installation and terminal setup

Prebuilt builds embed MuPDF and the PDF base14 fonts. Python is not a runtime
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
exits naming each one missing. `:text` shows extracted text instead of page
images on such a terminal; other terminals cannot use it either. The file
picker's fzf previews cannot ask the terminal, so the reader passes on what it
found: an image with Unicode placeholders where the terminal draws them, else
text. The [CLI][cli] owns launch options; use `vpdf --help` for the installed
interface.

The reader, in either presentation, requires the [Kitty keyboard
protocol][keyboard]'s disambiguation, so Tab and Ctrl-i stay distinct and Esc
needs no timeout. Without it the reader exits and names what is missing.
WezTerm needs [`enable_kitty_keyboard = true`][wezterm-keyboard].

tmux ignores the protocol's query. Under tmux 3.2 or later the reader needs
`set -s extended-keys on`, asks for [modifyOtherKeys][modify-keys] instead, and
waits briefly after a lone Esc. Kitty ignores modifyOtherKeys, so through tmux
in Kitty Ctrl-i arrives as Tab ([tmux modifier keys][tmux-keys]). While it
runs, the reader sets `allow-passthrough` for its own pane: `all` with
`set -s focus-events on`, so it can remove its images while tmux shows
something else, else `on`, and leaves `on` at exit, as `kitten icat` does.
tmux tells an inactive pane nothing about a popup over it, so that pane's
images stay over the popup.

Image placement depends on the terminal's cell metrics. Reader panes work inside
one terminal; a multiplexer is optional. Actual painting cadence depends on the
terminal's rendering settings and display, even when the reader schedules frames
more frequently. See [rendering][rendering] for transport and scheduling
choices.

## Optional helpers

Install Tesseract and the requested language data to recognize scanned pages.
Existing PDF text, including an embedded OCR layer, does not need Tesseract.

PDFs that name a font without embedding it draw with installed fonts: Core
Text finds them on macOS, and [fontconfig][fontconfig]'s `fc-match` on Linux.
Without a match, Latin text falls back to a base14 font and CJK text draws
wrong glyphs.

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

The base14 fonts inside MuPDF retain their upstream licenses. Consult
[the manifest][manifest] and [lockfile][lockfile] when identifying the
dependencies used in a particular build.

[homebrew]: https://brew.sh/
[mise]: https://mise.jdx.dev/
[release]: ../.github/workflows/release.yml
[development]: development.md#build-from-source
[kitty]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
[keyboard]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
[wezterm-keyboard]: https://wezterm.org/config/lua/config/enable_kitty_keyboard.html
[modify-keys]: https://invisible-island.net/xterm/modified-keys.html
[tmux-keys]: https://github.com/tmux/tmux/wiki/Modifier-Keys
[cli]: ../src/main.rs
[rendering]: rendering.md
[integrations]: sessions.md#external-actions
[fontconfig]: https://www.freedesktop.org/wiki/Software/fontconfig/
[license]: ../LICENSE
[agpl]: ../LICENSE-AGPL
[mupdf-license]: https://mupdf.readthedocs.io/en/latest/license.html
[manifest]: ../Cargo.toml
[lockfile]: ../Cargo.lock
