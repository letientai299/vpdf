# vpdf

A keyboard-driven terminal PDF reader. Read, navigate, and copy text without
changing the source document.

## Start

```sh
brew install letientai299/tap/vpdf
vpdf document.pdf
```

Use `vpdf --help` for CLI options, and `:help` inside the reader
for keys and commands. Use `:jumps` to list recorded jump destinations from
both history directions; Enter follows a row.

The reader requires [Kitty graphics][kitty]; `:text` shows extracted text.
The reader requires [Kitty keyboard][keyboard] disambiguation, or tmux's
extended keys; see [terminal setup][setup].

Normal `h j k l` and arrow keys pan or scroll the viewport; counts scale the
movement. `i` enters Cursor at a visible click/jump target or the first visible
text. Cursor shows a glyph cursor: `h j k l`, word, line, paragraph, and page
motions position it before `v` or `V` starts character or line Visual selection.
Direct Normal `v`/`V` selects from the same visible entry point. `o` swaps ends,
`gv` restores the last range, and `y` copies. Visual returns to its entry mode;
Esc from idle Cursor returns to viewport Normal.

Cursor and Visual `W B E gE` move between whitespace-separated WORDs; `( )`
move between sentences, preserving scientific abbreviations such as `Fig.`,
`et al.`, and `e.g.`. Only Visual and explicit copy operators accept `i` (inner)
and `a` (around) objects: `w W s p`, bracket pairs `() [] {} <>` (`b` and `B`
alias parentheses and braces), and single or double quotes, including
typographic quotes. For example, `yiw` copies a word, while `viw` selects it.
Counts repeat objects or select enclosing bracket pairs. Objects can cross
extracted lines and pages; paragraphs follow extracted blocks. Bare Cursor
`i`/`a` never start an object.

`[[` and `]]` jump to the heading before or after the viewport in Normal,
or the active glyph in Cursor/Visual. Counts repeat heading jumps;
Ctrl-o and Ctrl-i return through jump history.

Visual highlights the active line; `:cursorline` toggles this band. Normal shows
no glyph cursor; Cursor shows its glyph. Link, search, mark, and heading jumps
tint the destination line for 600 ms in PDF and text views. Links, hints,
previews, and panel rows land a target that sits outside the view or on its
first or last row below one row of context; a target already in view stays put,
and the text view stops at its page's end. Explicit scroll keys preserve glyph
and selection positions;
zoom, crop, rotation, panels, and resize keep the reading area anchored. `K`,
Enter, and `*` use the visible active glyph in Cursor/Visual, or a visible
click/jump target in Normal. A textless view reports
selection unavailable; start OCR explicitly with `:ocr`.

Esc cancels an owned transient or pending input before leaving a reading mode.
It preserves side-panel focus, the underlying entry mode, and committed jobs.
`:cancel` stops the active pane's search, OCR, and outline work. Extracted text
follows the same modes and viewport rules.

Tab and Shift-Tab switch between content and the visible side panel, keeping
selection and panel state. Ctrl-o/Ctrl-i navigate history; prompt Tab completes.
Panel commands focus their list. Enter follows a row in Normal while keeping the
list; `o` or a row click keeps panel focus. An unopened document opens in a new
tab with Enter, or a split with `o`/click; an open view is reused. The recent
view and `:fzf` open the document in place; `o` does nothing there. `:e`,
`:tabe`, `:split <file>`, and `:vsplit <file>` also focus an existing view
before applying placement. Pane changes keep Cursor mode, Visual ranges,
committed searches, panel position, and local focus; incomplete input clears.
`gq` or panel-local `q` closes the list. Overview occupies the content slot.
Hover and wheel preserve keyboard focus. Hover underlines a panel row other than
the selected one and tints a thumbnail. In the status line it shows a clipped
row title in full as room allows (`…` marks a cut), a thumbnail's page label and
section, or a commented note's author and comment; a message or error takes the
line first. Over a link inside a commented highlight, the link wins; `K` shows
the note. Clicking outside a prompt or preview dismisses it; the next click
activates the underlying surface.

