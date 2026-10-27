# Files and saved state

<!-- help-topic:state -->

Open a file with `vpdf book.pdf` from the shell. Multiple references open tabs;
`vpdf 'book.pdf#page=23'` overrides the saved position with physical page 23.
`vpdf --help`
shows the installed launch options. Without saved files, the start screen shows
opening and help hints. Use `:recent` for saved files or `:edit book.pdf` to
enter a path.

## Document references

Shell arguments and opening commands accept relative paths, absolute paths,
and local file URIs such as `file:///home/me/book.pdf#page=23`.
Relative inputs use the working directory. File URIs accept an empty host or
`localhost`; other hosts and remote downloads fail. URI paths use percent
encoding, including `%20` for spaces and `%23` for filename hashes. Ordinary
paths preserve spaces and literal percent signs. Quote references in the shell.
A bare `#` starts a target even when the full argument names an existing file.

Targets follow [PDF fragment identifiers][fragments]:

| Target                               | Example                           |
| ------------------------------------ | --------------------------------- |
| Physical page                        | `#page=23`                        |
| Named destination                    | `#nameddest=proof`                |
| Structure ID                         | `#structelem=section-id`          |
| Annotation name on the selected page | `#page=23&comment=note-id`        |
| Transient page rectangle             | `#page=23&highlight=10,100,20,50` |
| First match of any quoted word       | `#search=%22proof%20theorem%22`   |

Operations run from left to right; `&` and `#` separate parameters.
Highlight coordinates are left, right, top, bottom in points from the page's
upper-left corner. Rectangles require page view and do not change the PDF.
Unknown parameters, invalid pages, missing IDs, and unresolved searches report
errors. Parameters such as `zoom`, `view`, `ef`, and `fdf` are unsupported.

Each argument carries its own target, including when its document is already
open. Explicit targets apply after saved state and password entry; omitting a
target restores the saved view when opening a document. OS file events and
recent-file entries keep filesystem paths without fragments.

## Opening and discovery

`:edit book.pdf` opens in the focused pane; `:open book.pdf` and
`:tabedit book.pdf` open in a new tab. Omit the path to choose a file: the
terminal uses fzf when available and the macOS app uses its system file chooser.
Without fzf, `:open` starts path entry with Tab completion. Canceling keeps the
current view. `:tabnew` creates a recent-files tab. A file already open focuses
its existing pane.

In the terminal, `:fzf` finds PDFs below the current directory when fzf is on
`PATH`; use its upstream setup in [Optional helpers][helpers]. Without fzf, use
`:open` or enter a path with `:edit book.pdf` and Tab completion. With `:fzf `,
Tab completes PDFs and directories. A PDF opens directly; a directory limits the
picker to that tree.

Dropped PDF files open new tabs in the native app and terminals with file-drop
support, even while a prompt is open. Already open files focus their existing
views. Text drops and pastes still edit a prompt. Recent-file previews follow
the hovered row without changing the selected entry.

`:password` opens hidden password entry and reopens the focused PDF. Passwords
are not command-line arguments. Inspect `:info` after an open failure and check
the path, permissions, and password. Built-in help has no file to unlock.

The bar and percentage show the saved reading position, not pages read. Unknown
page counts show `…` until background loading finishes. Preview details retain
the exact saved page, even when a changed document has fewer pages.

The preview sits beside the list or below it in narrower windows. Drag its
divider to resize; double-click to reset. Press `Ctrl-r`, then `h/l` beside the
list or `j/k` below it; Escape leaves resize mode. The status bar shows `RESIZE`
while these keys move the divider. Each placement remembers its ratio across
window changes and restarts. Small windows omit the preview to keep navigation
and path entry usable.

## Removing recent files

Recent groups files by parent directory. Press Space to select a file or all
files in a directory. Directory marks show none (`○`), some (`◐`), or all (`●`)
selected. Enter or clicking a directory toggles its selection; Enter on a file
opens its saved position. Directory selection includes offscreen files.

