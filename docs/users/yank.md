# Selecting and copying

<!-- help-topic:selection -->

Copying reads extracted glyphs. It never edits the PDF. Start with a paragraph
in [Practice text][sample], then repeat the exercise on your book.

## Select a passage

1. In Normal, press `i` for Cursor mode. Move with `h`, `j`, `k`, `l`, or word
   motions until the cursor reaches the passage.
2. Press `v` and move to extend a character selection. Use `o` to move the
   other end. Press `y` to copy, or Escape to abandon the selection.
3. Use `V` for whole lines. `vip` selects an inner paragraph; `vis` selects
   a sentence. Press `y` after inspecting the range.
4. Press `gv` to reselect the previous range. Escape returns to the mode
   that began Visual; another Escape from Cursor returns to Normal.

A pointer drag selects text too. Repeated clicks select a word, line, or
paragraph. Drag past the top or bottom of the view to keep scrolling until
release; a key also ends that drag. Scrolling can move a selected passage
outside the view without discarding it.

## Copy without a visible selection

`yiw` copies the word at the cursor; `yis` the sentence; `yip` the paragraph.
`yy` copies the current line. `3yy` copies three lines. Line yanks join wrapped
prose and rejoin line-end hyphenation. Use `3yY` to preserve three extracted
lines as laid out, useful for code fragments. Character selections and object
copies retain extracted line boundaries; they do not reconstruct formatting.

The first practice sentence copies as `Select this sentence to try text
selection.` The Unicode practice text retains `café, naïve, Ελληνικά, →, ≤.`
Compare your pasted result with the visible page. Missing fonts, extraction
order, columns, and scanned text can make the result differ. Try `:text` to
inspect extraction or [OCR][ocr] for an image page.

## Registers and feedback

A quote chooses a register: `"ayy` copies a line into register a; `"Ayy`
appends to a. Each yank also updates the unnamed register and register 0.
Registers belong to the live reader and are not saved across launches. They
hold copied text; there is no PDF paste or editing command.

The `Yanked ... characters` message reports text extraction into the reader.
Clipboard delivery is a separate step. Uppercase append retains accumulated
text in the register; clipboard delivery carries the new yank. If extraction
fails, inspect the status error, reduce the range, or recognize the page.

`yp` copies the source path. On the manual it reports `builtin:help`, which
is a document identity rather than a disk filename.

## Clipboard delivery

Text copying sends OSC 52 to the terminal and can use a local clipboard helper.
Terminal permission and payload limits can reject a copy. Test a short sentence
first. Over SSH native clipboard helpers are skipped because they would target
the remote clipboard; allow OSC 52 in the local terminal and multiplexer.

Image copying needs a native helper locally, or terminal image-clipboard support
and acknowledgement over SSH. Successful text copying does not establish image
support. See [Optional helpers][helpers] for installation. Terminal selection
is independent: `:mouse off` gives clicks and drags back to the terminal;
`:mouse on` restores reader capture.

[sample]: sample.md
[ocr]: ocr.md
[helpers]: install.md#optional-helpers
