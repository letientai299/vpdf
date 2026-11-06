# Demos

These captures use [Attention Is All You Need][paper],
[Think Python, second edition][book], the [EDVAC report scan][scan], and Vpdf's
embedded practice manual. The paper and book are by Ashish Vaswani and
colleagues and Allen B. Downey; the report is by John von Neumann. Accent
folding and Chinese search use the World Bank's [Việt Nam 2045][vi-report]
report in Vietnamese and the IMF's
[World Economic Outlook, October 2024][zh-report] in Chinese.
Open images at full resolution.

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

## Open documents

Recent starts with opening commands and later reopens documents at their last
position; the page grid and tabs move between pages and files.

### Recent documents

Hovering the scan previews its first page and file details beside the selected
paper. The list shows each document's reading position.

![A hovered scan preview in recent documents][recent]

### Page overview and document tabs

The page grid shows nearby thumbnails and the selected page. The status line
identifies the active book, inactive paper tab, and page total.

![Book thumbnails and document tabs][pages]

### Start screen

Without a file, Vpdf starts in Recent. With no saved files, it lists commands
to pick a PDF, open one by path, or read the manual.

![The start screen with its opening commands][home]

## Help and discovery

`:help` opens the manual; clues list the keys that follow a prefix.

### Built-in manual

`:help` opens the manual as a document. It supports the same outline, links,
search, and selection as a PDF. Close help to return to its invoking document.

![The built-in manual as a document with its outline][help]

`:help! panes` opens the manual beside a book, keeping both reading positions.

![Help beside Think Python in separate panes][help-split]

### Next-key clues

A pause after a prefix such as `g` lists the keys that can follow and their
actions.

![Clues listing the keys that follow g][clues]

## Read comfortably

Zoom, crop, rotation, text view, and palettes adapt pages to the pane and
reader.

### Zoom, crop, and rotation

A cropped, rotated manual can sit beside its wrapped text in a narrow pane.
View changes leave source PDFs unchanged.

![A rotated view beside extracted text][zoom]

### Continuous text

`:text` continues extracted text across page boundaries. The header names the
pages in view; a separator marks the next page.

![Extracted text spanning pages two and three][continuous-text]

### Themes

The same manual and outline demonstrate a small set of built-in palettes.
[Theme notices][theme-notices] retain upstream attribution.

![Dark palette][theme-dark]

![Light palette][theme-light]

![Nord palette][theme-nord]

### Page colors

`:recolor sepia` and `:recolor dark` repaint PDF pages in the focused view. The
top row shows sepia at 30, 60, and 100; the bottom row shows dark at the same
amounts. Diagram fills change with the text. The source PDF stays unchanged.

![Figure 1 in sepia and dark at three recolor amounts][recolor]

## Find text

Search highlights matches on the page and lists them as results.

### Search

Results list matching passages and highlight the phrase on the rendered page.

![Search results with a highlighted Multi-Head match][search]

### Find and return

[![Find and return video poster][find-return-poster]][find-return-player]

[Play video][find-return-video]

Transcript: type `/multi-head`, then Enter to search. Press `n` for the next
match. Open results with `gs`; Enter follows the selected result. `Ctrl-o`
returns to the source passage. Setup opens the paper with cropped margins.

### Vietnamese and Chinese text

Search folds Vietnamese diacritics: `kinh te` matches `kinh tế` 274 times in
the World Bank report.

![Vietnamese matches for kinh te in results and on the page][search-vi]

`经济` finds 1,084 matches in the IMF report.

![Chinese matches for 经济 in results and on the page][search-zh]

## Links and references

Hints and `gd` follow links and labels; `K` previews a target in place.

### Point at link hints

Pointing at a hint picks it while other labels stay visible. Enter or a click
follows the destination.

![Link hints with one picked by the pointer][mouse-hints]

### Link underlines

`:links on` underlines every link in the pane. `f` then labels each visible
link; typing a label follows it.

![Underlined citations with hint labels][hints]

### Follow link hints

[![Follow link hints video poster][follow-hints-poster]][follow-hints-player]

[Play video][follow-hints-video]

Transcript: press `f` to label visible links, then `a` to follow reference 13.
`yf` labels links to copy; `a` copies an arXiv address. `Ctrl-o` returns to
the citation.

### Reference preview

