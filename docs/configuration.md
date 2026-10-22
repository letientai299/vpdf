# Configuration

Vpdf reads one optional TOML file at launch for both frontends. It never
creates or rewrites that file. Changes apply on the next launch; reloading a
PDF keeps the current settings.

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
