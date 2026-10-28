# Ex commands

<!-- help-topic:commands -->

Press `:` to enter a command and Enter to run it. Escape cancels entry.
A command name can be followed by arguments separated by whitespace.
`:help :split` opens the generated entry for `:split`; `:help split` also
accepts its bare name. Use colon notation when a key or lesson has that name:
`:help :ocr` describes the command and `:help ocr` teaches recognition.

## Syntax and completion

`<file>` means a required filename; `[file]` means an optional filename. These
angle brackets and square brackets are reference notation; do not type them. In
`:tabp[revious]  :tabN`, the bracketed suffix is optional and `:tabN` is another
name for the same command. `:tabedit notes.pdf` opens that file. Paths are taken
from the argument text; use Tab completion rather than shell quoting or shell
expansion syntax. `:split` without a file duplicates the focused view.

Tab completes command names and supported arguments, including paths, themes,
recolor presets, OCR languages, and help topics. `:tabnext` completes open tab
numbers with file labels. `:ocr eng+` completes another installed language,
preserving selected languages. Up and Down recall prompt history. `Ctrl-u`
clears text before the cursor. `Ctrl-x Ctrl-e` edits command or search text in
the external editor selected by `VISUAL`, then `EDITOR`, then `vi`. Password
entry excludes the external editor. See [Installation and setup][install].

Exact command names and aliases win. A unique prefix can abbreviate a command;
use its complete name when prefixes become ambiguous. Page commands have their
own grammar: `:23` means physical page 23; `:iv` can name a printed label.
See [Navigation][navigation].

## Availability and related keys

The Built-in Ex commands section carries syntax, aliases, required helpers,
and examples. Commands come from the command prompt's registry and
cannot be customized. Other built-in references cover default keys and settings.
Look up a colon name even when an optional helper is missing: `:help :fzf`
explains its requirement while `:fzf` can be absent from command completion.

Opening commands, configuration, themes, help, version, and quit-all work from
the start screen. Reading commands need a document; page OCR and reload need a
PDF. `:print` is macOS only. Built-in help cannot print, reload from disk,
export a region, or hold persistent global marks.

`:split` and `:vsplit` correspond to `Ctrl-w s` and `Ctrl-w v`; `:close` to
`Ctrl-w c`; `:only` to `Ctrl-w o`; tab navigation to `gt` and `gT`.
`:toc`, `:annotations`, and `:overview` correspond to `go`, `ga`, and `gp`.
`:help! search` consults help beside a book; `Ctrl-w p` returns focus.
Bare `:new` and `:vnew` split with Recent; with a file, they match `:split`
and `:vsplit`.

## Common requests

Use `:edit book.pdf` to replace the focused document, or `:tabedit notes.pdf` to
keep it open. Bare `:edit` chooses a file for the focused pane; bare `:open` or
`:tabedit` chooses one for a new tab. The terminal picker requires fzf; the
macOS app uses its system chooser. `:recent` or `:enew` saves and unloads the
document, then browses saved files. `Ctrl-^` and `:e #` reopen the alternate;
`q` or Escape on Recent does the same from saved state. `:bd[elete]` unloads the
focused document and reopens the alternate, or shows Recent. On Recent or help,
`:bd` closes the pane. Space checks files and `x` requests removal. See
[Files and saved state][state]. `:fzf` discovers PDFs below the current
directory when its helper is installed. Tab completes paths after `:fzf `; a PDF
opens directly, and a directory sets the picker root.

Use `:cursorline off` to hide the cursor band, `:crop on` to crop margins,
`:text` to inspect extracted text, `:recolor sepia` for page colors,
and `:theme light` or `:theme dark` to change
UI colors. Use `:theme ~/.config/vpdf/themes/nord.toml` for a theme file; see
[UI themes][themes] for path resolution and fallback behavior.
`:config edit` opens the selected configuration file; `:config reload` rereads
it.

Use `:cancel` to stop pane jobs and `:nohlsearch` to hide search highlights.
`:reload` or `:e!` rereads the source PDF. `:password` opens hidden password
entry and reopens the document. `:ocr eng+vie` recognizes the page with
installed models. `:print` opens macOS printing for an unencrypted file-backed
PDF.

Use `:info` for document, reader, and terminal diagnostics; `:version` for build
identity; `:perf` for performance statistics. `:close` closes a pane,
`:tabclose` its entire tab, and `:qall` every tab and the reader.
`:q!`, `:x`, and `:wq` match `:q`; `:qa!` matches `:qall`. `ZZ` and `ZQ`
also close the focused pane.

This ends the teaching chapters. In the reader, continue into the generated
action, configuration, and command references, or return to the
[manual contents][home]. On GitHub, use the command lookups above in your
installed reader for the current interface.

[install]: install.md
[navigation]: nav.md
[home]: readme.md
[state]: reading.md
[themes]: config.md#ui-themes