`K` previews an internal reference without moving the source passage. The
preview can scroll beyond its target page.

![A bibliography preview over its source citation][preview]

`gd` also resolves recognizable unlinked references. Here it finds the caption
for a body reference to Figure 1.

![A resolved Figure 1 reference with its destination highlighted][definition]

### Inspect a reference

[![Reference inspection video poster][inspect-reference-poster]][inspect-reference-player]

[Play video][inspect-reference-video]

Transcript: press `i`, then `l` to place the cursor on a citation. `K` opens its
preview; `j` and `k` scroll it. Escape closes the preview. `gd` follows the
reference, and `Ctrl-o` returns. Setup searches for `[13]` in the paper.

## Move through a document

Marks, jump history, contents, and the scrollbar move between passages.

### Marks and return context

Local marks identify passages in one pane; global marks include a document.
Jump history keeps heading and search destinations in both directions.

![Local and global marks beside their document][marks]

![Heading and search destinations in jump history][jumps]

### Contents tree

`go` opens CONTENTS. `zM` folds every branch and `l` reopens the selected one;
folded rows count their hidden entries. A bar marks the row that holds the
current section.

![A folded contents tree with the current section marked][contents]

### Fold the contents

[![Fold the contents video poster][contents-fold-poster]][contents-fold-player]

[Play video][contents-fold-video]

Transcript: `go` opens CONTENTS and `zM` collapses every branch. `2j` selects
Training and `l` opens it; `3j` selects Optimizer. Enter jumps to the section
and `Ctrl-o` returns to the earlier page.

### Scrollbar

Scrolling or resting the pointer on the right edge shows a scrollbar. Ticks
mark top-level headings and pages with matches; dragging the thumb names the
page and section.

![A scrollbar drag label over heading and match ticks][scrollbar]

## Select and copy

Keyboard and mouse selections copy text and capture regions.

### Mouse context menu

A mouse drag selects text; right-click opens selection actions. Hover chooses a
row and a click runs it. The menu also supports dragging from its opening press
and releasing on an action.

![Selection context menu with a hovered action][mouse-menu]

### Select and capture

[![Selection and capture video poster][select-capture-poster]][select-capture-player]

[Play video][select-capture-video]

Transcript: `i` enters Cursor mode; `v` starts a selection and `2e` extends it.
`R` starts a rectangle. `8l` and `4j` resize it. Enter confirms; `l` chooses
Copy text and Enter runs it. Escape closes capture. Setup searches for
`Encoder` in the paper.

## Figures, regions, and notes

Detected figures and swept regions open capture actions; PDF comments stay
readable.

### Figures and tables

FIGURES lists detected diagrams and tables by caption. Its selected destination
can remain beside the document.

![Figure and table list beside the selected diagram][figures]

### Figure capture

A detected figure offers image copying, text copying, PNG export, and optional
OCR. The action bar highlights Copy text.

![A selected diagram with its capture actions][figure]

### Sweep a capture region

Alt-drag dims the page outside the rectangle. Releasing opens capture actions;
the status line explains finishing and cancelling the sweep.

![A held Alt-drag with its release and cancel hint][mouse-region]

### Annotation comments

The embedded practice manual supplies an authored note. Its annotation list,
full comment, hover author, and context actions leave the source unchanged.

![The practice note with its comment and author][annotations]

![Show note and Copy comment actions for the practice note][annotation-actions]

## Panes

Splits show two passages at once; panes resize and close by key.

### Compare passages

[![Pane comparison video poster][compare-passages-poster]][compare-passages-player]

[Play video][compare-passages-video]

Transcript: press `Ctrl-w`, then `v` to split. `3gg` moves the new pane to page
three. `Ctrl-w`, then `p` returns focus. Type `8`, then `Ctrl-w`, then `>` to
widen that pane. `Ctrl-w`, then `=` equalizes; `Ctrl-w`, then `c` closes it.
Setup opens the paper with cropped margins.

## Scans and OCR

Recognition turns scanned pages into text without changing the PDF.

### OCR on a scan

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

## macOS app

The app shares the reader and adds native window controls.

### Toolbar and Find

The toolbar holds the sidebar toggle, page field, zoom, fit, and Find. Find
starts a document search; here `attention` matches 97 times.

![The app toolbar above highlighted attention matches][app-toolbar]

### Contents sidebar

The sidebar button shows CONTENTS beside the page.

