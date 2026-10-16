# vpdf

A keyboard-driven terminal PDF reader with continuous scrolling, Vim motions,
glyph selection, document search, outlines, links, and session OCR. The release
executable embeds MuPDF and fallback fonts. Python is not required.

## Install and run

Install [mise][mise], a C/C++ compiler, Make, and libclang. On macOS, the Xcode
command line tools provide the native build tools. Then run:

```sh
mise trust
mise install
mise run install
vpdf document.pdf
```

`mise run install` builds the release binary and atomically copies it to
`~/.local/bin/vpdf`. Put that directory on `PATH`.

[Kitty][kitty] displays PDF graphics. Other terminals use extracted text;
`--text` forces text mode. With tmux, enable `allow-passthrough on`. The reader
uses the terminal's reported cell size for pixel placement.

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
hyphenation. Prefix `\v` for regular expressions. The bounded regex engine
supports lookaround and backreferences; it is not Python's regex implementation.

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
a new tab; the command is hidden without fzf. Opening a document that is already
open selects its tab. The status line lists tabs when more than one is open,
shows the current outline section, the percentage read, and the zoom when it
differs from fit width. The terminal title shows the document title. Each tab
keeps its own position, zoom, panels, search, and workers.

## Recent documents

The reader saves each document's cursor, zoom, and rotation, and reopens it
there; `--page` overrides the saved page. `gr` or `:recent` lists recent
documents. `vpdf` without a path opens a start screen with that list: Enter or
a click opens a document, and `:e` opens any file with path completion.
Command and search history persist too. State lives in
`$XDG_STATE_HOME/vpdf/state.json`, or `~/.local/state/vpdf/state.json`, and is
saved every few seconds while the position changes, when a tab closes, and on
exit. Concurrent readers merge their entries.

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

## Commands

`:` opens the command line. Tab and Shift-Tab cycle completions for command
names and arguments; Up and Down recall earlier commands or searches that start
with the typed text. Left, Right, `Ctrl-a`, `Ctrl-e`, `Ctrl-w`, `Ctrl-u`, and
`Ctrl-k` edit the line. A command name can be shortened to any unique prefix,
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
