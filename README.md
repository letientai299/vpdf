# vpdf

A keyboard-driven terminal PDF reader. Read, navigate, and copy text without
changing the source document.

## Start

```sh
brew install letientai299/tap/vpdf
vpdf document.pdf
```

Use `vpdf --help` for CLI options and subcommands, and `:help` inside the reader
for keys and commands. Use `:jumps` to list recorded jump destinations from
both history directions; Enter follows a row.

The reader requires [Kitty graphics][kitty]; `--text` shows extracted text.
The reader requires [Kitty keyboard][keyboard] disambiguation, or tmux's
extended keys; see [terminal setup][setup].

Normal `h j k l` and arrow keys pan or scroll the viewport; counts scale the
movement. `v` and `V` enter character or line Visual selection at a visible
click/jump target, or the first visible text. Text motions belong to Visual;
`y{motion}` and `yy` also copy from that visible entry point. `o` swaps ends,
`gv` restores the last range, and `y` copies and returns Normal.

Normal shows no glyph cursor. View scrolling preserves selection endpoints;
zoom, crop, rotation, panels, and resize keep the reading area anchored.
`K`, Enter, and `*` use a visible click/jump target. A textless view reports
selection unavailable; start OCR explicitly with `:ocr`.

Esc cancels transient input and returns Normal while preserving side-panel
focus and committed jobs. `:cancel` stops the active pane's search, OCR, and
outline work. Extracted text follows the same modes and viewport rules.

## Documentation

Start at the [docs index][docs] for installation, project principles,
architecture, and development.

## License

vpdf's original code is [MIT-licensed][license]. Executables that include
MuPDF and its Rust bindings remain subject to their AGPL obligations. See
[dependency licensing][notices].

[kitty]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
[keyboard]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
[setup]: docs/installation.md#terminal-setup
[docs]: docs/readme.md
[license]: LICENSE
[notices]: docs/installation.md#licensing
