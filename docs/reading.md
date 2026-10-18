# Reading a PDF

Open a PDF using the [installation guide][installation]. Use `:help` in the
reader for current bindings and commands. The [gallery][demo] shows these
workflows on a real document.

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
OCR results stay in memory; vpdf does not embed them in the source PDF.

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
The preview is a temporary reading surface with its own scrolling. Hovering an
internal link can open it without moving the reading position. Link labels can
also be picked with the pointer before following their destination. Jump
history returns to earlier destinations; marks keep places worth revisiting.
Following an external web link opens it through the desktop integration.
See [external actions][external] for clipboard and SSH boundaries.

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

Reading positions and view choices persist across launches. A saved position
does not restore a full layout of tabs, splits, and panels. The reader also
reloads a changed PDF and retains a useful position; arbitrary document edits
can change where the passage lands. See [sessions and state][sessions] for
persistence and reload constraints.

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

[installation]: installation.md
[demo]: demo.md
[text]: text.md
[external]: sessions.md#external-actions
[sessions]: sessions.md
[helpers]: installation.md#optional-helpers
