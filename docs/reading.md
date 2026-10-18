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
it, or use repeated clicks to select a word, line, or paragraph. The context
menu offers actions for the location or selection under the pointer; hover
chooses an action and a click runs it.

A middle drag pans a zoomed page. While the view moves, or while the pointer
rests on the page's right edge, a scrollbar shows ticks for top-level headings
and pages with search matches. A click on the track jumps there and records
the jump; dragging the thumb scrubs, with the page under the pointer labeled.

Glyph positions and the viewport are separate. Scrolling can move a selected
passage out of view without discarding it. Zooming and resizing keep reading
positions tied to document content. Leaving Visual returns to the mode that
started the selection. Escape dismisses a pending interaction before leaving
a reading mode.

Text selection follows the PDF's extracted glyphs and reading order. Complex
columns or missing text can make that order differ from the visual page.
The [text guide][text] explains copying fidelity and optional OCR for scans.
OCR results stay in memory, can be evicted, and disappear when the document
reloads or closes. vpdf does not embed them in the source PDF.

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
status line keeps its destination while the pointer is over it. The status line
names what the pointer rests on: a link's destination, the reference a citation
cites, a note, a clipped row, or a thumbnail's page and the section at its top.
Once the pointer moves, that detail covers an older message and takes room from
the tab names; errors stay until the next key or click. Link labels can also be
picked with the pointer before following their destination. Jump history returns
to earlier destinations; marks keep places worth revisiting. Following an
external web link opens it through the desktop integration. See [external
actions][external] for clipboard and SSH boundaries.

Hovering a note or commented highlight shows its author and comment in the
status line. A click on a note icon opens the whole comment beside it; the
right-click menu on any note offers Show note and Copy comment.

## Compare views

Use tabs to keep multiple documents open. Split a pane to compare passages in
one PDF, or place another document alongside it. Each pane keeps its own
position, selection, search, and panel focus, so navigating one view leaves
the other in place. Pointing and scrolling over a surface can move that
surface without changing keyboard focus.

A middle click on an internal link, a panel row, or a thumbnail shows that
place in a split beside the pane, which keeps focus; on a web link it copies
the address. Drag the separator between panes, the status line between stacked
panes, or the side panel's edge to resize them; a double click evens the split
or restores the panel's width. Tab names in the status line switch tabs on a
click and close on a middle click.

For saved positions and live reload, see [saved state and reload][saved-state].

## Take text or a figure

Use rendered pages when layout and figures matter. Extracted-text presentation
uses the same reading modes for navigating and selecting text, with the
fidelity limits described in the [text guide][text].

Select a detected figure or sweep a region to capture part of a page. Its
action bar offers image copying, text copying, PNG export, and optional OCR.
Point at an action to choose it, then click to run it. During a region sweep,
the status line explains how to finish or cancel the capture.
Region OCR copies recognized text; page OCR supplies temporary selectable
text for reading. Both leave the source PDF unchanged. Clipboard support and
OCR dependencies follow the [installation guide][helpers].

## Saved state and reload

Reading positions, view choices, marks, and prompt history persist across
launches. A saved position does not restore a full layout of tabs, splits,
panels, and running jobs. An explicit launch page overrides the restored
position.

The state file lives at `$XDG_STATE_HOME/vpdf/state.json`, falling back to
`~/.local/state/vpdf/state.json`. Missing or unreadable state starts empty.
State saves during activity and on exit; abrupt process or machine failure can
lose recent changes. Simultaneous readers can race when saving.

The reader reloads a changed PDF and retains a useful position, view choices,
and marks. Arbitrary document edits can change where the passage lands. Reload
clears temporary OCR text and jump history. Hidden panes check for changes when
shown again. See [sessions and state][sessions] for persistence and reload
design.

## External actions

Opening a document does not automatically launch its links. Explicitly followed
HTTP, HTTPS, and `mailto:` links use the platform opener. Mail links lose their
query parameters before opening; other URI schemes are rejected. Passwords use
a hidden interactive prompt rather than command-line arguments.

Text copying sends OSC 52 and can also invoke a local clipboard helper. SSH
sessions skip the native helper because it would target the remote host's
clipboard. Terminal policy controls OSC 52 acceptance and size limits.

Figure and region image copying uses a native clipboard helper locally. Over
SSH it depends on the terminal's image-clipboard protocol and acknowledgment;
text clipboard support alone does not imply image support. See
[terminal setup][terminal] and executable help for available actions.

[installation]: install.md
[demo]: demos.md
[text]: dev/text.md
[external]: #external-actions
[sessions]: dev/sessions.md
[helpers]: install.md#optional-helpers
[saved-state]: #saved-state-and-reload
[terminal]: install.md#terminal-setup
