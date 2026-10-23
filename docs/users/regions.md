# Figures and regions

<!-- help-topic:regions -->

A detected figure selects a PDF image region. A manual rectangle captures any
part of one page, including text in a scan. Both offer the same action bar.
Use your own PDF to practice export; built-in help permits copying and OCR but
cannot save regions.

## Detected figures

`]i` selects the next figure and `[i` the previous one. `zF` fits the selected
figure in the view. `gf` opens FIGURES; choose a row and follow it with Enter.
`gF` opens actions for the selected figure. A PDF with no detected figures can
still use manual capture. Figure detection is a navigation aid, not a guarantee
that every diagram or vector drawing receives its own entry.

## Draw a rectangle

1. Press `R`. REGION starts at the glyph cursor in Cursor mode, or near the
   view center otherwise.
2. Use `h`, `j`, `k`, and `l` to resize from the active corner. A count makes
   a larger adjustment. The thicker border marks that corner.
3. Press `o` to exchange corners. Use `H`, `J`, `K`, and `L` to move the whole
   rectangle without changing its size. It stays within its page.
4. Press Enter to confirm. ACTIONS opens. Use `h` and `l` to choose an action,
   then Enter to run it. `R` returns to editing; Escape or `q` closes capture.

Alt-drag on the page draws a rectangle with the pointer. Click an action to
run it. Under tmux, Alt-drag may need the forwarding binding in
[Installation and setup][install]. Closing capture restores the reading mode
that was active underneath it.

## Choose an action

Copy image rasterizes the rectangle and delivers it to the image clipboard.
It needs local clipboard support or the terminal's image-clipboard protocol
over SSH. Copy text copies existing extracted or page-OCR text inside the
rectangle; it does not recognize the image.

Save PNG exports an image of the region and reports its destination. It requires
a file-backed document and a writable destination; it is unavailable for
built-in help. OCR text runs Tesseract for the selected rectangle and copies
the recognized text. It needs the helper and configured language data; it does
not add selectable text to the page. Use `:ocr` for that.

The action bar reports Running, completion, or an error. Wait for the result
before assuming clipboard delivery or export succeeded. On failure, check the
helper, destination, or clipboard policy and retry. `:cancel` stops pane region
work as well as the other background jobs. See [Scanned pages and OCR][ocr]
and [Troubleshooting][troubleshooting] for recovery.

[install]: install.md
[ocr]: ocr.md
[troubleshooting]: troubleshooting.md
