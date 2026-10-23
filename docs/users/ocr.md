# Scanned pages and OCR

<!-- help-topic:ocr -->

A scan can look like text while containing only page images. If Cursor mode
cannot find glyphs, `:text` is empty, and search misses visible words, try OCR.
A PDF with an existing text layer normally needs no recognition.

## Recognize a page

After the current image page is fully displayed, Vpdf checks it in the
background. With Tesseract and the configured language data installed, it
recognizes an image page without native text. Blank pages and pages with
embedded text are skipped. Opening and rendering never wait for the check.
Automatic failures stay silent; use an explicit request to see an error.

Type `:ocr` and Enter to recognize or retry the current page. `:ocr vie`
requests Vietnamese; `:ocr eng+vie` requests English and Vietnamese together.
Omitting the argument uses `ocr.language` from configuration. A running job
continues with the language it started with when configuration changes.

Recognition supplies temporary selectable text with approximate word and glyph
positions. Select and search it as you would embedded text. Check copied text
against the scan: recognition can confuse characters, columns, and equations.
The source PDF stays unchanged.

## Install the helper

Install [Tesseract][tesseract] using its upstream installation instructions,
including data for each requested language. Keep `tesseract` on `PATH`.
Run `tesseract --list-langs` in the shell to check language availability.
On macOS, `brew install tesseract` installs the engine; additional models can
require `brew install tesseract-lang`. On Debian or Ubuntu, install
`tesseract-ocr` and the required language package, such as `tesseract-ocr-eng`.

A missing executable or model prevents recognition, not PDF rendering. Install
it, return to the page, and retry `:ocr`. An unsupported language name is not
a request to download a model. See [Optional helpers][helpers].

## Progress and cancellation

OCR runs off the interaction thread. Automatic work yields to rendering and
stops when its pane moves away or needs new tiles. `:cancel` stops OCR and
disables automatic recognition for this document until reload. An explicit
`:ocr` can retry. Native work already in progress may take time to return.

Page text stays in memory while retained and disappears on eviction, reload,
or close. When disk caching is enabled and the document was not opened with a
password, recognized output can also be cached for reuse with the same image,
language, and models. `VPDF_CACHE=off` keeps extracted content in memory only.
Search sees embedded text and available OCR, not an automatically recognized
copy of every page in a scanned book.

Region OCR is different: `R`, Enter, OCR text recognizes a rectangle and copies
the result. It does not install a selectable page layer. Use page OCR when you
want to navigate and search recognized text in the reader.

[tesseract]: https://tesseract-ocr.github.io/tessdoc/Installation.html
[helpers]: install.md#optional-helpers
