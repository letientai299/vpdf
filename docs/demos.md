# Demos

These terminal captures use [Attention Is All You Need][paper],
[Think Python, second edition][book], the [EDVAC report scan][scan], and Vpdf's
embedded practice manual. The paper and book are by Ashish Vaswani and
colleagues and Allen B. Downey; the report is by John von Neumann. Open images
at full resolution.

Each scene identifies its executable, inputs, viewport, and recipe in the
[capture manifest][manifest]. [Publication records][media] retain canonical
URLs, hashes, and earlier attachments on the [closed hosting issue][host]. Older
captures retain their original build identity. Use the [reading guide][reading]
and executable help for current workflows and bindings.

## Three documents in split panes

An outline and citation preview sit beside search results and highlighted URLs
in Think Python. A third pane shows selected text recognized from the scanned
EDVAC report. Each pane keeps its document, reading position, and mode.

![Three PDFs with outline, reference preview, search, and OCR selection][overview]

## Built-in manual

`:help` opens the manual as a document. It supports the same outline, links,
search, and selection as a PDF. Close help to return to its invoking document.

![The built-in manual as a document with its outline][help]

`:help! panes` opens the manual beside a book, keeping both reading positions.

![Help beside Think Python in separate panes][help-split]

## Search

Results list matching passages and highlight the phrase on the rendered page.

![Search results with a highlighted Multi-Head match][search]

## Reference preview

`K` previews an internal reference without moving the source passage. The
preview can scroll beyond its target page.

![A bibliography preview over its source citation][preview]

`gd` also resolves recognizable unlinked references. Here it finds the caption
for a body reference to Figure 1.

![A resolved Figure 1 reference with its destination highlighted][definition]

## Continuous text

`:text` continues extracted text across page boundaries. The header names the
pages in view; a separator marks the next page.

![Extracted text spanning pages two and three][continuous-text]

## Annotation comments

The embedded practice manual supplies an authored note. Its annotation list,
full comment, hover author, and context actions leave the source unchanged.

![The practice note with its comment and author][annotations]

![Show note and Copy comment actions for the practice note][annotation-actions]

## Figures and tables

FIGURES lists detected diagrams and tables by caption. Its selected destination
can remain beside the document.

![Figure and table list beside the selected diagram][figures]

## Figure capture

A detected figure offers image copying, text copying, PNG export, and optional
OCR. The historical capture points at Copy text.

![A selected diagram with its capture actions][figure]

## Mouse context menu

A mouse drag selects text; right-click opens selection actions. Hover chooses a
row and a click runs it. The menu also supports dragging from its opening press
and releasing on an action.

![Selection context menu with a hovered action][mouse-menu]

## Point at link hints

Pointing at a hint picks it while other labels stay visible. Enter or a click
follows the destination.

![Link hints with one picked by the pointer][mouse-hints]

## Sweep a capture region

Alt-drag dims the page outside the rectangle. Releasing opens capture actions;
the status line explains finishing and cancelling the sweep.

![A held Alt-drag with its release and cancel hint][mouse-region]

## Page overview and document tabs

The page grid shows nearby thumbnails and the selected page. The status line
identifies the active book, inactive paper tab, and page total.

![Book thumbnails and document tabs][pages]

## Recent documents

This historical capture shows the hovered scan beside the selected paper.
Current cards show a first-page thumbnail, reading position, and file size.

![A hovered scan preview in recent documents][recent]

## Marks and return context

Local marks identify passages in one pane; global marks include a document.
Jump history keeps heading and search destinations in both directions.

![Local and global marks beside their document][marks]

![Heading and search destinations in jump history][jumps]

## Zoom, crop, and rotation

A cropped, rotated manual can sit beside its wrapped text in a narrow pane.
View changes leave source PDFs unchanged.

![A rotated view beside extracted text][zoom]

## Themes

The same manual and outline demonstrate a small set of built-in palettes.
[Theme notices][theme-notices] retain upstream attribution.