The wheel scrolls whatever is under the pointer: the page, the `K` preview or
note, a panel list, or the page thumbnails, without moving a list's selection. A
quick run of turns with the pointer at rest stays on one surface and one axis,
so a preview that scrolls under it does not take over and sideways drift does
not pan. Over the page, Ctrl-wheel zooms the PDF view and keeps the page point
under the pointer in place where the view allows; Shift-wheel or a sideways
wheel pans a zoomed page or wide extracted text. Lists and thumbnails ignore
the modifiers.

Dropping a PDF on a pane opens it there, or focuses the view that already shows
it; an open prompt takes the dropped path, or dragged text, as typed text.
Kitty 0.47 and later send drops through its [drag and drop protocol][dnd].
Files that the source app only promises, such as Mail attachments, exist only
while the drop lasts, so the reader cannot reload them later. Other terminals
paste a dropped file's path, so a pasted path to a PDF in Normal mode opens it.

Web addresses, DOIs, and e-mail addresses printed without a link annotation
count as links everywhere links do. Web addresses and DOIs wrapped across
lines join; an e-mail address split at a line break stays unlinked, since the
break could belong to the address or end a sentence.

Hovering a link tints it and shows its destination in the status line: the
page label and outline section, or the web address. A click follows the link
and records a jump; a drag that starts on a link selects text. The mouse back
and forward buttons act as Ctrl-o and Ctrl-i in the pane under the pointer.
Resting 400 ms on an internal link opens the `K` preview beside it. The preview
stays while the pointer is on the link or the preview, a click on it follows
the link, and leaving both or pressing a key closes it; keys keep their usual
meaning.

A drag selects characters; a double click selects a word, a triple click a line,
and a fourth click the paragraph. Dragging after a double or triple click
extends by words or lines. Dragging past the top or bottom edge scrolls, faster
the farther the pointer goes; above the window it scrolls one row per move.
Alt-click off a figure moves the selection's free end, or starts Visual from the
same entry point as `v`. Pointer selections return to their entry mode on Esc.

A right click opens a menu at the pointer with actions for the link, selection,
text, or figure under it, plus history, fit, outline, and overview. Each item
shows its key; hovering or `j`/`k` choose one, and Enter or a click run it. A
right press dragged onto an item runs it on release. Esc or a click outside
closes the menu; the wheel still scrolls the page. Hovering an `f`/`F` label
picks it, and Enter or a click follows or copies its link.

`K` owns its preview: `j`/`k` scroll rows, Ctrl-d/Ctrl-u scroll half its
height, and Enter follows the original destination. Counts apply to scrolling.
The border shows the destination page label and arrows at clipped edges.
Command prompts return to the preview; Esc clears pending input first.

`]i` and `[i` move between detected figures; counts repeat. `zF` fits the
selected figure, and `gF` opens its region action bar. `h`/`l` or hovering
choose Copy image, Copy text, Save PNG, or OCR text; Enter or a click runs the
selected action. Esc or a click outside closes the bar while preserving Cursor
and Visual state. PNG captures use 144 DPI, capped at 8 MP, and save beside the
PDF with a numbered filename. OCR requires Tesseract and copies text without
replacing the page's extracted text. Image copy uses native clipboard tools
locally and OSC 5522 over SSH when the terminal answers its capability query. A
remote copy succeeds after terminal acknowledgment; a timeout blocks newer image
writes until its final status. Figure detection groups image/vector bounds and
nearby captions; caption-only targets capture that caption. Alt-drag sweeps a
region, dimming the rest of the page, and its release opens the same bar;
Alt-click on a figure picks it.

## Documentation

Start at the [docs index][docs] for installation, project principles,
architecture, and development.

## License

vpdf's original code is [MIT-licensed][license]. Executables that include
MuPDF and its Rust bindings remain subject to their AGPL obligations. See
[dependency licensing][notices].

[kitty]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
[keyboard]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
[dnd]: https://sw.kovidgoyal.net/kitty/dnd-protocol/
[setup]: docs/installation.md#terminal-setup
[docs]: docs/readme.md
[license]: LICENSE
[notices]: docs/installation.md#licensing
