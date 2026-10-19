# Documentation

Use `vpdf --help` for the CLI, and `:help` inside the reader for keys and
commands. These describe the installed executable.

## Learn and use

Tutorials and how-to guidance share pages. The reading guide assumes familiarity
with Vim motions and explains vpdf's modes and workflows.

| Read                         | For                                                           |
| ---------------------------- | ------------------------------------------------------------- |
| [Installation][installation] | Install the reader, set up the terminal, and add helpers      |
| [Reading][reading]           | Navigate, select, compare documents, and copy text or figures |
| [Demos][demo]                | Browse reading demonstrations                                 |

Maintainers can use [Development][development] for building, validation,
benchmarks, demo capture, and packaging.

## Reference

Use executable help for the complete CLI, keys, and reader commands. These
guide sections cover requirements and behavior:

- [Terminal setup][terminal] and [optional helpers][helpers].
- [Mouse][mouse], including terminal notes and known limits.
- [Saved state and reload][saved-state].
- [Performance display][performance].
- [External actions][external], including clipboard and SSH constraints.
- [Licensing][licensing], including dependency obligations.

## Explanation

Maintainer documentation lives under `dev/`.

| Read                           | For                                                         |
| ------------------------------ | ----------------------------------------------------------- |
| [Architecture][architecture]   | Project principles, ownership, and worker boundaries        |
| [Rendering][rendering]         | Tiles, scheduling, terminal transport, and memory tradeoffs |
| [Text and search][text]        | Glyph coordinates, normalized search, outlines, and OCR     |
| [Sessions and state][sessions] | Panes, focus, persistence, and live reload                  |

## Documentation boundaries

Keep the root [README][readme] focused on identifying the reader, showing its
capabilities, and getting started. Track user workflows, setup, constraints,
design rationale, and reproduction procedures here. Categorize pages by purpose
in this index; keep related instructions and facts together. Link to
implementation files for current limits, task definitions, and protocol details.

Keep all image and future video demonstrations in [Demos][demo], with media in
`assets/`. Reuse individual images in other pages when useful.

Keep changing measurements in versioned evidence. Historical demonstrations
must identify their build and capture conditions; they do not describe every
current build. Keep interface inventories in executable help.

Update the [embedded key reference][keys] when bindings change, the
[command table][commands] when reader commands change, and the [CLI][cli] when
launch options change. Avoid copying their inventories into Markdown.

[installation]: install.md
[reading]: reading.md
[demo]: demos.md
[architecture]: dev/arch.md
[rendering]: dev/render.md
[text]: dev/text.md
[sessions]: dev/sessions.md
[development]: dev/dev.md
[readme]: ../README.md
[keys]: ../crates/ui/src/help.txt
[commands]: ../crates/ui/src/commands.rs
[cli]: ../src/main.rs
[terminal]: install.md#terminal-setup
[helpers]: install.md#optional-helpers
[mouse]: reading.md#mouse
[saved-state]: reading.md#saved-state-and-reload
[external]: reading.md#external-actions
[licensing]: install.md#licensing
[performance]: reading.md#performance-display
