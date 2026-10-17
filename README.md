# vpdf

A keyboard-driven terminal PDF reader. Read, navigate, and copy text without
changing the source document.

## Start

```sh
brew install letientai299/tap/vpdf
vpdf document.pdf
```

Use `vpdf --help` for CLI options and subcommands, and `:help` inside the reader
for keys and commands.

[Kitty][kitty] displays page graphics; other terminals use extracted text.
The reader requires [Kitty keyboard][keyboard] disambiguation, which tmux
lacks; see [terminal setup][setup].

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
