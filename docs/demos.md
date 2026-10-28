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

[paper]: https://arxiv.org/abs/1706.03762
[book]: https://greenteapress.com/wp/think-python-2e/
[reading]: users/reading.md
[overview]: https://github.com/user-attachments/assets/2be01fa8-8b03-48c9-b135-32458351aa2a
[help]: https://github.com/user-attachments/assets/237e8c23-e0b6-4644-8280-50514f7bbf22
[search]: https://github.com/user-attachments/assets/58754861-bb18-4ba4-8e23-68e54630093d
[preview]: https://github.com/user-attachments/assets/003bf1e3-85ab-4f52-8ac9-e826a866115c
[figure]: https://github.com/user-attachments/assets/70f257dd-b0c9-4979-a7a9-74620f89650d
[mouse-menu]: https://github.com/user-attachments/assets/b5b6619a-498c-42b3-8261-a2a957c5ad5a
[mouse-hints]: https://github.com/user-attachments/assets/b5272ec1-f8ca-42dc-9ef1-4f6dd5e14331
[mouse-region]: https://github.com/user-attachments/assets/8dbd7958-af0b-4bb6-88cd-6a6c81b56209
[pages]: https://github.com/user-attachments/assets/5b2e216b-6c85-482b-a1ab-2c386ea9b967
[recent]: https://github.com/user-attachments/assets/3cf580dc-1686-4b1a-86b6-ac23ccd3ab38
[jumps]: https://github.com/user-attachments/assets/7540c436-f636-444f-9722-48fdc4a4812e
[ocr]: https://github.com/user-attachments/assets/bb3a5bf2-755a-4a2f-b641-2f9f6d32e133
[scan]: https://web.mit.edu/sts.035/www/PDFs/edvac.pdf
[manifest]: assets/capture.json
[corpus]: ../tools/corpus/corpus.sh
[help-split]: https://github.com/user-attachments/assets/aff76a07-0ddd-4975-a767-64be70a04743
[definition]: https://github.com/user-attachments/assets/c56ec85d-a5a1-4e09-8181-564766c2d790
[continuous-text]: https://github.com/user-attachments/assets/bebfc118-8bad-4ff1-8813-b9844bd9468f
[annotations]: https://github.com/user-attachments/assets/5a3343e5-8fa3-4592-8e05-e557591122c0
[annotation-actions]: https://github.com/user-attachments/assets/8cb57837-e6bf-4800-b967-058cab0c9a01
[figures]: https://github.com/user-attachments/assets/476fba9a-4297-4f5e-a48d-d08882cb021e
[marks]: https://github.com/user-attachments/assets/af502147-5324-41da-a190-f8a9f34f3ea3
[zoom]: https://github.com/user-attachments/assets/06b534dc-5ba0-400c-aa1e-61b0f51b07e7
[theme-dark]: https://github.com/user-attachments/assets/c8435906-7b90-4c35-9e35-0bd2961b9e77
[theme-light]: https://github.com/user-attachments/assets/1741672e-03be-4ddf-a419-e2263868d9a6
[theme-nord]: https://github.com/user-attachments/assets/c6489a93-15e0-4cd5-a90e-6c917130516e
[find-return-poster]: https://github.com/user-attachments/assets/fb7867b8-bef0-4050-8714-c8d129dbe9f5
[find-return-video]: https://github.com/user-attachments/assets/d0c7a5bc-ddee-4c49-ab13-757e2fd4f964
[inspect-reference-poster]: https://github.com/user-attachments/assets/e6d251d5-532f-43ef-ad2e-3fb9e8fddec4
[inspect-reference-video]: https://github.com/user-attachments/assets/4db2675d-c1b4-4e98-9e63-cce7a76cce0f
[compare-passages-poster]: https://github.com/user-attachments/assets/9fe6c0fd-d6ff-40e8-8094-9f5710698466
[compare-passages-video]: https://github.com/user-attachments/assets/70907979-9791-4eea-966b-92385dab0530
[select-capture-poster]: https://github.com/user-attachments/assets/334bd685-2bdc-40ea-89cb-1c9de301ee8c
[select-capture-video]: https://github.com/user-attachments/assets/87267719-61a1-4a35-81a4-46fd4743517d
[media]: assets/media.json
[host]: https://github.com/letientai299/vpdf/issues/9
[theme-notices]: licenses/themes.txt
[ocr-guide]: users/ocr.md
[find-return-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442164224
[inspect-reference-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442176726
[compare-passages-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442188323
[select-capture-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6442422265
