# First session

<!-- help-topic:first-session -->

Try this exercise with default bindings. With customized bindings, use
`:help` followed by the key to see what it does in the current mode.

## Open and follow

1. Open a book, or start `vpdf` without a filename. Type `:help` and press
   Enter. The manual opens in a reusable tab; your book keeps its place.
2. Press `f`. Labels appear beside visible links. Type the label beside
   [Practice text][practice]. You arrive at that chapter. If the link is off
   screen, scroll with `j` until it is visible, then press `f`.
3. Press `Ctrl-o` to return here. `Ctrl-i` goes forward to the practice text.
   Press `Ctrl-o` again. Escape cancels unfinished hint input.

A mouse click follows a link too. In Cursor mode, Enter or `Ctrl-]` follows
the link beneath the glyph cursor. Under tmux in Kitty, Ctrl-i can arrive as
Tab; use the JUMPS panel if forward history is unavailable.

## Move and find

1. Press `Ctrl-d` to scroll down half a viewport and `Ctrl-u` to scroll up.
2. Type `/navigation` and press Enter. The reader finds that word in this
   manual. Press `n` for another match and `N` to reverse direction.
3. Press `Ctrl-o` to return from the search jump. Type `:nohlsearch` and Enter
   to hide the highlights.

Search may continue through later pages in the background. Escape while typing
cancels the prompt; it does not mean that all document jobs have stopped.

## Copy a sentence

1. Type `:help practice` and Enter, then `/Select this sentence` and Enter.
2. Press Escape if a temporary interaction owns input. In Normal mode, press
   `i` to enter Cursor mode. Use `h`, `j`, `k`, and `l` to reach the sentence.
3. Type `yis`: `y` starts copying, `i` means inner, and `s` means sentence.
   The status message reports the copied character count. Paste into your
   notes to check clipboard delivery.
4. For a visible selection, type `vis` instead, then `y`. Escape leaves
   Cursor mode and returns to viewport scrolling.

The copied sentence should be `Select this sentence to try text selection.`
PDF extraction determines the characters and boundaries. See
[Selecting and copying][selection] if the clipboard stays empty.

## Resume the book

Press `Ctrl-o` to return through your help jumps. Type `:close` and Enter to
close this help pane. If the invoking book is still open, it regains focus at
its reading position. From that book, `:help! search` opens the search lesson
beside it; `Ctrl-w p` returns focus to the book.

Use `:help` again to start at the manual's first page. Use `gt` or `gT` to
revisit an existing help tab at its current place. Help is one document:
looking up a topic retains its search, local marks, and jump history.

[practice]: practice.md
[selection]: yank.md
