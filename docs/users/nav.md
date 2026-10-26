# Navigation

<!-- help-topic:navigation -->

Scroll in Normal mode with `j` and `k`. The [built-in scroll binding][scroll]
shows the default half-viewport key. `Ctrl-d` and `Ctrl-u` move half a
viewport; `Ctrl-f` and `Ctrl-b` move a viewport. Space moves down with overlap;
Shift-Space moves up. `Ctrl-e` and `Ctrl-y` scroll rows without moving the
glyph cursor. In Cursor mode, `zt`, `zz`, and `zb` align the cursor at the top,
middle, or bottom of the view.

## Pages and printed labels

A PDF has physical pages, counted from 1 in reader commands. Its printed page
labels can differ. A status counter `23/244 [1]` means physical page 23 of 244,
whose printed label is 1. Front matter often uses Roman labels.

`:23` goes to physical page 23, even when its printed label is 1. `23G` and
`23gg` do the same. `gg` goes to the first physical page and `G` to the last.
`Ctrl-n` and `Ctrl-p` move to the next and previous physical page; a count
repeats the move. Backspace also moves to the previous page.

Type a nonnumeric page label after the colon to find that label: `:iv` opens
front matter labeled iv. Exact command names and aliases take precedence over
labels. Numeric input always names a physical page, so `:1` does not mean the
printed page labeled 1. Use `vpdf --page 23 book.pdf` to start at physical page
23 instead of the saved position.

## Chapters and the outline

Press `go`, or run `:toc`, to open CONTENTS. `j` and `k` select rows. Enter
follows the row and returns focus to content. `o` follows while keeping focus
in the panel, useful for comparing sections. Tab changes content/panel focus;
`q` closes the focused panel and `gq` closes a side panel from content.

Use `h` to close a branch or select its parent; use `l` to open a branch
or select its first child. Counts repeat these steps. `zR` expands all
branches; `zM` collapses all. Closing a branch preserves nested folds.
`za` toggles auto-expand: open the current section and its ancestors, and
close other branches. Manual fold commands turn auto-expand off. The row
and header menus also offer these commands.

While the panel has focus, `/word` searches its rows rather than the whole PDF.
Enter accepts the query. `n` and `N` navigate panel matches. Search includes
folded entries and page labels. Preview reveals a match; Escape restores folds,
selection, and scroll. Accepting a match retains its path and turns auto-expand
off. Use `gg` and `G` to reach the first and last rows. A selected row is your
panel choice. The current-section indicator follows the viewport; moving the row
alone does not move the book.

`]]` jumps to the next section and `[[` to the previous one. A count repeats
section jumps. The reader uses the PDF's authored outline first. Without one,
it tries to infer headings from text size and numbering. Inference can omit
headings or choose the wrong structure. Wait for preparation to finish before
assuming the outline is empty; a scan without recognized text may have none.
Use page jumps or search when no headings are available.

## Page overview and pointer navigation

Press `gp`, or run `:overview`, to browse thumbnails. Select a page and Enter
to open it; Escape returns to reading. Clicking the status page counter opens
overview, and clicking the section name opens the outline at that section.

On rendered pages the scrollbar appears during movement or when you point at
the right edge. Click its track to jump; drag its thumb to scrub. The label
shows the physical page and section. A middle click on a thumbnail or panel
row opens its destination beside the current pane, preserving focus.

Page jumps show the top of the page. A jump to a passage already comfortably
visible can keep the current view. Press `Ctrl-o` after a detour to return;
see [Marks and jump history][marks].

[marks]: marks.md

<!-- help:scroll=half_down -->

[scroll]: keys.md#built-in-bindings
