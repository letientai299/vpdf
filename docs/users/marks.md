# Marks and jump history

<!-- help-topic:marks -->

Marks name passages you intend to revisit. Jump history records detours such
as following a reference or search result. Both leave the source PDF unchanged.

## Keep two passages

1. Move to an exercise in your book and type `ma` to set local mark a.
2. Move to its explanation and type `mb` to set local mark b.
3. Type `'a` to return to the exercise's line. Type `'b` to return to the
   explanation. Backtick followed by `a` returns to the exact glyph column.
4. Run `:marks` to list marks. Select a row with `j` or `k` and Enter to follow.

A lowercase mark belongs to the current reading pane. A same-document split
starts from a copy of its view, then keeps independent live reading state.
Uppercase marks are global and include a file destination: `mA` sets one and
`'A` follows it, opening or focusing that file as needed. A new mark with the
same name replaces the old one.

File-backed local and global marks can survive a later launch through saved
state. Built-in help supports local session marks but rejects persistent global
marks. Topic lookups within help retain local marks; a manual refresh clears
page-based navigation because pagination may have changed.

## Return from a detour

`Ctrl-o` goes back through jumps; `Ctrl-i` goes forward. Run `:jumps` to choose
a recorded destination directly. Repeated jumps can cross pages and restore
the recorded view. Scrolling alone does not create a bookmark for every row.

Going back and making a new jump discards the old forward branch. Use named
marks when you need both places regardless of later detours. Jumps to a target
already clear of the view's edge rows can preserve the viewport; otherwise
the target appears near the top. Page jumps show the page top.

Reload clears jump history and temporary OCR, retaining a useful position,
view choices, and marks. Large edits to the PDF can move the passage beneath
a saved position. Help is excluded from saved document state. See
[Files and saved state][state] for file locations and save boundaries.

[state]: reading.md
