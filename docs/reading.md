# Reading a PDF

Open a PDF using the [installation guide][installation]. Use `:help` in the
reader for current bindings and commands. This guide assumes familiarity with
Vim motions. The [demos][demo] show these workflows on real documents.

## View, position, select

Normal mode moves the viewport. Scrolling and zooming change what is visible
without turning the page into a text editor or changing the PDF.

Cursor mode places a glyph cursor in the text. Move to the word, sentence,
or paragraph of interest, then enter Visual mode to select a range. Visual
selection can start directly from Normal too. Copy the selection to use its
text elsewhere; the source PDF stays unchanged.

Pointer selection follows the same reading modes. Drag across text to select
it, or use repeated clicks to select a word, line, or paragraph. Holding a drag
past the top or bottom edge keeps scrolling, faster farther out, until release,
a move back inside, or a key. A right click opens a menu of actions for what
is under the pointer; see [mouse][mouse].

A middle drag pans a zoomed page or the extracted text. On rendered pages, not
the extracted text, a scrollbar shows while the view moves or while the
pointer rests on the right edge, with ticks for top-level headings and pages
with search matches. A click on the track jumps there and records the jump;
dragging the thumb scrubs, labeling the page under the pointer and the section
at its top, as the status line names it while reading.

Glyph positions and the viewport are separate. Scrolling can move a selected
passage out of view without discarding it. Zooming and resizing keep reading
positions tied to document content. Leaving Visual returns to the mode that
started the selection. Escape dismisses a pending interaction before leaving
a reading mode.

Temporary interactions take input while preserving the reading mode beneath
them. Region capture and its action bar are one such interaction. Commands,
search entry, link hints, keyboard previews, menus, the page overview, and
focused panels also change what keys do. The status line identifies the active
interaction; `:help` explains its labels, including line selection and pending
yank operations. Rendered pages and extracted text share the reading modes.
Hover previews and held pan or resize gestures do not add keyboard modes.

Text selection follows the PDF's extracted glyphs and reading order. Complex
columns or missing text can make that order differ from the visual page.
The [text guide][text] explains copying fidelity and optional OCR for scans.
Page OCR text stays in memory, can be evicted, and disappears when the document
reloads or closes. Where search indexes persist (below), Tesseract's output is
also stored, so running page OCR again on an unchanged page skips recognition.
Vpdf does not embed OCR text in the source PDF.

## Find a passage and return

Browse the outline for document structure or search for a phrase. A side panel
keeps destinations visible beside the page. Keyboard input follows focus: a
focused list navigates rows, while focused content navigates the document.
Following a row can return focus to reading or keep it in the list for comparing
destinations. In the status line, the section name opens the outline at that
entry and the page counter opens the page overview; key hints and completion
lists also take clicks. Switching focus preserves the current selection. In
recent documents, the preview card follows the hovered row, so inspecting
another file does not change the selected entry.

Preview an internal reference to inspect its destination before following it.
The preview is a temporary reading surface with its own scrolling, which runs on
past the target page into the following pages. Hovering an internal link can
open it without moving the reading position. It opens in the pane under the
pointer, stays while the pointer travels from the link into it, and closes when
the pointer leaves both or the link leaves the view. A left click inside any
link preview follows it, a middle click shows its target in a split, and the
status line keeps its destination while the pointer is over it. A web address
wrapped onto the next line, column, or page highlights whole, and link lists
show a line or column wrap once. The status line names what the pointer rests
on: a link's destination, the reference a citation cites, a note, a clipped row,
or a thumbnail's page and the section at its top. Messages and errors stay
until the next key or click; the detail shows when none is up and takes room
from the tab names. Link labels can also be picked with the pointer
before following their destination. Each label sits beside its link, on the
link's line, clear of other links and labels. Jump history returns to earlier
destinations and their views; marks keep places worth revisiting. A jump to a
place clear of the screen's edge rows, such as a mark, search match, or link
target, keeps the view; one elsewhere shows the target one row below the top,
in both presentations. A history entry from the other presentation follows the
same rule.
Page jumps, such as `:12` or a thumbnail, show the page from its top.
Following an external web link opens it through the desktop integration. See
[external actions][external] for clipboard and SSH boundaries.