Press `x` to remove checked files, or the focused file or directory's files when
none are checked. Enter, `y`, or a click on Remove confirms; `n`, Escape,
`Ctrl-c`, or Cancel cancels. Files hidden by a filter remain checked and count
toward removal. Escape outside confirmation clears checks before closing the
panel. Leaving Recent clears its checks.

Removal forgets saved positions, local marks, and outline choices. It preserves
source PDFs, open reading panes, and global marks. Saving an already open pane
does not restore its forgotten entry; explicitly reopening starts a new entry.
Removing the final file returns Recent to the opening and help hints.

## Saved state and reload

Reading positions, view choices, marks, outline choices, and prompt history
persist across launches. The performance toggle is saved too. These are separate
from the configuration file. Each document reopens with the sidebar it was left
with, or closed; Results, Jumps, Recent, and Version reopen closed. A saved
position does not restore tabs, splits, or running jobs. An explicit launch page
overrides the saved position.

State lives at `$XDG_STATE_HOME/vpdf/state.json`, falling back to
`~/.local/state/vpdf/state.json`. Missing or unreadable state starts empty.
State saves during activity and on exit; abrupt process or machine failure can
lose recent changes. Concurrent readers merge saved entries; removal prevents
older panes from restoring forgotten state. A later explicit open can add the
file again. Close readers before editing or backing up their state file.

On launch, a background scan removes saved positions and marks for paths the
filesystem reports as missing. Other access errors preserve saved entries.
Disconnected storage that reports missing paths can lose its saved state.

The reader reloads a changed PDF while retaining a useful position, view
choices, and marks. `:reload` requests this explicitly. Reload clears temporary
OCR and jump history. Hidden panes check for changes when shown again. Large
edits can move the passage beneath a retained position. Keep a separate copy
before replacing a source file if you need the original text or pagination.

## Content caches

On macOS, extracted-content caches live at `~/Library/Caches/vpdf/content`.
Search indexes, resolved outlines, figure lists, annotations, recent preview
cards, and eligible page OCR can persist there. These records can contain PDF
text. They are readable only by your account; PDFs opened with a password are
excluded. The system may purge caches at any time.

Set `VPDF_CACHE` to a directory to select another location, or to `off` to keep
extracted content in memory only. For example, launch
`VPDF_CACHE=off vpdf book.pdf` for a sensitive document. Linux requires an
explicit cache directory for disk caching. This option controls content caches;
it does not disable the separate saved reading state or prompt history.

Missing, damaged, or stale cache records fall back to PDF extraction. Page OCR
text can be evicted from memory and disappears on reload or close; eligible
cached recognition can avoid repeating Tesseract work on an unchanged image. A
cache is not a backup of the source PDF.

Contents, figures, and annotations prepare in the background after the first
complete view. Opening a panel requests its load immediately. `:cancel` stops
preparation until explicitly requested; it does not delete saved caches.

## Source boundaries

Reading, selection, marks, crop, rotation, recoloring, and OCR leave the source
PDF unchanged.
Region export creates a separate PNG. Copying uses the extracted text or image;
printing uses the source PDF. Reader crop, zoom, selections, and temporary OCR
do not change printed output.

Built-in help stays out of recent files, file watching, disk content caches, and
saved document state. Its identity is `builtin:help`. Its page colors stay fixed
across interface themes; `:recolor` changes them for the current view. It
supports session local marks, copying, and OCR; region export, printing, disk
reload, and persistent global marks are unavailable.

## External actions

On macOS, `:print` opens the system print dialog for an unencrypted file-backed
PDF. Choose pages, copies, and printer there. Cancel closes the dialog without
an error. Both frontends remain responsive; a logged-in graphical session is
required on the machine running Vpdf, including over SSH.

Followed HTTP, HTTPS, and mail links use the hosting machine's desktop opener.
Clipboard delivery has different local and SSH boundaries; see [Selecting and
copying][selection].

[helpers]: install.md#optional-helpers
[selection]: yank.md
[fragments]: https://www.rfc-editor.org/rfc/rfc8118.html#section-3