![Dark palette][theme-dark]

![Light palette][theme-light]

![Nord palette][theme-nord]

## OCR on a scan

Automatic page OCR makes the opening definitions in the EDVAC scan selectable.
The capture starts with a private empty content cache, waits for recognized
text, and selects a passage beside the source scan. Its recipe records the OCR
mode, engine, language, and cache conditions. Recognition leaves the PDF
unchanged. Disk caching and explicit requests are described in the
[OCR lesson][ocr-guide].

![Recognized text beside the original EDVAC scan][ocr]

The scan is MIT's copy of the report's IEEE reprint. Its title page credits
Michael D. Godfrey's corrected transcription. Capture verifies that extraction
is empty before recognition; document URLs and pinned hashes come from the
[public corpus script][corpus].

## Find and return

[![Find and return video poster][find-return-poster]][find-return-player]

[Play video][find-return-video]

Transcript: type `/multi-head`, then Enter to search. Press `n` for the next
match. Open results with `gs`; Enter follows the selected result. `Ctrl-o`
returns to the source passage. Setup opens the paper with cropped margins.

## Inspect a reference

[![Reference inspection video poster][inspect-reference-poster]][inspect-reference-player]

[Play video][inspect-reference-video]

Transcript: press `i`, then `l` to place the cursor on a citation. `K` opens its
preview; `j` and `k` scroll it. Escape closes the preview. `gd` follows the
reference, and `Ctrl-o` returns. Setup searches for `[13]` in the paper.

## Compare passages

[![Pane comparison video poster][compare-passages-poster]][compare-passages-player]

[Play video][compare-passages-video]

Transcript: press `Ctrl-w`, then `v` to split. `3gg` moves the new pane to page
three. `Ctrl-w`, then `p` returns focus. Type `8`, then `Ctrl-w`, then `>` to
widen that pane. `Ctrl-w`, then `=` equalizes; `Ctrl-w`, then `c` closes it.
Setup opens the paper with cropped margins.

## Select and capture

[![Selection and capture video poster][select-capture-poster]][select-capture-player]

[Play video][select-capture-video]

Transcript: `i` enters Cursor mode; `v` starts a selection and `2e` extends it.
`R` starts a rectangle. `8l` and `4j` resize it. Enter confirms; `l` chooses
Copy text and Enter runs it. Escape closes capture. Setup searches for
`Encoder` in the paper.

## Capture identity

Screenshots preserve their source dimensions. Video key strips sit below the
reading viewport and use the same monotonic timestamps as dispatched inputs.
Setup is separate from demonstrated inputs. Waits run at normal speed; these
clips contain no cuts or time compression. Posters link to stable player
comments. Direct H.264 links also render as inline players on GitHub.

The manifest separates capture from encoding: requested capture rate, delivered
frame rate and statuses, writer drops, delivery gaps, encoded rate, dimensions,
duration, overlays, and input timeline remain inspectable. ScreenCaptureKit does
not expose a source-drop counter; its record says unknown. Encoded frame rate
does not establish application frame rate or physical input latency.

These examples demonstrate reading states and interactions. They do not measure
performance, OCR accuracy, or clipboard delivery on another machine. Source
PDFs retain their authors' rights. See [text extraction][text-guide] for OCR
limits and the [development guide][capture] for regeneration and recovery.