Hovering a note or commented highlight shows its author and comment in the
status line. A click on a note icon opens the whole comment beside it; the
right-click menu on any note offers Show note and Copy comment.

## Compare views

Use tabs to keep multiple documents open. Split a pane to compare passages in
one PDF, or place another document alongside it. Each pane keeps its own
position, selection, search, and panel focus, so navigating one view leaves
the other in place. Pointing and scrolling over a surface can move that
surface without changing keyboard focus.

Dropping a PDF opens it in a new tab in the native app and terminals with
file-drop support, including while a prompt is open. A PDF already open
focuses its existing view. Text drops and pastes still edit an open prompt.
Keyboard commands such as `:e` and `:fzf` open files in the focused pane.

A middle click on an internal link, a panel row, or a thumbnail shows that
place in a split beside the pane, which keeps focus; on a web link it copies
the address. Drag the separator between panes, the status line between stacked
panes, or the side panel's edge to resize them; a double click evens the split
or restores the panel's width. Tab names in the status line switch tabs on a
click and close on a middle click.

For saved positions and live reload, see [saved state and reload][saved-state].

## Take text or a figure

Use rendered pages when layout and figures matter. Extracted-text presentation
scrolls on through the document, with a rule naming each next page, and
uses the same reading modes for navigating and selecting text, with the
fidelity limits described in the [text guide][text].

Select a detected figure or sweep a region to capture part of a page. Its action
bar offers image copying, text copying, PNG export, and optional OCR. Use the
keyboard to start a rectangle at the glyph cursor in Cursor mode, or at the view
center otherwise. Resize a corner or move the whole rectangle, then confirm to
open the same actions; see `:help` for bindings. REGION edits the rectangle;
ACTIONS chooses an operation. Return to editing to resize it again. The
rectangle stays within its page. Swap corners to move the anchor, or move
through it to capture in the opposite direction. Thicker borders mark the
active corner; swapping corners moves the emphasis. Point at an action to choose
it, then click to run it. The status line shows REGION from the start of capture
and explains how to finish or cancel. Closing capture restores the underlying
reading mode. Region OCR copies recognized text; page OCR supplies temporary
selectable text for reading. Both leave the source PDF unchanged. Clipboard
support and OCR dependencies follow the [installation guide][helpers].

## Mouse

The pointer works on every surface: pages, extracted text, panels, the page
overview, previews, menus, and the status line. Hover only informs: it tints
links, names what the pointer rests on, and can preview an internal link, but it
never moves the text selection or keyboard focus. A press focuses the pane or
panel under it; the wheel scrolls whatever is under the pointer, or the pane
above a status line, and leaves focus in place. The `Mouse` sections of `:help`
list each gesture.

In Kitty and Ghostty the reader takes
[pointer positions in pixels][pixel-mouse], so adjacent citations and small
markers hit separately; under tmux positions are whole cells. Where the terminal
supports [pointer shapes][pointer-shapes], the pointer shows a hand over things
a click acts on, a text cursor over text, resize arrows on dividers and the
panel edge, a crosshair while capturing a region, and a grab while panning.

A right click opens a menu at the pointer. On a page it offers actions for the
link, note, selection, word, or figure there, then common views. On a panel row
it offers following the row, opening it in a split, copying it, and closing the
panel; on a thumbnail it offers opening the page here or in a split. Hover or
`j` and `k` choose an item, and a click or Enter runs it. Items with a key show
it, so the menu also teaches the keyboard.

Kitty keeps Shift-click and Shift-drag for its own selection while a program
uses the mouse; see its [mouse actions][kitty-mouse]. `:mouse off` hands every
click and drag to the terminal, so its selection works without a modifier;
`:mouse on` takes the pointer back. Under tmux they go to tmux: with its `mouse`
option on, a drag selects in tmux's copy mode; with it off, the outer terminal
selects. For Alt-drag capture, tmux may need the optional
[forwarding binding][tmux-drag]. The choice holds across child programs, such as
the prompt editor, and shows in `:info`. macOS can turn Ctrl-click into a right
click; no gesture uses Ctrl-click.

Known limits:

