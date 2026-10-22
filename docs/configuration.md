# Configuration

Vpdf reads one optional TOML file at launch for both frontends. It never
creates or rewrites that file. Use `:config reload` to apply edits throughout
the running process. Reloading a PDF keeps the current settings.

## Create a configuration

Run `vpdf --config-example` to print the supported keys and built-in defaults.
The native executable supports the same output. Both commands exit without
reading configuration or opening a frontend.

Save and edit that output at `$XDG_CONFIG_HOME/vpdf/config.toml`, or
`$HOME/.config/vpdf/config.toml` when `XDG_CONFIG_HOME` is unset, empty, or
relative. Vpdf uses the same location on macOS and Linux, following the
[XDG base directory rules][xdg]. Without an absolute base directory it uses
built-ins. It does not search document directories or parent directories.

For example, to disable the initial cursor highlight and select two OCR
languages:

```toml
[reading]
cursorline = false

[ocr]
language = "eng+vie"
```

Omitted settings keep their built-in values. Use executable help for file
selection options and the generated example for the current key reference.
An explicitly selected relative path resolves from the invocation directory.

## Reading behavior

`reading.zoom` sets width-relative zoom for documents without a saved view.
Saved document zoom takes precedence. The launch page overrides the saved
position. Cursor highlight starts from `reading.cursorline`; `:cursorline`
changes it in the current pane. Reloading a PDF preserves live pane values.
A same-document split copies the source view.

`reading.inactive_brightness` sets inactive split content brightness between
`0` (black) and `1` (full brightness). Status lines stay unchanged.

`ocr.language` supplies the default for page and region recognition.
An explicit `:ocr <lang>` overrides it for that request. Language combinations
use Tesseract's `+` separator. Install the required [OCR helper and language
data][helpers]; Vpdf checks availability when recognition runs.

Document positions, marks, histories, and the performance toggle remain in
[saved reading state][state]. They are separate from this configuration.

## Keyboard bindings

Built-in mappings remain active until individually replaced or unbound:

```toml
[keybindings.normal]
"<C-n>" = { action = "tab.next" }
"<Space>r" = { command = "config reload" }
"q" = { unbind = true }

[keybindings.prompt]
"<C-a>" = { action = "prompt.start" }
```

Bindings target actions or existing commands directly. They never expand other
bindings. Contexts separate navigation from prompt typing. The common `prompt`
context supplies command, search, and password defaults; those contexts can
override it. Password prompts exclude the external editor.

Exact bindings cannot prefix longer bindings within a context. Explicitly
unbind conflicting descendants before replacing a prefix family. Shared
prefixes wait without a timeout. Invalid continuations are consumed and clear
the pending sequence. Cancellation controls can also be rebound or unbound.
Terminals can use only the keyboard events they deliver.

## UI themes

Set `appearance.theme` to a built-in theme:

```toml
[appearance]
theme = "nord"
```

Use `:theme <name>` or `:colorscheme <name>` to switch themes in the running
process. Tab completion lists built-in names. The selection applies to all
tabs and splits, including documents opened later. It does not change the
configuration file or saved state. `:config reload` restores the configured
theme.

The default theme preserves the current appearance and terminal palette
inheritance. Named themes use explicit RGB colors in both frontends. UI colors
include text, bars, panels, menus, scrollbars, and page overlays. PDF pixels and
document fonts stay unchanged. Theme changes preserve cached PDF tiles.
[Theme notices][theme-notices] retain palette attribution and licenses.
[Palette provenance][palettes] records the imported source revisions.

## Reload configuration

`:config reload` rereads the original selected source. Each running process
reloads independently; `--no-config` keeps built-ins. Changing environment
variables does not select another file.

Bindings and themes apply to every open tab and split.
Inactive brightness applies to existing splits. OCR language applies to
subsequent requests; running OCR continues. Initial zoom and cursorline apply
to newly opened documents. Reload preserves current page, live zoom, selection,
reading position, history, and prompt text. It clears incomplete key sequences,
counts, registers, operators, and prompt editing prefixes.

Omitted values restore built-ins. Invalid values keep their previous working
values while valid entries apply. Unreadable files, malformed TOML, and missing
explicit files preserve all live settings and warn. A missing discovered file
acts as empty configuration.

## Errors

A missing discovered file silently uses built-ins. Other read failures,
invalid UTF-8, or TOML syntax errors use all built-ins and show a startup
warning in the status bar, including on the start screen. A missing explicitly
selected file also warns. Configuration errors never stop startup.

With valid TOML syntax, Vpdf keeps each valid recognized setting and warns
about invalid values and unknown keys. An invalid section type discards only
that section. Invalid setting values use their built-in defaults.
Warnings name the file and affected settings; syntax errors include parser
locations. Dismiss them using the normal status-message controls.

[xdg]: https://specifications.freedesktop.org/basedir/latest/
[helpers]: install.md#optional-helpers
[state]: reading.md#saved-state-and-reload

[palettes]: licenses/theme-sources.json
[theme-notices]: licenses/themes.txt
