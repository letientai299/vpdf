# Documentation

Use `vpdf --help` for the CLI, and `:help` inside the reader for keys and
commands. These describe the installed executable.

| Read                           | For                                                         |
| ------------------------------ | ----------------------------------------------------------- |
| [Installation][installation]   | Distribution, terminal setup, and optional helpers          |
| [Reading][reading]             | Modes, selections, panels, and comparing documents          |
| [Screenshot gallery][demo]     | Captured examples of reading workflows                      |
| [Architecture][architecture]   | Project principles, ownership, and worker boundaries        |
| [Rendering][rendering]         | Tiles, scheduling, terminal transport, and memory tradeoffs |
| [Text and search][text]        | Glyph coordinates, normalized search, outlines, and OCR     |
| [Sessions and state][sessions] | Panes, focus, persistence, and live reload                  |
| [Development][development]     | Building, validation, benchmarks, and release decisions     |

## Documentation boundaries

Keep the root [README][readme] focused on identifying the reader, showing its
capabilities, and getting started. Track user workflows, setup, constraints,
design rationale, and reproduction procedures here. Link to implementation
files for current limits, task definitions, and protocol details.

Keep changing measurements in versioned evidence. Historical demonstrations
must identify their build and capture conditions; they do not describe every
current build. Keep interface inventories in executable help.

Update the [embedded key reference][keys] when bindings change, the
[command table][commands] when reader commands change, and the [CLI][cli] when
launch options change. Avoid copying their inventories into Markdown.

[installation]: installation.md
[reading]: reading.md
[demo]: demo.md
[architecture]: architecture.md
[rendering]: rendering.md
[text]: text.md
[sessions]: sessions.md
[development]: development.md
[readme]: ../README.md
[keys]: ../src/help.txt
[commands]: ../src/cmdline.rs
[cli]: ../src/main.rs
