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

## Native Mac opening

In the macOS app, File → Open and Command-O open a window sheet from Home
or document content. This platform shortcut takes precedence over custom
bindings. It is disabled during prompts, overlays, and another open request.
Cancel leaves the document unchanged. Other reader bindings keep their
configured behavior.

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
