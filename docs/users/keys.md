# Keys and input modes

<!-- help-topic:keys -->

Normal mode moves the view. Cursor mode moves a glyph cursor through extracted
text. Visual mode extends a selection from an anchor to that cursor. These
modes work on rendered pages and in extracted-text preview.

## Key notation

`j` means press the lowercase key. `gg` means press `g` twice. `Ctrl-d` and
`<C-d>` mean hold Control while pressing `d`. `<CR>` means Enter; `<Esc>`,
`<Tab>`, and `<Space>` name individual keys. Uppercase matters: `v` selects
characters; `V` selects lines. `<C-w>p` means Ctrl-w, release it, then `p`.

`:help <C-d>` accepts angle-bracket notation. `c-d`, `C-d`, and `CTRL-D` are
also accepted. For prompt bindings, use `:help command:<C-u>` or
`:help search:<Tab>`. A key can do different work on different surfaces.

## Built-in bindings

`:help <C-d>` describes the built-in action and its preferred default shortcut.
Modes with the same shortcut and behavior share one entry. Terminal encoding
variants are omitted; cancellation appears as `<Esc>`. Marks use `m{mark}` to
set a mark and a quote or backtick followed by `{mark}` to jump. Counts use
`{count}`, and hint labels share a `{label}` entry. Custom mappings do not
change these entries. Use `:help config` for built-in setting defaults.
Check your TOML configuration when a key behaves differently from the manual.

## Native Mac commands

In the macOS app, menu shortcuts take precedence over custom bindings.
Vim keys and terminal bindings keep their configured behavior. Menus disable
commands that the focused view cannot perform.

| Shortcut                          | Action                                        |
| --------------------------------- | --------------------------------------------- |
| Command-O                         | Open PDFs in a window sheet                   |
| Command-Comma                     | Open the config file in the plain-text editor |
| Command-T                         | Open the Recent view in a new tab             |
| Control-Tab / Control-Shift-Tab   | Next / previous tab                           |
| Command-W                         | Close the tab and its splits; last tab quits  |
| Command-F                         | Find in the active PDF                        |
| Command-G / Shift-Command-G       | Next / previous document result               |
| Command-C                         | Copy the focused selection                    |
| Command-A                         | Select all document text or prompt text       |
| Command-X / V / Z                 | Cut / paste / undo in a prompt                |
| Command-P                         | Print through the existing print dialog       |
| Command-Shift-= / Command-Shift-- | Increase / decrease UI scale                  |
| Option-Command-T                  | Hide or show the toolbar                      |
| Command-Q                         | Quit                                          |

On the last Recent tab, Command-W closes the window. Close Pane remains a
separate Window menu action. Zoom Window resizes the window; View zoom magnifies
the PDF. UI scale persists across launches. Help opens the embedded manual.

The toolbar toggles the sidebar, shows the page, zooms, fits the page or its
width, and starts Find. Type a page number and press Return to go there; Escape
returns to the document. Hidden toolbars stay hidden on relaunch. Narrow windows
move controls into the toolbar's overflow menu.

File > Open Recent lists the system's recent documents. Its Clear Menu clears
only that list; the Recent view keeps its entries. Removing a file from the
Recent view leaves its system entry, because macOS offers no per-file removal.
The title bar shows the focused PDF, and File > Reveal in Finder selects it.

The window reopens at its last size and position, moved onto a connected
display when needed. When macOS reopens windows on relaunch (System Settings >
Desktop & Dock, Close windows when quitting an application), the app also
reopens its last tabs, splits, pane positions, and full-screen state. The last
active tab opens first; the others follow without blocking input. Files opened
from Finder or the command line keep focus. Missing or password-protected files
stay closed and are listed in a notice; passwords are never saved. A tab whose
files are all missing or unreadable stays saved for three launches, so a
disconnected drive can return; locked tabs and missing split panes are dropped.
If you read a document in the terminal after quitting, its first pane opens
where you left it there. Set
`reading.restore_session` to change this ([configuration][config]).

Document Select All preserves the reading location. Copy processes the whole
document one page at a time, up to the clipboard size limit; further input
cancels unfinished work. A limit or extraction error leaves the clipboard
unchanged. Other panels retain their existing selection operations.

Prompts support Shift-arrow selection, Command-arrow line movement, Option-arrow
word movement, replacement by typing or paste, selection deletion, and undo.
Option-produced characters and composed input enter prompt text. Passwords
remain masked, cannot be copied or cut, and never enter prompt history.

## Change modes

In Normal, `h`, `j`, `k`, and `l` scroll the viewport. Press `i` to enter
Cursor. Those keys now move left, down, up, and right through glyphs. `w`, `b`,
and `e` move by words; uppercase `W`, `B`, and `E` use whitespace-delimited
WORDs. `0`, `^`, and `$` go to line start, first nonblank, and line end.
`(` and `)` move by sentences; `{` and `}` move by extracted paragraphs.

Press `v` to select characters or `V` to select whole lines. These also start
Visual directly from Normal. Motions extend the selection. `o` exchanges the
moving end and the anchor. `y` copies; `gv` selects the last range again.
Escape leaves Visual for the mode that started it. Escape in Cursor returns
to Normal. Vpdf reads text: `i` does not insert, and there is no edit buffer.

`Ctrl-^` (also `Ctrl-6`) and `:e #` reopen the pane's alternate document.
On Recent, `q` and Escape return to that document from saved state; without an
alternate, `q` closes the pane and Escape does nothing. `ZZ` and `ZQ` close
the pane, as do `:q`, `:q!`, `:x`, and `:wq`. See [Tabs and panes][panes].

## Counts and operators

A leading count repeats an operation: `3j` moves three rows or lines, according
to mode. `3w` advances three words. `12G` and `12gg` go to physical page 12.
A lone `0` is line start rather than a count.

`y` waits for a motion or text object. `yw` copies through a word motion;
`yy` copies a line; `3yy` copies three lines. `yiw` copies an inner word and
`yip` an inner paragraph. `i` and `a` after an operator mean inner and around;
the around form includes adjacent separation where the object supports it.
`viw` selects a word for inspection. See [Selecting and copying][selection].

`m` waits for a mark name, as in `ma`. A quote selects a register, as in
`"ayy`. `Ctrl-w` waits for a window operation. Pending input appears in the
status line. Sequences wait without a timeout. An invalid continuation clears
the sequence and consumes the key; repeat the intended command from its start.

## Input focus and cancellation

A mode describes text reading. Focus decides which pane or panel receives
keys. Tab switches between content and its side panel; Ctrl-w moves between
panes. Switching focus preserves the text selection.

Prompts, link hints, keyboard previews, menus, overview, and region capture
are temporary interactions. Their keys take precedence over document keys.
Escape dismisses the active interaction or pending sequence before leaving a
reading mode. Repeat Escape as needed, watching the status label. It does not
quit the reader. `:cancel` stops background jobs in the focused pane.

The reading mode stays underneath an interaction. Hovering a preview or
holding a pointer drag does not create another reading mode. Custom bindings
can replace cancellation too; check your configuration when a key differs.

[selection]: yank.md
[config]: config.md#reading-behavior
[panes]: panes.md
