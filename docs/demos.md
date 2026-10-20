# Demos

These captures show vpdf reading [Attention Is All You Need][paper],
[Think Python, second edition][book], and a [page scan][scan] from First Draft
of a Report on the EDVAC. Open an image at full resolution. Each capture shows
the build recorded in its [manifest][manifest]; newer builds can differ. See the
[reading guide][reading] for workflows and `:help` for current bindings.

The papers are by Ashish Vaswani and colleagues and John von Neumann;
Think Python is by Allen B. Downey.

## Three documents in split panes

The paper's outline and citation preview sit beside search results and
highlighted URLs in Think Python. A third pane shows selected text recognized
from the scanned EDVAC report. Each pane keeps its own document, reading
position, and mode.

![Three PDFs with an outline, citation preview, search results, and OCR selection][overview]

## Info, help, and pending keys

`:info` shows document metadata, reader state, and terminal capabilities.
`:help` lists keys and commands. Pausing after `g` shows the available next
keys while the page stays visible.

![Document info, help reference, and pending-key hints together][help]

## Search

Search results keep matching passages in a side panel and highlight the phrase
on the rendered page.

![Search results and a highlighted Multi-Head match][search]

## Reference preview

An internal citation opens a preview of its bibliography entry while the
source passage and links panel remain visible.

![A bibliography preview over the source passage][preview]

## Figure capture

A detected figure has an action bar for image copying, text copying, PNG
export, and optional OCR. The pointer chooses Copy text in this capture.
Captures leave the source PDF unchanged.

![A selected diagram with its capture actions][figure]

## Mouse context menu

A mouse drag selects text, and a right click opens selection actions. Moving
the pointer over a row chooses it; a click runs it. The menu also supports
dragging from the opening right press and releasing on an action.

![Page context menu with a row chosen by hovering][mouse-menu]

## Point at link hints

Link hints label visible destinations. Pointing at a label picks it while the
other labels stay visible; Enter or a click follows that destination.

![Link labels with one picked by the pointer][mouse-hints]

## Sweep a capture region

An Alt-drag dims the page outside the capture rectangle. The status line shows
how to finish or cancel the sweep. Releasing opens the figure action bar,
whose actions also respond to hover and click.

![A held Alt-drag capture with its release and cancel hint][mouse-region]

## Page overview and document tabs

Thumbnails show the visible portion of Think Python's page grid around the
selected page. Scroll to browse the rest of the book. The status line shows
the active book tab, an inactive paper tab, and the document's page total.

![Page thumbnails, the selected page, and document tabs][pages]

## Recent documents

The recent-document view lists visited PDFs and saved reading positions.
Hovering the scan updates its preview card while the selected row stays on
the paper. The card shows reading details and document metadata.

![Hovered scan preview beside the selected paper in recent documents][recent]

## Jump history

Jump history shows heading and search destinations with their surrounding
context, page, and word. It retains destinations in both history directions.

![Heading and search destinations in jump history][jumps]

## OCR on a scan

Page OCR makes the opening definitions in John von Neumann's
[scanned EDVAC report][scan] selectable. The recognized text appears beside the
original scan. Recognition stays in memory; the source PDF stays unchanged.

![Selected OCR text beside the scanned EDVAC report][ocr]

## Capture identity

These examples use a release build on macOS in Kitty, captured directly at
2560 × 1440 pixels. The terminal font uses 200% scaling: 26 points from a
13-point base. Images use optimized JPEG with lossy compression.
The [capture manifest][manifest] records the build, machine, terminal, font,
document hashes, and scene inputs. The [development guide][capture] explains
regeneration and storage. Scene encoding describes the current images; capture
encoding records the original output. Reencoded scenes retain the source image
name and hash.

The screenshots demonstrate visible reading states. They do not measure frame
rate, input latency, OCR accuracy, or clipboard delivery. Optional page OCR
keeps recognized text in memory without adding an OCR layer to the source PDF;
see the [text guide][text-guide].

The displayed paper and book retain their authors' rights. Document URLs and
verified hashes come from the [public corpus script][corpus].

The OCR input is MIT's public scan of the EDVAC report's IEEE reprint. Its
title page credits Michael D. Godfrey's corrected transcription of the 1945
report. The capture checks that text extraction is empty and selection is
unavailable before running Tesseract. It then waits for recognition to finish
before selecting text. The manifest records the OCR engine and language.

[paper]: https://arxiv.org/abs/1706.03762
[book]: https://greenteapress.com/wp/think-python-2e/
[reading]: reading.md
[overview]: assets/overview.jpg
[help]: assets/help.jpg
[search]: assets/search.jpg
[preview]: assets/preview.jpg
[figure]: assets/figure.jpg
[mouse-menu]: assets/mouse-menu.jpg
[mouse-hints]: assets/mouse-hints.jpg
[mouse-region]: assets/mouse-region.jpg
[pages]: assets/pages.jpg
[recent]: assets/recent.jpg
[jumps]: assets/jumps.jpg
[ocr]: assets/ocr.jpg
[scan]: https://web.mit.edu/sts.035/www/PDFs/edvac.pdf
[manifest]: assets/capture.json
[capture]: dev/dev.md#gallery-capture
[text-guide]: dev/text.md
[corpus]: ../scripts/corpus.sh
