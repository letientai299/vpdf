# Installation and setup

<!-- help-topic:install -->

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

The terminal reader runs in a compatible terminal. The macOS native app uses
its own window and does not require terminal graphics or keyboard negotiation.
Both frontends share reading commands and configuration. For source builds,
see the online [development guide][development].

## Launching a reader

Run `vpdf book.pdf` to open a book; pass more filenames for more tabs.
`vpdf 'book.pdf#page=23'` starts at physical page 23. Omit filenames for the
start screen. The native app opens PDFs from Finder or drag and drop.
To make it the default reader, select a PDF in Finder, choose Get Info,
select Vpdf under Open with, then click Change All.
`--help` and `--version` print information and exit. `-u PATH` (or
`--config PATH`) selects one TOML file; `-u NONE` uses built-in settings when
`NONE` is not a readable file. See the [configuration defaults][defaults]
before remapping keys.

## Terminal setup

The interactive reader requires [Kitty graphics][kitty], keyboard
disambiguation, and cell dimensions in pixels. It probes these at startup and
names any missing requirements before exiting. `:text` changes the presentation
to extracted text; it requires the same terminal capabilities. If using tmux,
follow its [installation instructions][tmux-install] too.

The file picker's fzf previews use the capabilities detected by the reader:
images with [Unicode placeholders][placeholders] where supported, otherwise
text. Use `vpdf --help` for the installed launch interface.

The reader, in either presentation, requires the
[Kitty keyboard protocol][keyboard]'s disambiguation, so Tab and Ctrl-i stay
distinct and Esc needs no timeout. Without it the reader exits and names what is
missing. Kitty, Ghostty, Rio, and iTerm2 have been exercised with the reader's
demo workflows on macOS. WezTerm requires the settings below. Use a release
accepted by the reader's startup check. Terminal names alone do not establish
support; the startup diagnostic names missing requirements.

In WezTerm's existing [Lua configuration][wezterm-config], set:

```lua
config.enable_kitty_keyboard = true
config.enable_kitty_graphics = true
```

Here `config` is the table returned by the configuration file. The [keyboard
option][wezterm-keyboard] needs explicit enabling. Graphics is enabled by
default in the tested release; keep it enabled if customizing the config.
Restart WezTerm after changing these settings. Other terminals need both
protocols and pixel cell dimensions; graphics support alone is insufficient.

tmux ignores the protocol's query. Under tmux 3.2 or later the reader needs
`set -s extended-keys on`, asks for [modifyOtherKeys][modify-keys] instead, and
waits briefly after a lone Esc. Kitty ignores modifyOtherKeys, so through tmux
in Kitty Ctrl-i arrives as Tab. See the [tmux manual][tmux-manual] for current
options. While it runs, the reader sets `allow-passthrough all` for its own pane
so it can remove images while hidden, then leaves `on` at exit, as `kitten icat`
does. `set -s focus-events on` gives prompt cleanup; background polling covers
missed focus events and terminal client changes. tmux tells an inactive pane
nothing about a popup over it, so that pane's images stay over the popup.

## Alt-drag under tmux

tmux can bind Alt-drag to moving floating panes before the reader sees it. If
`tmux list-keys -T root M-MouseDrag1Pane` shows `move-pane -M`, this optional
replacement forwards capture drags while the application requests mouse input
and keeps that pane-movement action otherwise:

```tmux
bind-key -n M-MouseDrag1Pane if-shell -F '#{mouse_any_flag}' 'send-keys -M' 'move-pane -M'
```

Apply it to the intended server or add it to your existing tmux configuration.
Inspect and preserve any different existing binding. The reader never changes
the root key table. Forwarding the drag alone suffices with the default press
and release handling; leave those bindings unchanged. `:mouse off` restores
tmux's pane-movement branch.

Image placement depends on the terminal's cell metrics. Reader panes work inside
one terminal; a multiplexer is optional. Actual painting cadence depends on the
terminal's rendering settings and display, even when the reader schedules frames
more frequently. Use `:info` to inspect the detected terminal and mouse state.

## Optional helpers

Install [Tesseract][tesseract] and the requested language data to recognize
scanned pages. Its repository links to installation and language-data setup
instructions. Ensure `tesseract` is on `PATH`; use `tesseract --list-langs` to
check available languages. Existing PDF text, including an embedded OCR layer,
does not need Tesseract.

PDFs that name a font without embedding it draw with installed fonts: Core Text
finds them on macOS, and [fontconfig][fontconfig]'s `fc-match` on Linux. On
Linux, install your distribution's `fontconfig` package and the fonts your PDFs
need. Without a matching font, Latin text falls back to a base14 font and CJK
text draws wrong glyphs.

Install other helpers only for the integrations you use, and keep their
executables on `PATH`:

- File picker: install [fzf][fzf] using its upstream installation instructions.
- Native Linux clipboard: install [wl-clipboard][wl-clipboard] (`wl-copy`) for
  Wayland, or [xclip][xclip] for X11. [xsel][xsel] is a text-only X11 fallback;
  image copying needs `xclip`.
