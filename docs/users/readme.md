# Vpdf user manual

<!-- help-topic:introduction -->

Open help with `:help`, `g?`, or F1. It replaces the book in the current pane,
and the book becomes the alternate document. `:help!` opens beside the book.
Later lookups reuse help in the current tab.

With default bindings:

- Press `f`, then a link label, to follow [First session][first].
- Press `i` for Cursor mode; `Ctrl-]` follows a link there.
- Press `Ctrl-o` to go back; `Ctrl-i` goes forward.
- Type `/` to search, then Enter; `n` finds the next match.
- Type `:close` and Enter to return to the book.

`:help` followed by a key describes its built-in effect. `:h c-w` and `:h <c-w>`
open the `<C-w>` key entries. A prefix shared by several actions, such as `g`,
opens key notation. Modifier and named-key spellings accept either case; letter
keys keep their case. `:help helphelp` returns here. Vpdf leaves source PDFs
unchanged.

## How to read this manual

Read the chapters in order for a course in using the reader. The first
session uses the practice text in this manual;
no other PDF or optional helper is needed until you try clipboard delivery.

- [First session][first]: open help, follow, search, copy, and return.
- [Practice text][practice]: a passage, a reference, and a note.
- [Keys and input modes][keys]: notation, counts, and cancellation.
- [Navigation][navigation]: pages, printed labels, and the outline.
- [Zoom and narrow panes][zoom]: fit, crop, rotate, and text preview.
- [Search][search]: queries, results, and background work.
- [Links and notes][links]: follow, preview, and inspect comments.
- [Selecting and copying][selection]: motions, objects, and registers.
- [Figures and regions][regions]: capture and export a rectangle.
- [Scanned pages and OCR][ocr]: recognize text without changing the PDF.
- [Marks and jump history][marks]: keep places and return from detours.
- [Tabs and panes][panes]: compare passages and manage focus.
- [Configuring the reader][configuration]: bindings and reading defaults.
- [Files and saved state][state]: opening, persistence, reload, and caches.
- [Status and progress][status]: labels, jobs, and performance numbers.
- [Installation and setup][install]: terminals and optional helpers.
- [Troubleshooting][troubleshooting]: recover from common errors.
- [Ex commands][commands]: command syntax and reference lookup.

## Finding help

`:help search` opens a lesson. `:help :split` opens a command entry;
`:help split` accepts the command name too. `:help yy` looks up the built-in
key in the invoking mode. Use an explicit context for another surface, for
example `:help panel:j` or `:help search:<Tab>`. Tab completes topics; Enter
opens the selected candidate. Escape dismisses completion.

Help describes built-in keys and configuration defaults. Custom mappings and
configured values do not change it. Four generated references follow the
lessons:

- [Keys][keys]: `:help yf` lands on the `yf` entry; `:help y` then Tab lists
  sequences starting with `y` and their modes.
- [Actions][actions]: `:help actions` lists every configuration action with
  its fields, contexts, and a TOML example; `:help mark.jump` opens one.
- [Commands][commands]: `:help :split` opens one Ex command.
- [Settings][defaults]: `:help config` lists setting defaults;
  `:help reading.zoom` opens one setting.

`:config edit` opens your configuration file; see
[Configuring the reader][configuration].

Tab completes topic names and keys; it does not search descriptions. Inside
help, `/` searches the manual text, including generated descriptions. For
example, `/copy URL` finds the `yf` entry and its `hints.copy` action.

Plain `:help` always returns to this first page. `Ctrl-^` switches between help
and the book, and each keeps its reading position. `:close`, `:bd`, or `q`
closes help and reopens the book it replaced.

[first]: intro.md
[practice]: practice.md
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
[troubleshooting]: debug.md
[commands]: commands.md

<!-- help:defaults=config -->
<!-- help:actions=actions -->

[defaults]: config.md#built-in-defaults
[actions]: config.md#find-actions
