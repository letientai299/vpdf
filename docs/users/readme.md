# Vpdf user manual

<!-- help-topic:introduction -->

Open help with `:help`, `g?`, or F1. By default it opens a tab;
`:help!` opens beside the book. Later lookups reuse help in the current tab.

With default bindings:

- Press `f`, then a link label, to follow [First session][first].
- Press `i` for Cursor mode; `Ctrl-]` follows a link there.
- Press `Ctrl-o` to go back; `Ctrl-i` goes forward.
- Type `/` to search, then Enter; `n` finds the next match.
- Type `:close` and Enter to return to the book.

For customized bindings, use `:help` followed by a key.
`:help helphelp` returns here. Vpdf leaves source PDFs unchanged.

## How to read this book

Read the chapters in order for a course in using the reader. The first
session uses the practice text in this book;
no other PDF or optional helper is needed until you try clipboard delivery.

1. [First session][first]: open help, follow, search, copy, and return.
2. [Practice text][sample]: a passage, a reference, and a note.
3. [Keys and input modes][keys]: notation, counts, and cancellation.
4. [Navigation][navigation]: pages, printed labels, and the outline.
5. [Zoom and narrow panes][zoom]: fit, crop, rotate, and text preview.
6. [Search][search]: queries, results, and background work.
7. [Links and notes][links]: follow, preview, and inspect comments.
8. [Selecting and copying][selection]: motions, objects, and registers.
9. [Figures and regions][regions]: capture and export a rectangle.
10. [Scanned pages and OCR][ocr]: recognize text without changing the PDF.
11. [Marks and jump history][marks]: keep places and return from detours.
12. [Tabs and panes][panes]: compare passages and manage focus.
13. [Configuring the reader][configuration]: bindings and reading defaults.
14. [Files and saved state][state]: opening, persistence, reload, and caches.
15. [Status and progress][status]: labels, jobs, and performance numbers.
16. [Installation and setup][install]: terminals and optional helpers.
17. [Troubleshooting][troubleshooting]: recover from common errors.
18. [Ex commands][commands]: command syntax and reference lookup.

## Finding help

`:help search` opens a lesson. `:help :split` opens a command entry;
`:help split` accepts the command name too. `:help yy` looks up the built-in
key in the invoking mode. Use an explicit context for another surface, for
example `:help panel:j` or `:help search:<Tab>`. Tab completes topics; Enter
opens the selected candidate. Escape dismisses completion.

The lessons and generated keybinding reference describe built-in shortcuts.
Custom mappings do not change the reference. The [setting defaults][defaults]
(`:help config`) list built-in configuration values.

Plain `:help` always returns to this first page. Switching back to the existing
help tab retains its reading position. Close the final help pane with `:close`
to resume the invoking pane when it remains open.

[first]: intro.md
[sample]: sample.md
[keys]: keys.md
[navigation]: nav.md
[zoom]: zoom.md
[search]: search.md
[links]: links.md
[selection]: yank.md
[regions]: regions.md
[ocr]: ocr.md
[marks]: marks.md
[panes]: panes.md
[configuration]: config.md
[state]: reading.md
[status]: status.md
[install]: install.md
[troubleshooting]: troubleshooting.md
[commands]: commands.md

<!-- help:defaults=config -->

[defaults]: config.md#built-in-defaults
