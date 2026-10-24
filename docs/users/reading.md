# Files and saved state

<!-- help-topic:state -->

Open a file with `vpdf book.pdf` from the shell. Multiple filenames open tabs;
`--page 23` overrides the saved position with physical page 23. `vpdf --help`
shows the installed launch options. Without saved files, the start screen shows
opening and help hints. Use `:recent` for saved files or `:edit book.pdf` to
enter a path.

## Opening and discovery

`:edit book.pdf` opens in the focused pane; `:open book.pdf` and
`:tabedit book.pdf` open in a new tab. Omit the path to choose a file: the
terminal uses fzf and the macOS app uses its system file chooser. Canceling
keeps the current view. `:tabnew` creates a recent-files tab. A file already
open focuses its existing pane.

In the terminal, `:fzf` finds PDFs below the current directory when fzf is on
`PATH`; use its upstream setup in [Optional helpers][helpers]. Without fzf,
enter a path with `:edit book.pdf` and Tab completion.

Dropped PDF files open new tabs in the native app and terminals with file-drop
support, even while a prompt is open. Already open files focus their existing
views. Text drops and pastes still edit a prompt. Recent-file previews follow
the hovered row without changing the selected entry.

`:password` opens hidden password entry and reopens the focused PDF. Passwords
are not command-line arguments. Inspect `:info` after an open failure and check
the path, permissions, and password. Built-in help has no file to unlock.

## Removing recent files

In Recent, press Space to check or uncheck a file without moving the cursor.
Press `x` to remove checked files, or the cursor row when none are checked.
Enter or `y` confirms; `n`, Escape, or `Ctrl-c` cancels. Files hidden by a
filter remain checked and count toward removal. Escape outside confirmation
clears checks before closing the panel. Leaving Recent clears its checks.

Removal forgets saved positions and local marks. It preserves source PDFs,
open reading panes, and global marks. Saving an already open pane does not
restore its forgotten entry; explicitly reopening the file starts a new entry.
Removing the final file returns Recent to the opening and help hints.

## Saved state and reload

Reading positions, view choices, marks, and prompt history persist across
launches. The performance toggle is saved too. These are separate from the
configuration file. A saved position does not restore a layout of tabs, splits,
panels, or running jobs. An explicit launch page overrides the saved position.

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
Search indexes, resolved outlines, figure lists, annotations, current-page
links, recent preview cards, and eligible page OCR can persist there. These
records can contain PDF text. They are readable only by your account; PDFs
opened with a password are excluded. The system may purge caches at any time.

Set `VPDF_CACHE` to a directory to select another location, or to `off` to
keep extracted content in memory only. For example, launch
`VPDF_CACHE=off vpdf book.pdf` for a sensitive document. Linux requires an
explicit cache directory for disk caching. This option controls content
caches; it does not disable the separate saved reading state or prompt history.

Missing, damaged, or stale cache records fall back to PDF extraction. Page OCR
text can be evicted from memory and disappears on reload or close; eligible
cached recognition can avoid repeating Tesseract work on an unchanged image.
A cache is not a backup of the source PDF.

Contents, figures, annotations, and links prepare in the background after the
first complete view. Opening a panel requests its load immediately. `:cancel`
stops preparation until explicitly requested; it does not delete saved caches.

## Source boundaries

Reading, selection, marks, crop, rotation, and OCR do not modify the source PDF.
Region export creates a separate PNG. Copying uses the extracted text or image;
printing uses the source PDF. Reader crop, zoom, selections, and temporary OCR
do not change printed output.

Built-in help stays out of recent files, file watching, disk content caches,
and saved document state. Its identity is `builtin:help`. It supports session
local marks, copying, and OCR; region export, printing, disk reload, and
persistent global marks are unavailable.

## External actions

On macOS, `:print` opens the system print dialog for an unencrypted file-backed
PDF. Choose pages, copies, and printer there. Cancel closes the dialog without
an error. Both frontends remain responsive; a logged-in graphical session is
required on the machine running Vpdf, including over SSH.

Followed HTTP, HTTPS, and mail links use the hosting machine's desktop opener.
Clipboard delivery has different local and SSH boundaries; see
[Selecting and copying][selection].

[helpers]: install.md#optional-helpers
[selection]: yank.md