![The app with its contents sidebar beside the paper][app-sidebar]

[paper]: https://arxiv.org/abs/1706.03762
[book]: https://greenteapress.com/wp/think-python-2e/
[reading]: users/reading.md
[overview]: https://github.com/user-attachments/assets/f76fe814-c5bf-4631-b623-271f516d4eee
[help]: https://github.com/user-attachments/assets/2a4d2a01-7e69-464a-b5f4-b4d09e90cdf8
[search]: https://github.com/user-attachments/assets/a7ba5556-8b83-4bba-a266-1536a214878f
[preview]: https://github.com/user-attachments/assets/ba24a6d3-1998-4c8c-87de-cdabf3ed94c8
[figure]: https://github.com/user-attachments/assets/0517fe6d-871e-46d4-b822-6865e1a0a0fa
[mouse-menu]: https://github.com/user-attachments/assets/83beb354-8316-4e25-9fca-d66a102d534c
[mouse-hints]: https://github.com/user-attachments/assets/aa9a70d0-48e5-4315-993d-5432b8e82ad0
[mouse-region]: https://github.com/user-attachments/assets/7e82428d-8328-4e5f-8064-6c4207d65094
[pages]: https://github.com/user-attachments/assets/0506e0b7-a35c-4bba-98d7-8c13b393a0f0
[recent]: https://github.com/user-attachments/assets/bfe28dc8-af5b-4dd3-b58e-4a1572cf024b
[jumps]: https://github.com/user-attachments/assets/5e9f4775-ec56-4af0-a64b-78249e892401
[ocr]: https://github.com/user-attachments/assets/1223fd13-5fdc-4a70-aabc-1eeaa1a6aed3
[scan]: https://web.mit.edu/sts.035/www/PDFs/edvac.pdf
[manifest]: assets/capture.json
[corpus]: ../tools/corpus/corpus.sh
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
[inspect-reference-poster]: https://github.com/user-attachments/assets/bbf256cc-4ee3-426c-b364-2e33d343caf4
[inspect-reference-video]: https://github.com/user-attachments/assets/b1ea51ac-55f8-4d77-bf90-24ab206ce667
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
[recolor]: https://github.com/user-attachments/assets/8a184f35-0fda-425c-bc62-88f5fd69a336
[home]: https://github.com/user-attachments/assets/730a2abe-4c95-4442-a7bb-350543e49b6d
[clues]: https://github.com/user-attachments/assets/3135d5ed-14ac-4c34-8edd-dbe41a9f2a0f
[search-vi]: https://github.com/user-attachments/assets/267d8e90-5a22-4575-985d-0e212a6513c3
[hints]: https://github.com/user-attachments/assets/ad42c579-f384-4317-b364-781fbd5a776b
[contents]: https://github.com/user-attachments/assets/49be8683-0daf-41dc-98cd-cd307d3ed4cb
[scrollbar]: https://github.com/user-attachments/assets/6cd84e65-fc2e-4e7f-b2af-34552332edba
[vi-report]: https://documents1.worldbank.org/curated/en/099051625142537135/pdf/P178784-a1508fbd-9062-44cf-a256-f70aaed00704.pdf
[search-zh]: https://github.com/user-attachments/assets/cf45e5be-bb87-42a9-acdb-0855c3675dc9
[zh-report]: https://www.imf.org/-/media/files/publications/weo/2024/october/chinese/text.pdf
[follow-hints-poster]: https://github.com/user-attachments/assets/28a2aa0e-0288-4ead-a995-2297527d59de
[follow-hints-video]: https://github.com/user-attachments/assets/8d1c0df3-c410-4150-ab50-95a73d034714
[follow-hints-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6683452578
[contents-fold-poster]: https://github.com/user-attachments/assets/ec6268a1-8252-4412-8ef3-03dd7d4e1647
[contents-fold-video]: https://github.com/user-attachments/assets/90b23594-7c0a-49eb-b0ab-dc09fa29584d
[contents-fold-player]: https://github.com/letientai299/vpdf/issues/9#issuecomment-6683470547
[app-toolbar]: https://github.com/user-attachments/assets/a16cbb09-b8e3-4be5-bcdc-c36dec09af64
[app-sidebar]: https://github.com/user-attachments/assets/92022bf6-3e11-4614-b14a-1ac59e6ab52d
