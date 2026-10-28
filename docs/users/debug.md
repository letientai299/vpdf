# Troubleshooting

<!-- help-topic:troubleshooting -->

Start with `:info` in the affected pane. It reports the document, reader, and
terminal context; Render and Terminal start folded, so press `l` on either
heading or `zR` to open them. `:version` reports the build and bundled help
identity under a Build heading, folded in a document pane.
Record the exact message, command, frontend, and PDF when reporting a failure.
Avoid sharing a private PDF or its extracted text in a public report.

## Reader exits before opening

The terminal frontend checks graphics, keyboard disambiguation, and pixel cell
dimensions. Its error names missing capabilities. Fix the terminal or tmux
settings in [Terminal setup][terminal], then relaunch. `:text` still requires
those capabilities. The macOS native app can read without a compatible terminal.

For a missing file or permission error, check the filename and read access. Use
`:edit book.pdf` with Tab path completion. For an encrypted PDF, use `:password`
and the hidden prompt. A bad password or damaged PDF cannot be repaired by
changing zoom or switching to extracted text.

## A key has the wrong effect

Read the status label first. Escape dismisses a prompt, hint, preview, region,
or pending sequence before leaving Cursor or Visual. Tab changes panel focus;
Ctrl-w changes pane focus. Use `:help panel:j` to read about a built-in panel
key and `:help j` for the invoking context. Help describes built-in keys, not
your mappings. `:config reload` warns about rejected mappings. Launch with
`-u NONE` to compare shipped bindings.

Under tmux in Kitty, Ctrl-i can arrive as Tab. Use `:jumps` for a forward
history destination. The native reader and a directly compatible terminal
preserve that distinction.

## Missing text or wrong glyphs

Use `:text` to inspect extraction. An empty page can be a scan: try `:ocr`.
Missing Tesseract or language data affects recognition; run
`tesseract --list-langs` in the shell, install the requested data, then retry.
Automatic OCR failures are silent; explicit `:ocr` reports them.

A PDF that omits its fonts needs matching system fonts. Install the required
fonts; Linux also needs fontconfig and `fc-match`. Without them CJK can draw
wrong glyphs. OCR errors and extraction order can make copied text differ from
the page even when rendering looks correct. Compare a short copied passage
with the original before using it as a quotation.

## Empty outline or incomplete search

Wait for background preparation; opening CONTENTS explicitly requests it.
A PDF without an authored outline may receive inferred headings, or none.
Use physical page jumps and search when structure is unavailable. Search
results arrive incrementally; the pending indicator means the count can grow.
A scan contributes recognized pages, not text for the whole book automatically.

`:cancel` stops pane jobs. Start the corresponding search, panel, or explicit
OCR request to retry. `:nohlsearch` hides matches; `n` shows them again.
Change a regex to a short literal query when diagnosing a pattern error.

## Copy or export failed

Test a short text selection first. A yank message confirms extraction; check
pasting separately. Locally, check the clipboard helpers in
[Optional helpers][helpers]. Over SSH, check the local terminal's OSC 52
policy and multiplexer forwarding. Image copying needs separate image-clipboard
support and acknowledgement.

Save PNG writes beside the source PDF. Check write access there. Built-in help
cannot save regions or print; practice on a file-backed PDF. macOS printing
requires a graphical session and does not support encrypted PDFs. Crop and
OCR overlays do not alter printed source pages.

## Settings or state appear unchanged

Saving a configuration file that existed at launch reloads it; otherwise
`:config reload` rereads the originally selected file. Malformed TOML preserves
live settings; invalid individual entries warn while valid ones apply. Initial
zoom and cursorline affect newly opened documents. Saved and live pane values
can differ from configured defaults. `:help config` lists built-in defaults,
not the values in use.

A later launch restores eligible reading state and each document's sidebar, not
a full workspace of tabs and splits. An explicit launch page overrides the saved
position. Removing a Recent entry forgets its saved position and local marks,
including in open panes. Explicitly reopen the file to save a new entry. See
[Files and saved state][state].

[terminal]: install.md#terminal-setup
[helpers]: install.md#optional-helpers
[state]: reading.md
