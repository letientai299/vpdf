# Tabs and panes

<!-- help-topic:panes -->

A tab contains panes. Each pane has its own reading position, selection,
search, and panel focus. Moving in one pane leaves the other at its passage.

## Compare passages

1. Open a book and move to the first passage.
2. Type `:vsplit` and Enter, or press `Ctrl-w v`. The document is duplicated
   in a pane to the left, which becomes focused. `:split` or `Ctrl-w s`
   creates a pane above instead.
3. Move the new pane to the second passage. Use `Ctrl-w p` to switch back to
   the previously focused pane. Ctrl-w followed by `h`, `j`, `k`, or `l`
   focuses the neighboring pane in that direction.
4. Press `Ctrl-w c`, or run `:close`, to close the focused pane. `:only`
   closes the other panes in this tab.

To compare different files, use `:vsplit notes.pdf` or `:split notes.pdf`.
A filename already open anywhere focuses its existing pane instead of creating
a duplicate. To duplicate an already open document, first focus it and split
without a filename. `:edit other.pdf` replaces the focused pane's document;
`:tabedit other.pdf` opens a new tab. Without a path, `:edit` chooses a file
for the focused pane; `:tabedit` and `:open` choose one for a new tab.

## Tabs and help

`gt` goes to the next tab, `gT` to the previous one, and `2gt` to tab 2.
`:tabnext 2` also selects tab 2. `:tabnew` opens recent files in a new tab.
`:tabclose` closes the current tab and every pane in it.

From a book, `:help` reuses a help-only tab or creates one. `:help! search`
opens help beside the invoking pane. `Ctrl-w p` returns focus to the book.
Topic lookups from within help stay in that pane and retain local marks,
search, and jump history. Bare `:help` returns to the manual's first page;
switching tabs keeps the place you left.

Closing the final help pane returns to its invoking pane if it still exists,
otherwise an adjacent tab or the start screen. Opening help from another book
refreshes its captured settings context. Splits of help share its document
while keeping independent reading positions.

## Resize and arrange

`Ctrl-w =` equalizes pane sizes. `Ctrl-w +` and `Ctrl-w -` change height;
`Ctrl-w >` and `Ctrl-w <` change width. Counts enlarge these adjustments.
`Ctrl-w _` maximizes height and `Ctrl-w |` maximizes width. Ctrl-r followed by
`h`, `j`, `k`, or `l` repeatedly extends a pane edge; Escape ends resizing.

Ctrl-w followed by uppercase `H`, `J`, `K`, or `L` moves a pane to that outer
edge. `Ctrl-w x` exchanges sibling panes, `Ctrl-w r` rotates panes, and
`Ctrl-w T` moves a pane into a new tab. Drag a divider to resize; double-click
it to equalize. Drag the side panel edge to resize that panel separately.

## Close the intended layer

Escape cancels a temporary interaction or pending input before leaving a
reading mode. `q` closes a keyboard preview, focused panel, or text preview
before it closes the reading pane. `:close` explicitly closes the pane;
`:tabclose` closes its tab; `:qall` exits every tab and the reader.

Closing saves eligible reading state, not a snapshot of the full layout.
The next launch can restore a book's position and view; it does not rebuild
tabs, splits, panels, or background jobs. See [Files and saved state][state].

See [help beside a book][help-demo] and [a pane comparison][panes-demo].

[state]: reading.md
[help-demo]: ../demos.md#built-in-manual
[panes-demo]: ../demos.md#compare-passages