[paper]: https://arxiv.org/abs/1706.03762
[book]: https://greenteapress.com/wp/think-python-2e/
[reading]: users/reading.md
[overview]: https://github.com/user-attachments/assets/feee9af7-54ff-45bc-aa03-64459bd70e3f
[help]: https://github.com/user-attachments/assets/1c5f1c1e-bd79-4031-97d5-1727b489185a
[search]: https://github.com/user-attachments/assets/af3490e1-844f-4dd8-8213-55a7f9083ab3
[preview]: https://github.com/user-attachments/assets/b533f77d-9bbb-4edc-bfd8-93ac4b2aa74b
[figure]: https://github.com/user-attachments/assets/cf953a6b-a559-4790-a53a-2cc49f3098e3
[mouse-menu]: https://github.com/user-attachments/assets/2fb1fa49-f091-4929-9d84-7069f9511c51
[mouse-hints]: https://github.com/user-attachments/assets/5b300331-1bda-4fc7-980f-776164d13a4b
[mouse-region]: https://github.com/user-attachments/assets/df82a7f1-03d5-4380-9657-b10cd6394185
[pages]: https://github.com/user-attachments/assets/59d78455-4174-484f-86f3-3915acc9fbe8
[recent]: https://github.com/user-attachments/assets/ea1b0baa-d065-4cbc-bbda-747aaa0fec10
[jumps]: https://github.com/user-attachments/assets/71764f60-821c-4710-818f-f95cf2b737dc
[ocr]: https://github.com/user-attachments/assets/eb32e49a-8255-42d7-84b8-8ede423861d3
[scan]: https://web.mit.edu/sts.035/www/PDFs/edvac.pdf
[manifest]: assets/capture.json
[capture]: dev/dev.md#gallery-capture
[text-guide]: dev/text.md
[corpus]: ../scripts/corpus.sh
[help-split]: https://github.com/user-attachments/assets/b0cd6933-0749-4186-a157-46f00cc48612
[definition]: https://github.com/user-attachments/assets/5e474613-97dc-4f5d-bfbb-8747cb1d9490
[continuous-text]: https://github.com/user-attachments/assets/283281bd-de4f-45f0-9dfa-c20f9c591f1b
[annotations]: https://github.com/user-attachments/assets/d31b8ba2-b314-4432-a64f-59208e3d82f8
[annotation-actions]: https://github.com/user-attachments/assets/336992ca-644c-404c-8556-fcb48bde363e
[figures]: https://github.com/user-attachments/assets/59ac8670-e069-4afa-a7a5-90ba5dcd9916
[marks]: https://github.com/user-attachments/assets/507418d7-c12d-4c2d-a7e6-7175bf72be7d
[zoom]: https://github.com/user-attachments/assets/cb420fb1-3839-482f-a773-a69ac5c01cfc
[theme-dark]: https://github.com/user-attachments/assets/1efcb184-6791-440b-9ad3-f3589d928d62
[theme-light]: https://github.com/user-attachments/assets/dc0b622f-4844-4546-a333-59361e5c01d7
[theme-nord]: https://github.com/user-attachments/assets/4abfdf74-e0e7-4446-a04f-4ce860ff6b60
[find-return-poster]: https://github.com/user-attachments/assets/9c0f3f9c-7c17-4591-90eb-277d463cac62
[find-return-video]: https://github.com/user-attachments/assets/d0c7a5bc-ddee-4c49-ab13-757e2fd4f964
[inspect-reference-poster]: https://github.com/user-attachments/assets/d2a9a301-ff79-4c11-90f6-c2064ceca1a9
[inspect-reference-video]: https://github.com/user-attachments/assets/4db2675d-c1b4-4e98-9e63-cce7a76cce0f
[compare-passages-poster]: https://github.com/user-attachments/assets/8442cbf5-f518-485d-bac9-2c7057dfb87c
[compare-passages-video]: https://github.com/user-attachments/assets/70907979-9791-4eea-966b-92385dab0530
[select-capture-poster]: https://github.com/user-attachments/assets/e3421241-9bc7-4b17-821d-49388c3eb5ad
[select-capture-video]: https://github.com/user-attachments/assets/87267719-61a1-4a35-81a4-46fd4743517d
[media]: assets/media.json
[host]: https://github.com/letientai299/vpdf/issues/9
[theme-notices]: licenses/themes.txt
[ocr-guide]: users/ocr.md
[find-return-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442164224
[inspect-reference-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442176726
[compare-passages-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442188323
[select-capture-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442422265
