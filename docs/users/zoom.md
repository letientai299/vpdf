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

## Page colors

`:recolor dark 80` or `:recolor sepia 30` changes page pixels in the focused
view. The setting is saved per document; splits inherit it and open views
remain independent. `:recolor off` restores the original pixels. Use
`:help :recolor` for amounts and defaults. Try recoloring the help PDF too;
its setting lasts for the view’s session. `:theme` styles the interface.

## Text preview

`:text` opens extracted text and scrolls through successive pages. The built-in
manual wraps its text to the pane width. Page separators identify each new page,
and the top row names the pages in view. Cursor and Visual modes still navigate
and select document text. Search and link destinations retain their document
positions.

Press `q` to leave text preview before closing the document. Text preview can
make small split panes easier to read. It depends on extracted or recognized
text: it cannot reconstruct missing characters, columns, equations, or the
original authoring format. A scan may need [OCR][ocr].

See [continuous text][text-demo] and [zoom and rotation][zoom-demo].

[configuration]: config.md
[ocr]: ocr.md
[text-demo]: ../demos.md#continuous-text
[zoom-demo]: ../demos.md#zoom-crop-and-rotation