- Past the window's top edge Kitty reports the top row, so a held drag there
  scrolls at its slowest pace.
- Outside tmux the reader does not track window focus. A held drag or pan
  whose release never arrives ends at the next pointer motion; a key also ends
  a text drag.
- Without pixel reports, as under tmux, a drag held past an edge scrolls at
  one pace.
- A middle drag that starts over a hover preview opens the preview's target in
  a split instead of panning.

## Saved state and reload

Reading positions, view choices, marks, and prompt history persist across
launches. A saved position does not restore a full layout of tabs, splits,
panels, and running jobs. An explicit launch page overrides the restored
position.

The state file lives at `$XDG_STATE_HOME/vpdf/state.json`, falling back to
`~/.local/state/vpdf/state.json`. Missing or unreadable state starts empty.
State saves during activity and on exit; abrupt process or machine failure can
lose recent changes. Simultaneous readers can race when saving.

On macOS, search indexes persist in `~/Library/Caches/vpdf/content`, so a
reopened PDF searches without rereading its pages. Resolved outlines and figure
lists also persist there, avoiding repeated extraction when reopening books.
Figure labels include adjacent caption descriptions. The reader prepares the
list during idle time after the visible pages finish rendering; `gf` also starts
it on demand. `:cancel` stops preparation until explicitly requested. These
records and stored page OCR output hold the PDF's text, readable only by your
account; PDFs opened with a password are never stored. Small records there
remember each PDF's hash by its file identity. The system may purge that
directory at any time. Set `VPDF_CACHE` to another directory, or to `off` to
keep indexes, outlines, figure lists, and OCR output in memory only. Linux
requires an explicit cache directory. Missing, damaged, or stale records fall
back to PDF extraction; the first open still resolves the authored destinations.

The reader reloads a changed PDF and retains a useful position, view choices,
and marks. Arbitrary document edits can change where the passage lands. Reload
clears temporary OCR text and jump history. Hidden panes check for changes when
shown again. See [sessions and state][sessions] for persistence and reload
design.

## Performance display

Use `:perf` to toggle session-wide performance statistics. The setting survives
restarts. The display wraps into columns above the panes as the terminal
narrows; very short terminals retain space for reading and clip excess statistic
rows. Executable help explains each label, unit, measurement window, and scope.

Content FPS falls to zero when the view stops changing. The display measures
application output, not monitor refresh or input-to-photon latency; Out
excludes the display's own repaints. Sampling slows to once a second while
nothing draws, but it still adds work, so compare enabled and disabled runs when
measuring idle overhead. CPU readings cover Vpdf, excluding the terminal
and separate helper processes. Search and cold rendering can consume CPU while
the interaction loop remains responsive.

## External actions

Opening a document does not automatically launch its links. Explicitly followed
HTTP, HTTPS, and `mailto:` links use the platform opener. Mail links lose their
query parameters before opening; other URI schemes are rejected. Passwords use
a hidden interactive prompt rather than command-line arguments.

Text copying sends [OSC 52][osc52] and can also invoke a local clipboard helper.
SSH sessions skip the native helper because it would target the remote host's
clipboard. Terminal policy controls OSC 52 acceptance and size limits.

Figure and region image copying uses a native clipboard helper locally. Over
SSH it depends on the terminal's [image-clipboard protocol][image-clipboard] and
acknowledgment; text clipboard support alone does not imply image support. See
[terminal setup][terminal] and executable help for available actions.

[installation]: install.md
[demo]: demos.md
[text]: dev/text.md
[external]: #external-actions
[sessions]: dev/sessions.md
[helpers]: install.md#optional-helpers
[saved-state]: #saved-state-and-reload
[terminal]: install.md#terminal-setup
[kitty-mouse]: https://sw.kovidgoyal.net/kitty/conf/#mouse-actions
[mouse]: #mouse
[tmux-drag]: install.md#alt-drag-under-tmux
[osc52]: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Operating-System-Commands
[image-clipboard]: https://sw.kovidgoyal.net/kitty/clipboard/
[pixel-mouse]: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Mouse-Tracking
[pointer-shapes]: https://sw.kovidgoyal.net/kitty/pointer-shapes/
