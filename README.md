# vpdf

A keyboard-driven terminal PDF reader with continuous scrolling, Vim motions,
glyph selection, document search, outlines, links, and session OCR. The release
executable embeds MuPDF and fallback fonts. Python is not required.

## Install and run

Install a prebuilt snapshot with [Homebrew][homebrew]:

```sh
brew install letientai299/tap/vpdf
vpdf document.pdf
```

The tap follows successful builds of the `main` branch. Run `brew update` and
`brew upgrade vpdf` to install a newer snapshot.

With [mise][mise], download the rolling `dev` build:

```sh
mise i github:letientai299/vpdf@dev
mise exec github:letientai299/vpdf@dev -- vpdf document.pdf
```

Prebuilt binaries support Linux x86-64 and macOS ARM64. Successful CI on `main`
updates this prerelease. It tracks development and can contain breaking changes.
To activate it globally, run `mise use -g github:letientai299/vpdf@dev`.
To download a newer build, run `mise i --force github:letientai299/vpdf@dev`.

### Build from source

Install a C/C++ compiler, Make, and libclang. On macOS, the Xcode command line
tools provide the native build tools. In a checkout, run:

```sh
mise trust
mise install
mise run install
vpdf document.pdf
```

`mise run install` builds the release binary and atomically copies it to
`~/.local/bin/vpdf`. Put that directory on `PATH`.

On x86-64 Ubuntu, install the native build dependencies:

```sh
sudo apt-get install build-essential libclang-dev musl-tools
```

`mise build` writes `target/release/vpdf` with symbols and debug information
stripped. On Ubuntu it links statically with musl and requires no shared
libraries. On macOS it embeds MuPDF and fallback fonts, and links only to
Apple-provided system libraries. Build separately on each platform; the
executables use different formats. OCR and clipboard helpers remain optional
external programs.

`mise release` builds and packages `target/dist/vpdf`. On Ubuntu it compresses
the executable with [UPX][upx], verifies the packed data, and checks that the
result starts. It leaves the Cargo build uncompressed. On macOS it packages
the stripped executable without compression because [UPX does not support
modern macOS executables][upx-macos].

[Kitty][kitty] displays PDF graphics. Other terminals use extracted text;
`--text` forces text mode. With tmux, enable `allow-passthrough on`. The reader
uses the terminal's reported cell size for pixel placement.

Scrolling animates at 120 frames per second; `--fps 240` matches a 240 Hz
display. Kitty paints at most once per `repaint_delay` (10 ms by default) and
waits `input_delay` (3 ms) before reading program output, so 120 frames per
second on screen also needs `repaint_delay 8` or lower in `kitty.conf`.

```sh
vpdf --page 12 document.pdf
vpdf text document.pdf --page 12
vpdf search document.pdf 'attention'
vpdf search document.pdf '\vattention(?= mechanism)'
vpdf info document.pdf
vpdf preview document.pdf --page 3
```

`vpdf preview` prints a page as a Kitty image built from Unicode placeholders,
so it scrolls like text and works as `fzf --preview 'vpdf preview {}'`, inside
tmux too. It sizes itself from `FZF_PREVIEW_COLUMNS` and `FZF_PREVIEW_LINES`,
and prints the page's text in other terminals.

Encrypted PDFs prompt without echoing the password. Batch commands accept
`--password-stdin`; passwords are never command-line arguments. The reader
opens PDF files for reading and does not change annotations or form values.

## Reading

Counts apply to motions: `10j`, `12gg`, `12G`, `2yw`. `h j k l` and arrows move
through extracted glyphs. `w b e`, `0 ^ $`, `gg G`, and Space / Shift-Space (or
Backspace) follow the plugin's word, line, document, and page motions. `{` and
`}` move between extracted text blocks as paragraphs; `y}` excludes the next
paragraph start.