- Opening external links on Linux: install your distribution's `xdg-utils`
  package for `xdg-open` and configure a default browser or mail application.
- Prompt editing: install a terminal editor such as [Vim][vim]. Set `VISUAL` or
  `EDITOR` to its command; Vpdf uses `VISUAL` first and falls back to `vi` when
  neither is set.

macOS printing uses the system's `osascript`, PDFKit, and AppKit; no extra
installation is needed. It requires a logged-in graphical session on the machine
running Vpdf. Printing over SSH uses that machine's session.

macOS supplies `pbcopy` for text copying, `osascript` for image copying, and
`open` for links. Text copying also sends [OSC 52][osc52], subject to terminal
policy; SSH skips native clipboard helpers. See [external actions][integrations]
for clipboard and link boundaries.

## Mouse and focus

A press focuses the pane or panel under it. The wheel scrolls the surface under
the pointer without changing keyboard focus. Hover highlights and describes
links, notes, rows, and thumbnails. Right-click for actions on that item; choose
with the pointer or `j` and `k`, then click or Enter to run. Shift-F10 opens
the same menu for the cursor, selected row, or selected thumbnail. A middle drag
pans.
Middle-click an internal destination to open it in a split while retaining
focus in the original pane.

In the macOS app, a trackpad moves the page by exact amounts with the system's
momentum; panels and lists still move in whole wheel steps. A pinch zooms the
page under the pointer, keeping that point in place. Fit and keyboard zoom
still apply.

Kitty reserves Shift-click and Shift-drag for terminal selection. `:mouse off`
returns all clicks and drags to the terminal; `:mouse on` restores capture.
Under tmux, mouse events instead follow its `mouse` setting: with it on, a drag
selects in tmux copy mode; with it off, the outer terminal selects. Ctrl-click
can become right-click on macOS, so use the named gestures without Control.

Kitty clamps a drag above the window to its top row, limiting autoscroll there.
Without pixel reports, including under tmux, edge dragging scrolls at one pace.
Outside tmux, a missed release ends on the next pointer motion; a key also ends
a held text drag. A middle drag begun over a hover preview opens its target in
a split instead of panning.

## Licensing

Vpdf's original source code uses the [MIT license][license]. Dependencies retain
their own licenses. [Theme notices][theme-notices] retain palette attribution
and licenses.

MuPDF and its Rust bindings, `mupdf` and `mupdf-sys`, use AGPLv3 in the current
dependency stack. Distributed executables that link them remain subject to AGPL
obligations, including corresponding-source requirements. Licensing Vpdf's code
under MIT does not relicense these dependencies. The [AGPL text][agpl] is
retained for them; see also [MuPDF licensing][mupdf-license].

The base14 fonts inside MuPDF retain their upstream licenses. Consult [the
manifest][manifest] and [lockfile][lockfile] when identifying the dependencies
used in a particular build.

The help PDF and native interface bundle an Iosevka-derived font under the SIL
Open Font License, with Nerd Fonts icon attributions. Short notices and license
links appear at the end of help. Full notices accompany the binaries and the
macOS app's resources.

[homebrew]: https://brew.sh/
[mise]: https://mise.jdx.dev/
[development]: https://github.com/letientai299/vpdf/blob/main/docs/dev/dev.md#build-from-source
[kitty]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
[tmux-install]: https://github.com/tmux/tmux/wiki/Installing
[placeholders]: https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders
[keyboard]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
[wezterm-config]: https://wezterm.org/config/files.html
[wezterm-keyboard]: https://wezterm.org/config/lua/config/enable_kitty_keyboard.html
[modify-keys]: https://invisible-island.net/xterm/modified-keys.html
[tmux-manual]: https://man.openbsd.org/tmux
[tesseract]: https://github.com/tesseract-ocr/tesseract
[fontconfig]: https://fontconfig.pages.freedesktop.org/fontconfig/fontconfig-user.html
[fzf]: https://github.com/junegunn/fzf#installation
[wl-clipboard]: https://github.com/bugaevc/wl-clipboard
[xclip]: https://github.com/astrand/xclip
[xsel]: https://github.com/kfish/xsel
[vim]: https://www.vim.org/download.php
[osc52]: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Operating-System-Commands
[integrations]: reading.md#external-actions
[license]: https://github.com/letientai299/vpdf/blob/main/LICENSE
[theme-notices]: https://github.com/letientai299/vpdf/blob/main/docs/licenses/themes.txt
[agpl]: https://github.com/letientai299/vpdf/blob/main/LICENSE-AGPL
[mupdf-license]: https://mupdf.readthedocs.io/en/latest/license.html
[manifest]: https://github.com/letientai299/vpdf/blob/main/Cargo.toml
[lockfile]: https://github.com/letientai299/vpdf/blob/main/Cargo.lock
[defaults]: config.md#built-in-defaults
