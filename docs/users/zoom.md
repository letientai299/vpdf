# Zoom and narrow panes

<!-- help-topic:zoom -->

`+` enlarges the page and `-` shrinks it. `=` fits the page width; `zf` fits a
whole page. `zr` rotates clockwise. `zc`, or `:crop`, toggles automatic margin
cropping. `:crop on` and `:crop off` set it explicitly. These change the view;
the source PDF and printing remain unchanged.

A narrow split shrinks a fitted PDF page. Close the outline with `gq`, widen
the pane, or use extracted text when the page text becomes too small. A middle
pointer drag pans a zoomed page. Positions and selections remain tied to the
PDF's glyphs as zoom and pane size change.

`:cursorline off` hides the line band in Cursor and Visual modes. It does not
hide the glyph cursor or remove the selection. `:cursorline on` restores it.
The [configuration chapter][configuration] explains setting its initial value.

## Text preview

`:text` opens extracted text, wrapped to the pane width, and scrolls through
successive pages. Page separators identify each new page, and the top row names
the pages in view. Cursor and Visual modes still navigate and select document
text. Search and link destinations retain their document positions.

Press `q` to leave text preview before closing the document. Text preview can
make small split panes easier to read. It depends on extracted or recognized
text: it cannot reconstruct missing characters, columns, equations, or the
original authoring format. A scan may need [OCR][ocr].

[configuration]: config.md
[ocr]: ocr.md
