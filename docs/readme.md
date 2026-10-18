# Documentation

Use `vpdf --help` for the CLI, and `:help` inside the reader for keys and
commands. These describe the installed executable.

| Read                           | For                                                         |
| ------------------------------ | ----------------------------------------------------------- |
| [Installation][installation]   | Distribution, terminal setup, and optional helpers          |
| [Architecture][architecture]   | Project principles, ownership, and worker boundaries        |
| [Rendering][rendering]         | Tiles, scheduling, terminal transport, and memory tradeoffs |
| [Text and search][text]        | Glyph coordinates, normalized search, outlines, and OCR     |
| [Sessions and state][sessions] | Panes, focus, persistence, and live reload                  |
| [Development][development]     | Building, validation, benchmarks, and release decisions     |

## Documentation boundaries

Keep the root [README][readme] focused on starting the reader. Keep design
rationale and contributor guidance here. Link to implementation files for
current limits, task definitions, and protocol details.

Update the [embedded key reference][keys] when bindings change, the
[command table][commands] when reader commands change, and the [CLI][cli] when
launch options change. Avoid copying their inventories into Markdown.

[installation]: installation.md
[architecture]: architecture.md
[rendering]: rendering.md
[text]: text.md
[sessions]: sessions.md
[development]: development.md
[readme]: ../README.md
[keys]: ../src/help.txt
[commands]: ../src/cmdline.rs
[cli]: ../src/main.rs