`Ctrl-d/u` scroll half a screen; `Ctrl-f/b` and PageDown/PageUp scroll a full
screen. `Ctrl-e/y` and the wheel scroll rows. `zt zz zb` place the cursor at the
top, middle, or bottom. `zc` crops white margins with one trim measured across
sampled pages, so text grows and every page keeps the same scale; the choice
persists per document. `+ - =` zoom or fit width, `zf` fits a page, `zh zl`
pan, and `zr` rotates. `Ctrl-o/i` move through jump history.

Use `v` or `V` to select, `o` to swap ends, and `gv` to reselect. `y`, `yy`,
`yw`, `ye`, `y$`, `yG`, `ygg`, `y<Space>`, and `y<S-Space>` copy character or
line ranges. `yiw`, `yaw`, `viw`, and `vaw` address words. Selections cross
pages and retain Unicode and line breaks. A linewise yank includes its final
newline.

Yanks to the unnamed register, `"+`, or `"*` reach the system clipboard, as with
Vim's `clipboard=unnamedplus`. Named registers use `"ayiw`, stay inside the
reader, and append with uppercase names. The reader sends [OSC 52][osc52] and,
outside SSH, also pipes the text to `pbcopy`, `wl-copy`, `xclip`, or `xsel`. OSC
52 acceptance and size limits depend on the terminal. Click, drag, and
double-click move the cursor, select a range, and select a word. Editing keys
leave the PDF unchanged.

`/` and `?` search forward or backward and preview the first match while you
type; `Esc` returns to the start. The status line shows `[match/total]`; `n N`
cycle matches, and `*` finds the cursor word. Literal search folds Unicode case,
decomposes compatibility forms, collapses whitespace, and joins line-end
hyphenation. Prefix `\v` for regular expressions, which run on that folded,
decomposed text: `(?-i)` cannot match capitals, and accented letters in classes
need their decomposed form. The bounded regex engine supports lookaround and
backreferences; it is not Python's regex implementation.

`gs`, `go`, and `gl` open search results, the table of contents, and links. When
a PDF has no outline, `go` shows one inferred in the background from larger text
and section numbering such as `3.2 Title`, marked `inferred`. `gp` or
`:overview` shows a grid of page thumbnails; `h j k l` move, Enter or a click
opens a page, and `q` or Esc returns. `ga` lists the document's highlights,
underlines, notes, and other annotations with their marked text and comments;
the reader does not edit them. The side without focus is dimmed: the PDF while a
panel has focus, the panel while the PDF has focus. A focused panel owns the
keyboard: `j k gg G Ctrl-d Ctrl-u zt zz zb` move through rows, `/ ? n N` search
them with highlighted matches and smartcase, `{ }` jump between headings, and
`yy` yanks a row. Text motions and selections do not reach the PDF until `Esc`
returns focus. Enter follows a panel item; `o` follows while retaining panel
focus. The outline stays open after Enter. `go` focuses it again; `gq` closes a
panel. Enter on the PDF follows a link under the cursor and does nothing
elsewhere. `K` previews the destination of the link under the cursor in an
overlay, such as a cited reference or footnote, without moving; Enter follows it
and any other key closes it. `f` labels every visible link; typing a label
follows it, and `F` copies a URL label instead of opening it. Only explicitly
followed HTTP/HTTPS links launch a browser. Other URI schemes are rejected.

`gx` or `:text` opens text preview; `q` returns to the rendered view. `g?` and
F1 show the key and command reference. `Esc` cancels selection, search, and
OCR. `q` or `:q` closes the current tab and exits after the last one. Space
remains unmapped.

## Tabs

