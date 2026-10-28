# Documentation

Use `vpdf --help` for the CLI, and `:help` inside the reader for keys and
commands. These describe the installed executable.

The [user manual][manual] is the starting point on GitHub and the first page of
bare `:help`. Read its chapters in order, from the first reading exercise to
setup and command reference. Every chapter under `users/` is included in the
built-in manual, followed by generated action, configuration, and command
entries. The [chapter order][contents] defines the sequence.

See [Demos][demo] for demonstrations on real documents.

Maintainers can use [Development][development] for building, validation,
benchmarks, demo capture, and packaging.

## Reference

Use executable help for the complete CLI, keys, and reader commands. These guide
sections cover requirements and behavior:

- [Terminal setup][terminal] and [optional helpers][helpers].
- [Mouse][mouse], including terminal notes and known limits.
- [Configuration][configuration].
- [Saved state and reload][saved-state].
- [Performance display][performance].
- [External actions][external], including clipboard and SSH constraints.
- [Licensing][licensing], including dependency obligations.

## Explanation

Maintainer documentation lives under `dev/`.

| Read                           | For                                                         |
| ------------------------------ | ----------------------------------------------------------- |
| [Architecture][architecture]   | Project principles, ownership, and worker boundaries        |
| [Packages][packages]           | Package contracts, dependency gates, and focused checks     |
| [Rendering][rendering]         | Tiles, scheduling, terminal transport, and memory tradeoffs |
| [Text and search][text]        | Glyph coordinates, normalized search, outlines, and OCR     |
| [Native integration][native]   | Main-thread menus, sheets, and worker commands              |
| [Sessions and state][sessions] | Panes, focus, persistence, and live reload                  |

## Documentation boundaries

Keep the root [README][readme] focused on identifying the reader, showing its
capabilities, and getting started. Track user workflows, setup, constraints,
design rationale, and reproduction procedures here. Categorize pages by purpose
in this index; keep related instructions and facts together. Link to
implementation files for current limits, task definitions, and protocol details.

Keep demonstrations in [Demos][demo], with media hosted on the dedicated closed
GitHub issue. Capture identities, attribution, and publication records stay in
`assets/`; app icons and source artwork stay there too. See
[Demo media publication][demo-media] for review, regeneration, and recovery.
Reuse individual published images in other pages when useful.

Keep changing measurements in versioned evidence. Historical demonstrations must
identify their build and capture conditions; they do not describe every current
build. Keep interface inventories in executable help.

Keep bindings in the [key table][keys], reader commands in the [command
table][commands], and launch options in the [CLI][cli]. The [runtime
reference][key-help] uses resolved bindings and configuration. Keep [manual
sources][manual] under `users/`. Avoid copying inventories into Markdown.

[demo]: demos.md
[architecture]: dev/arch.md
[packages]: dev/packages.md
[rendering]: dev/render.md
[text]: dev/text.md
[sessions]: dev/sessions.md
[development]: dev/dev.md
[readme]: ../README.md
[keys]: ../crates/ui/src/keys.rs
[key-help]: ../src/manual/reference.rs
[manual]: users/readme.md
[commands]: ../crates/ui/src/commands.rs
[cli]: ../src/main.rs
[terminal]: users/install.md#terminal-setup
[helpers]: users/install.md#optional-helpers
[mouse]: users/install.md#mouse-and-focus
[saved-state]: users/reading.md#saved-state-and-reload
[external]: users/reading.md#external-actions
[licensing]: users/install.md#licensing
[performance]: users/status.md#performance-display
[configuration]: users/config.md
[contents]: ../tools/help/help/contents.json
[native]: dev/native.md
[demo-media]: dev/demo-media.md
