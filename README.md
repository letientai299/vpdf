# vpdf

A keyboard-driven terminal PDF reader. The executable embeds MuPDF; Python is
not a runtime dependency. Kitty graphics provide the rendered canvas.

## Development

Install [mise][mise], then run:

```sh
mise trust
mise install
mise run hooks
mise run check
mise run build
```

`mise run install` installs the release executable into Cargo's binary
directory. Rustfmt uses two spaces and an 80-column target. Clippy rejects
performance, complexity, and selected maintainability violations. Prek runs
formatting, linting, and tests before commits. Rumdl formats and checks
Markdown.

## License

AGPL-3.0-or-later. MuPDF is linked into the executable and carries its own
copyright notices. See [MuPDF licensing][mupdf-license].

[mise]: https://mise.jdx.dev/
[mupdf-license]: https://mupdf.readthedocs.io/en/latest/license.html