`vpdf a.pdf b.pdf` opens each document in a tab. `:tabe file.pdf` opens another
tab, `:e file.pdf` replaces the current one, and both complete directories and
PDF names. `gt` and `gT` cycle tabs; `3gt` selects the third. `:tabc` closes a
tab and `:qa` closes all. With [fzf][fzf] on `PATH`, `:fzf` picks a PDF below
the working directory, including hidden and VCS-ignored folders such as `.ai`
but not `.git`, previewing its first page as an image in Kitty, and opens it in
a new tab, or in the focused pane when the tab is split; the command is hidden
without fzf. Opening a document that is already open selects its tab. The status
line lists tabs when more than one is open, shows the current outline section,
the percentage read, and the zoom when it differs from fit width. The terminal
title shows the document title. Each tab keeps its own position, zoom, panels,
search, and workers.

## Panes

`Ctrl-w` manages panes within a tab as in Vim. `Ctrl-w s` and `Ctrl-w v` split
the focused pane above or to the left, showing the same view; `:split file.pdf`
and `:vsplit file.pdf` open another document instead. `Ctrl-w h j k l` and the
arrows move focus by direction, `Ctrl-w w`, `W`, and `p` cycle or return to the
last pane, and `3 Ctrl-w w` picks the third. `Ctrl-w c` or `:close` closes a
pane, `Ctrl-w o` or `:only` closes the others, and `:q` closes the pane, then
the tab. `Ctrl-w + - < >` resize by a count of cells, `Ctrl-w _ |` maximize,
`Ctrl-w =` equalizes, `Ctrl-w H J K L` move the pane to the far left, bottom,
top, or right edge, `Ctrl-w r` and `R` rotate the panes of a row or column
(refused, as in Vim, when one of them is split), `Ctrl-w n` opens a pane above
listing recent documents, `Ctrl-w x` swaps with the sibling pane, and `Ctrl-w T`
moves the pane to a new tab. Each pane has its own status line, position, zoom,
and panels; clicking a pane focuses it, and the wheel scrolls the pane under the
pointer without focusing it. Panes share one Kitty image budget.

## Recent documents

The reader saves each document's cursor, zoom, and rotation, and reopens it
there; `--page` overrides the saved page. `gr` or `:recent` lists recent
documents; in a split, a chosen document replaces the focused pane. `vpdf`
without a path opens a start screen with that list: Enter or a click opens a
document, and `:e` opens any file with path completion. Command and search
history persist too. State lives in `$XDG_STATE_HOME/vpdf/state.json`, or
`~/.local/state/vpdf/state.json`. It is written after the position rests for
five seconds, at most once a minute while it keeps changing, and on exit,
including when the terminal closes. Concurrent readers merge their entries.

## Marks

`ma` marks the cursor in this document and `mA` marks it across documents,
as in Vim. `'a` jumps to the mark's line, `` `a `` to its exact glyph, and
`''` back to where the last jump started. `:marks` lists them. Marks persist
with the recent-document state.

## Live reload

The reader watches each open document's directory through OS file events
(FSEvents, inotify, or kqueue via [notify][notify]), so a build that replaces
the PDF by rename is still seen. After the file's size and modification time
hold still for 100 ms, it reloads and keeps the cursor, zoom, marks, open panel,
and search, which makes it a live preview for LaTeX or Typst builds. Network
mounts can drop events, so a 2 s poll stays as a fallback. A half-written file
reports an error and reloads on the next change.
`:reload` forces a reload.

## Key clues

As with [mini.clue][clue], pausing half a second after a prefix such as `g`,
`z`, `"`, `m`, or `'` lists the keys that can follow it with their
descriptions above the status line. `"` previews the registers' contents and
`'` lists the set marks. The next key hides the list.

## Commands

