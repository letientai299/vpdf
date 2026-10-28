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
OCR. The action bar highlights Copy text.

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

Hovering the scan previews its first page and file details beside the selected
paper. The list shows each document's reading position.

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

[paper]: https://arxiv.org/abs/1706.03762
[book]: https://greenteapress.com/wp/think-python-2e/
[reading]: users/reading.md
[overview]: https://github.com/user-attachments/assets/9622fbb2-be4c-4fec-a309-bf9ab040088b
[help]: https://github.com/user-attachments/assets/2a4d2a01-7e69-464a-b5f4-b4d09e90cdf8
[search]: https://github.com/user-attachments/assets/a7ba5556-8b83-4bba-a266-1536a214878f
[preview]: https://github.com/user-attachments/assets/4fd1093b-1ab0-496e-8544-aaa98dbae025
[figure]: https://github.com/user-attachments/assets/0517fe6d-871e-46d4-b822-6865e1a0a0fa
[mouse-menu]: https://github.com/user-attachments/assets/83beb354-8316-4e25-9fca-d66a102d534c
[mouse-hints]: https://github.com/user-attachments/assets/67e64c56-3757-4032-87b0-f1e28cf333a9
[mouse-region]: https://github.com/user-attachments/assets/7e82428d-8328-4e5f-8064-6c4207d65094
[pages]: https://github.com/user-attachments/assets/0506e0b7-a35c-4bba-98d7-8c13b393a0f0
[recent]: https://github.com/user-attachments/assets/bfe28dc8-af5b-4dd3-b58e-4a1572cf024b
[jumps]: https://github.com/user-attachments/assets/5e9f4775-ec56-4af0-a64b-78249e892401
[ocr]: https://github.com/user-attachments/assets/1223fd13-5fdc-4a70-aabc-1eeaa1a6aed3
[scan]: https://web.mit.edu/sts.035/www/PDFs/edvac.pdf
[manifest]: assets/capture.json
[corpus]: ../scripts/corpus.sh
[help-split]: https://github.com/user-attachments/assets/556091c8-0229-4357-a4d6-c136ed717b58
[definition]: https://github.com/user-attachments/assets/a359a06e-c261-4593-ab11-da19bd38170f
[continuous-text]: https://github.com/user-attachments/assets/d4b29ff4-6bbb-4c97-b680-4c2d5440dd3c
[annotations]: https://github.com/user-attachments/assets/dfd5dd81-2d95-48b9-a1e3-8b1b4e6c2fdc
[annotation-actions]: https://github.com/user-attachments/assets/eacfd20c-9807-4678-afa1-8c8ea0b5c054
[figures]: https://github.com/user-attachments/assets/baeb1158-bc3e-42c0-a226-04badf79694a
[marks]: https://github.com/user-attachments/assets/8eef889c-020f-4f99-adb5-95efcede1e6d
[zoom]: https://github.com/user-attachments/assets/c1edd09b-7457-4a3c-b206-ad0ae4e059ed
[theme-dark]: https://github.com/user-attachments/assets/984b2e33-3dfd-428b-b9ac-99ec9ff5794f
[theme-light]: https://github.com/user-attachments/assets/a3b1523a-93e6-4fb4-8ea1-f6fb6aaa7dff
[theme-nord]: https://github.com/user-attachments/assets/75da4ac4-dfb3-4f64-979f-0ab1e615d978
[find-return-poster]: https://github.com/user-attachments/assets/b6f78488-3d59-4d31-8f35-b59a35ff129d
[find-return-video]: https://github.com/user-attachments/assets/b0994a42-0c41-45c7-8db3-f0f68459bf9e
[inspect-reference-poster]: https://github.com/user-attachments/assets/661b402d-c0fc-485e-847e-dec202baf6f8
[inspect-reference-video]: https://github.com/user-attachments/assets/25c9c0c0-0241-4464-aa07-fcc2f766b63a
[compare-passages-poster]: https://github.com/user-attachments/assets/d353185f-39b7-48d0-a410-d35b346da0c4
[compare-passages-video]: https://github.com/user-attachments/assets/ed5d5dbe-d8c4-46ec-99c1-20ada18b7bed
[select-capture-poster]: https://github.com/user-attachments/assets/90bda437-acc3-4b60-ab05-0a01e9a8ba28
[select-capture-video]: https://github.com/user-attachments/assets/a9ef4104-f558-4c7c-b1e2-97d57f353bc2
[media]: assets/media.json
[host]: https://github.com/letientai299/vpdf/issues/9
[theme-notices]: licenses/themes.txt
[ocr-guide]: users/ocr.md
[find-return-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442164224
[inspect-reference-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442176726
[compare-passages-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442188323
[select-capture-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442422265
