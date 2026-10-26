# Links and notes

<!-- help-topic:links -->

`f` labels visible links. Type a label to follow its destination; Backspace
corrects hint input and Escape cancels it. The labels work on the visible
page, so scroll first when the desired link is off screen. A click also
follows. In Cursor mode, put the glyph cursor on the link and press Enter or
`Ctrl-]`. `Ctrl-o` returns from an internal jump.

## Inspect a reference

1. Open [Practice text][sample] and put the cursor on its Navigation link.
2. Press `K` to preview the internal destination. `j` and `k` scroll the
   preview; page scrolling continues beyond the target page.
3. Press Escape or `q` to close the preview without moving the main view.
   Enter follows the target. Press `Ctrl-o` after following to return.

Hovering an internal link also previews it. The preview stays while the pointer
moves between the link and preview; leaving both closes it. A left click inside
the preview follows. A middle click opens the destination in a split, keeping
focus in the original pane. A middle click on an internal link does the same.

`yf` shows hints for copying web addresses instead of opening them. `gx` opens
the URL at the cursor or on the selected Contents row. A middle click on a web
link copies its address.

## Citations and definitions

`gd` looks for a definition or reference destination for the cursor text or
selection. Recognizable citations, numbered references, and definitions in the
PDF's extracted text can supply destinations. A plain word without a matching
entry may have none. `gd` cannot fetch missing references from the internet.
Try a PDF link first when the publisher supplied one; use text search when
lookup finds no destination.

## Notes and highlights

`ga`, or `:annotations`, lists highlights and notes throughout the document,
grouped under a heading per page. A glyph in the annotation's color names its
kind. Select a row with `j` or `k`; the selected row shows its note on a dim
line below. Enter goes to its position. `K` shows the full comment; Escape
closes the comment. `yy` copies the selected row's text. A PDF without
annotations shows a short message instead of rows.
Try the practice note in [Practice text][sample]. Its comment reads
`Notes explain passages without changing your source PDF.`

Hovering a note or commented highlight shows author and comment in the status
line. Click a note icon to read the full comment beside it. Right-click a note
for Show note and Copy comment. The reader inspects existing annotations;
these operations do not write annotations to the PDF.

## External destinations

Following an HTTP or HTTPS link opens the configured browser; `mailto:` uses the
mail application after removing query parameters. Other URI schemes are
rejected. Opening a PDF alone never launches its links. Desktop integrations run
on the machine hosting Vpdf; over SSH that can be the remote machine. See
[Installation and setup][install] if an opener or clipboard helper is missing.

[sample]: sample.md
[install]: install.md