`:` opens the command line, which edits like zsh's emacs keymap. Tab lists
every completion in columns above the prompt and inserts their common prefix;
further Tabs and Shift-Tabs cycle through them. Completion falls back to
ignoring case. Up and Down walk history; `Ctrl-r` and `Ctrl-s` walk entries
that contain the typed text. The newest history entry that extends the line
appears as a muted suggestion, as with zsh-autosuggestions: `Ctrl-Space`,
Right, or End accept it, and `Alt-f` accepts its next word. `Ctrl-w` and
Ctrl-arrows stop at `/`, `.`, `:`, and `=`, while `Alt-b`, `Alt-f`, `Alt-d`,
and `Alt-Backspace` use zsh's default `WORDCHARS`. `Ctrl-u`, `Ctrl-k`, and
`Ctrl-y` kill and yank, `Ctrl-t` transposes, `Ctrl-_` undoes, and `Ctrl-x
Ctrl-e` edits the line in `$VISUAL` or `$EDITOR`. The [help panel][keys] lists
every binding. A command name can be shortened to any unique prefix,
such as `:rel` for `:reload`. `:12` jumps to page 12; `:page iv` jumps to a page
label. `:ocr [language]` recognizes the current page with [Tesseract][ocr] and
completes installed languages. The key reference lists every command; the
[command table][commands] is the source.

OCR requires the `tesseract` executable and the requested language data. OCR
runs in a separate process, is cancellable, and stays in memory. Word boxes
provide approximate character positions. Existing PDF OCR text is available
without Tesseract.

## Development

```sh
mise run hooks
mise run check
mise run test:corpus
mise run test:ocr
mise run perf:check
```

Rustfmt uses two spaces and an 80-column target; a separate check enforces the
source width. Clippy rejects performance, complexity, and selected
maintainability violations. Prek runs formatting, linting, and tests before
commits. Rumdl formats and checks Markdown. The corpus tasks download public
PDFs into the local cache and verify their recorded SHA-256 hashes. OCR tests
require Tesseract; ordinary tests do not.

`mise run bench` measures real-PDF extraction, tiles, navigation, search, and
serialized terminal frames. `perf:check` requires warm frame p95 below 8.33 ms,
warm paired motion p95 below 1 ms, and no warm base-image uploads. These CPU
budgets do not measure terminal painting or guarantee a display frame rate.
Set `VPDF_BENCH_OUTPUT=/path/report.json` to save results.

`mise run bench:latency` drives the reader with a live render worker: cold
open, twenty page jumps, zoom steps, and scrolling paced at 10 to 120 inputs
per second. It reports input handling time, time to the first and the last
visible tile, frames with blank tiles, and bytes per frame. `mise run
bench:stages` splits cold page cost into page load, text extraction, display
list, tile raster, and encoding. Set `VPDF_EXTRA` to a `PATH`-style list of
PDFs to measure other documents and `VPDF_COLUMNS` to change the 179-column
screen.

For an interactive trace, run `VPDF_TRACE=/path/frames.jsonl vpdf document.pdf`.
The file is overwritten. It records frame construction, output flush, consumed
input to output flush, and base-image transfer bytes; it contains no document
text. Input timing starts when the event loop consumes an event. Terminal
rendering and physical input latency are outside this measurement.

See [architecture][architecture] for worker ownership and performance limits.
The [key reference][keys] is also embedded in the executable.

## License

AGPL-3.0-or-later. MuPDF is linked into the executable and carries its own
copyright notices. See [MuPDF licensing][mupdf-license]. Bundled fallback fonts
carry their upstream licenses in the dependency source distributions.

[homebrew]: https://brew.sh/
[mise]: https://mise.jdx.dev/
[kitty]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
[ocr]: https://tesseract-ocr.github.io/
[mupdf-license]: https://mupdf.readthedocs.io/en/latest/license.html
[architecture]: docs/architecture.md
[keys]: src/help.txt
[commands]: src/cmdline.rs
[osc52]: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h3-Operating-System-Commands
[notify]: https://docs.rs/notify/8.2.0/notify/
[fzf]: https://github.com/junegunn/fzf
[clue]: https://github.com/nvim-mini/mini.nvim/blob/main/readmes/mini-clue.md
[upx]: https://upx.github.io/
[upx-macos]: https://github.com/upx/upx/discussions/931
