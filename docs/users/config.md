# Configuring the reader

<!-- help-topic:configuration -->

Vpdf reads one optional TOML file at launch for both frontends. It never creates
or rewrites that file. Use `:config reload` to apply edits throughout the
running process. Reloading a PDF keeps the current settings.

Use `:help config` to inspect built-in setting defaults. Help describes the
built-in keys, even when your configuration changes them.

## Create a configuration

Create a TOML file using the [built-in defaults][defaults] as a reference.
Save it at `$XDG_CONFIG_HOME/vpdf/config.toml`, or
`$HOME/.config/vpdf/config.toml` when `XDG_CONFIG_HOME` is unset, empty, or
relative. Vpdf uses the same location on macOS and Linux, following the [XDG
base directory rules][xdg]. Without an absolute base directory it uses
built-ins. It does not search document directories or parent directories.

For example, to disable the initial cursor highlight and select two OCR
languages:

```toml
[reading]
cursorline = false

[ocr]
language = "eng+vie"
```

Omitted settings keep their built-in values. `vpdf -u path/to/config.toml`
(or `--config PATH`) selects one file explicitly; a relative path resolves
from the invocation directory. Missing or unreadable explicit paths disable
configuration silently. Use `vpdf -u NONE book.pdf` to select built-ins when
`NONE` is not a readable file. The native executable accepts the same option.

Keep TOML table names and quoted key sequences as shown. Each value belongs to
the most recent table heading. Start with this minimal file, run
`:config reload`, and inspect `:help config` before adding more mappings.

## Reading behavior

To open unfamiliar documents at a larger scale without a cursor highlight:

```toml
[reading]
zoom = 1.25
cursorline = false
```

`reading.zoom` sets width-relative zoom for documents without a saved view.
Saved document zoom takes precedence. The launch page overrides the saved
position. Cursor highlight starts from `reading.cursorline`; `:cursorline`
changes it in the current pane. Reloading a PDF preserves live pane values. A
same-document split copies the source view.

To make the focused pane easier to distinguish when comparing documents:

```toml
[appearance.opacity]
inactive = 0.4
```

`appearance.opacity.inactive` fades inactive split content toward the theme
background. `0` leaves content unchanged; `1` covers it. Status lines stay
unchanged. This replaces `reading.inactive_brightness`; the old key now warns.
Use `inactive = 1 - old_brightness` when migrating.

Document positions, marks, histories, and the performance toggle remain in
[saved reading state][state]. They are separate from this configuration.

## OCR languages

To recognize documents containing English and Vietnamese:

```toml
[ocr]
language = "eng+vie"
```

`ocr.language` supplies the default for page and region recognition. An explicit
`:ocr <lang>` overrides it for that request. Language combinations use
Tesseract's `+` separator. Install the required [OCR helper and language
data][helpers]; Vpdf checks availability when recognition runs. Choosing a
language here does not download its data or start OCR automatically. For a
single French page, use `:ocr fra` without changing the file. Language names
must be nonempty, including each part of a `+` combination, and contain no
control characters.

## Single keys and key sequences

Built-in mappings remain active until individually replaced or unbound. For
example, add tab navigation, make `q` close the current pane, and require a
sequence to quit:

```toml
[keybindings.normal]
"<C-n>" = { action = "tab.next" }
"<C-p>" = { action = "tab.previous" }
"q" = { command = "close" }
"<Space>q" = { action = "quit" }
"<Space>r" = { command = "config reload" }
```

`<Space>q` means press Space, release it, then press `q`. It is not a chord.
These bindings apply in Normal mode; Cursor and Visual keep their own mappings.
Binding another key to an action leaves the original key active. To move an
operation to a new key, also unbind the old sequence:

```toml
[keybindings.normal]
"gt" = { unbind = true }
"<C-n>" = { action = "tab.next" }
```

Each entry contains exactly one target: `action`, `command`, or `unbind = true`.
Actions take typed fields where required. Commands use existing Ex command
names and arguments without the leading colon. They execute directly; bindings
never expand other bindings or replay a string of keystrokes. For example,
`{ command = "theme dark" }` selects a theme, while `{ action = "tab.next" }`
selects the next tab.

The [built-in defaults][defaults] list setting names and values. Use the
built-in key and action references in `:help` to find operations. Generated
topic IDs can differ from configuration action names: a motion needs
`action = "motion"` and a separate `motion` field.

## Key notation and terminal caveats

Quote every key sequence in TOML. Literal characters are case-sensitive:
`"j"` and `"J"` differ, and `"gg"` means two consecutive lowercase keys.
Angle brackets name special keys or modified keys:

- `<C-n>` holds Control; `<A-n>` or `<M-n>` holds Alt.
- `<S-Tab>` holds Shift; `<D-n>` or `<Super-n>` holds Super, or Command on
  macOS.
- `<H-n>` and `<Meta-n>` name Hyper and Meta modifiers.
- `<CR>` or `<Enter>`, `<Esc>`, `<Tab>`, and `<Space>` name individual keys.
- `<BS>` or `<Backspace>`, `<Del>` or `<Delete>`, and `<Insert>` edit text.
- `<Left>`, `<Right>`, `<Up>`, `<Down>`, `<Home>`, `<End>`, `<PageUp>`, and
  `<PageDown>` name navigation keys. Function keys use forms such as `<F2>`.
- `<lt>` represents a literal `<`. Combine modifiers as in `<C-A-Left>`.

`<S-a>` and `A` name the same key. `<C-i>` and `<Tab>` are separate bindings
inside Vpdf, but a terminal may send the same event for both. The terminal or
window manager can intercept shortcuts before Vpdf receives them. Choose keys
that your frontend delivers, and inspect the relevant context with `:help`,
for example `:help normal:<C-n>`.

## Preserve prefix families such as g

Replace a complete sequence when possible. To move first-page navigation to a
new sequence while retaining other `g` operations:

```toml
[keybindings.normal]
"gg" = { unbind = true }
"<Space>g" = { action = "motion", motion = "first" }
```

Assigning `"g" = { action = "tab.next" }` conflicts with built-in sequences such
as `gg`, `go`, `gt`, and `g?`. Vpdf rejects the conflicting assignment and
warns; it does not remove the longer mappings for you. Unbinding `g` alone does
not remove the `g` family either. There is no wildcard unbind.

To assign an action to `g` alone, add an explicit `{ unbind = true }` entry for
every longer sequence starting with `g`, including custom sequences. The manual
shows preferred shortcuts, so it is not a complete conflict inventory. Reload
and use the rejected-binding warnings to locate remaining conflicts. Keep the
unbind entries alongside the new `g` binding. Repeat this for each context you
change; Normal, Cursor, and Visual are independent.

The reverse conflict also matters: adding `qq` requires unbinding the existing
exact `q` binding first. Families such as `z`, `Ctrl-w`, and prompt
`Ctrl-x` follow the same rule. Rebinding one alias leaves other aliases active.

A shared prefix waits without a timeout. An invalid continuation consumes that
key and clears the pending sequence; repeat the intended operation from its
start. Keep a working cancellation key when changing prefixes. Escape and its
aliases can themselves be rebound or unbound.

## Choose binding contexts

Use `[keybindings.<context>]` for the surface receiving input. A Normal binding
does not automatically apply to other modes or focused panels:

- `normal`, `cursor`, and `visual` cover document reading and selection.
- `operator` covers a pending operator such as yank; `object` chooses a text
  object after inner or around.
- `resize` covers window resize input; `register`, `mark-set`, and `mark-jump`
  cover pending register and mark choices.
- `panel`, `hints`, `region`, `menu`, `preview`, and `overview` cover those
  interactions. `home` covers the start screen.
- `prompt` supplies shared prompt edits. `command`, `search`, and `password`
  inherit them and can override individual sequences.

Not every action works in every context. Vpdf rejects unavailable actions,
such as a prompt edit in Normal mode. Temporary interactions take precedence
over the underlying reading mode. See [Keys and input modes][keys].

## Motions, selection, and window actions

For page navigation in Cursor mode and a word motion in Visual mode:

```toml
[keybindings.cursor]
"<C-j>" = { action = "motion", motion = "next-page" }
"<C-k>" = { action = "motion", motion = "previous-page" }

[keybindings.visual]
"<A-w>" = { action = "motion", motion = "word" }
```

Parameterized actions require their fields. For example, add half-view
scrolling and a split shortcut:

```toml
[keybindings.normal]
"<Space>j" = { action = "scroll", amount = 0.5 }
"<Space>k" = { action = "scroll", amount = -0.5 }
"<Space>v" = { action = "window", operation = "split", axis = "columns" }
"<Space>h" = { action = "window", operation = "focus", direction = "left" }
```

Other argument forms include `amount` for `screen`, `rows`, `align`, and `zoom`;
`name` for `mark.set`; `name` and `exact` for `mark.go`; and `dx` and `dy` for
`region.shift`. Window actions use `operation` with fields such as `axis`,
`direction`, or `amount`. Unsupported fields and wrong value types are rejected.

To choose a text object during an operator, or bind an object directly in
Visual mode:

```toml
[keybindings.operator]
"<A-i>" = { action = "object.begin", around = false }

[keybindings.object]
"<A-w>" = { action = "object.select", object = "word" }

[keybindings.visual]
"<A-p>" = { action = "object", object = "paragraph", around = false }
```

Use grammar actions to preserve pending counts, registers, and operators.
`register.choose` waits for a register name; `mark.choose` waits for a mark;
`mark.jump` also takes `exact`. `yank.begin` starts a pending yank, and
`count.digit` takes a numeric `digit`. Binding a direct `mark.set` or `mark.go`
with `name = "a"` instead targets that specific mark.

## Prompt editing

To add Home and End equivalents to all prompts, then change one only for
search:

```toml
[keybindings.prompt]
"<D-a>" = { action = "prompt.start" }
"<D-e>" = { action = "prompt.end" }

[keybindings.search]
"<D-a>" = { action = "prompt.kill", to_start = true }
```

Command and password prompts retain the shared `<D-a>` action. Prompt actions
also cover movement, word movement, deletion, kill/yank, transpose, history,
completion, undo, submission, cancellation, and the external editor. For
example, history takes `forward` and `search` booleans; completion takes
`forward`; word movement takes `forward` and `shell`.

Password prompts exclude `prompt.editor`, including when inherited from
`prompt`. Commands cannot be bound in `prompt` or `password`; use prompt actions
for editing. A binding in `command` or `search` can invoke an Ex command, so
check its context before assigning a typing key.

Merge recipes into one table per context. TOML rejects repeated table headings
and duplicate keys; do not concatenate these examples unchanged. After each
change, use `:config reload`, check the status message for warnings, and test
the key on the surface you customized.

## UI themes

Set `appearance.theme` to `system`, `light`, `dark`, or a theme file path:

```toml
[appearance]
theme = "dark"
```

`system` follows each frontend:

- The native app follows OS light/dark appearance changes. If the OS does not
  report an appearance, it uses dark.
- The terminal keeps its ANSI text colors, including tabs and badges. At launch,
  Vpdf queries the terminal background with OSC 11 and chooses a light or dark
  theme file for page fills, RGB overlays, and previews. No reply means dark.
  Under tmux, the query needs passthrough; tmux may instead answer with its own
  background. Text contrast depends on the terminal's ANSI colors. Select
  `light`, `dark`, or a theme file for contrast that Vpdf checks.

`light`, `dark`, and theme files use explicit RGB colors in both frontends.
Themes style text, bars, panels, menus, scrollbars, and page overlays.
Help pages keep fixed light colors. Use `:recolor` to change PDF page colors,
including help. Theme changes preserve cached PDF tiles.

Use `:theme light`, `:theme dark`, or `:colorscheme <name|path>` to switch in
the running process. Tab completion lists built-in names, directories, and TOML
files. The selection applies to every tab and split, including documents opened
later. It does not change the configuration file or saved state.
`:config reload` restores the configured theme.

### Native UI text size

Set `appearance.ui_font_size` in the configuration file, then run
`:config reload`. The generated configuration reference lists its default.
This changes native interface text and its hit targets independently of PDF
zoom and display pixel density. The terminal uses its terminal font settings.

On macOS, Increase Contrast strengthens interface text and focus outlines;
Reduce Motion completes navigation without easing. Selection and focus use
underlines or outlines as well as color. Changes apply while Vpdf is open.
Explicit themes retain their backgrounds and PDF pixels retain their colors.
The native canvas remains opaque with Reduce Transparency enabled.

### Theme files

Copy a [theme file][theme-files] and select its path:

```toml
[appearance]
theme = "themes/nord.toml"
```

Configuration paths resolve from the configuration file's directory. Runtime
commands such as `:theme ~/.config/vpdf/themes/nord.toml` resolve relative paths
from the process's working directory. Both expand `~`. A value containing `/`
or ending in `.toml` names a file. Former built-in names such as `nord` require
file paths now. Unknown names and unreadable or invalid theme files warn and
select `system`, including during configuration reload.

A theme file contains these required top-level tokens, each as a quoted
`#rrggbb` color. Missing tokens, extra tokens, and invalid colors reject the
file.

| Token        | Use                             |
| ------------ | ------------------------------- |
| `background` | Reader and manual background    |
| `foreground` | Main text and the active tab    |
| `surface`    | Bars, panels, and menus         |
| `cursorline` | Cursor line band                |
| `muted`      | Dim text and inactive tabs      |
| `accent`     | Headings, cursors, chosen items |
| `highlight`  | Visual selection                |
| `match`      | Search matches and hint labels  |
| `link`       | Links                           |
| `error`      | Errors                          |

Two optional top-level tokens separate indicator and selection colors:

| Token          | Use                                     | Default                               |
| -------------- | --------------------------------------- | ------------------------------------- |
| `track`        | Unfinished progress and scrollbar rails | Two parts `surface`, one part `muted` |
| `selection_bg` | Selected list row background            | `surface`                             |

The `track` default mixes RGB channels. Scrollbar rails apply
`opacity.scrollbar`; their thumbs retain `muted` at a higher opacity.
Completed progress retains `link`. `selection_bg` changes only the row fill;
text roles and selection markers remain. It is independent of PDF text
selection (`highlight`) and cursor-line bands (`cursorline`).

An optional `[mode]` table sets badge fills. Each key is optional and
defaults to the listed token:

```toml
[mode]
normal = "#81a1c1"
cursor = "#88c0d0"
visual = "#b48ead"
transient = "#ebcb8b"
```

| Key         | Badges                                          | Default     |
| ----------- | ----------------------------------------------- | ----------- |
| `normal`    | Normal and the home badge                       | `accent`    |
| `cursor`    | Cursor, Pages, panel and text-view titles       | `link`      |
| `visual`    | Visual and V-Line                               | `highlight` |
| `transient` | Yank, Resize, Hint, Menu, Peek, Region, Actions | `match`     |

Badge text uses `background` when it reaches 4.5:1 contrast on the fill, and
black or white otherwise. Chosen menu items and the selected Pages label use
`accent` with the same text rule; `[mode]` does not change them.

Tests check shipped theme files for 4.5:1 contrast on the main text styles in
the native renderer. Imported palettes lighten failing tokens and note it in
the file header.

[Theme notices][theme-notices] retain attribution and licenses for imported
palettes. [Palette provenance][palettes] records their source revisions.

Side panels mark rows with symbols such as `▣` for a figure and `§` for a
section. When the font lacks them or the terminal draws them at the wrong
width, use ASCII letters instead:

```toml
[appearance]
ascii_glyphs = true
```

## Reload configuration

`:config reload` rereads the original selected source. Each running process
reloads independently; a disabled configuration keeps built-ins. Changing
environment variables does not select another file.

Bindings, themes, and opacity apply to every open tab and split. OCR language
applies to subsequent requests; running OCR continues. Initial zoom and
cursorline apply to newly opened documents. Reload preserves current page, live
zoom, selection, reading position, history, and prompt text. It clears
incomplete key sequences, counts, registers, operators, and prompt editing
prefixes.

Omitted values restore built-ins. Invalid values keep their previous working
values while valid entries apply, except theme names and files use the
`system` fallback described above. Unreadable files, malformed TOML, and missing
explicit files preserve all live settings and warn. A missing discovered file
acts as empty configuration.

## Overlay opacity

`[appearance.opacity]` accepts numbers from `0` (transparent) to `1` (opaque).
Omitted keys retain their built-in opacity and page contrast cap. Explicit
values bypass that cap, including a value equal to the built-in default.
Choose explicit values with readable PDF text in mind.

```toml
[appearance.opacity]
selection = 0.4
veil = 0.2
inactive = 0.15
```

The [setting defaults][defaults] list each key and its built-in value.

| Key           | Applies to                          |
| ------------- | ----------------------------------- |
| `selection`   | Page selection                      |
| `search`      | Search match; landing uses ×65/90   |
| `cursor`      | Page cursor                         |
| `text_cursor` | Native text input cursor            |
| `cursorline`  | Page cursor line                    |
| `link`        | Link highlight                      |
| `veil`        | Pane veil; region veil uses ×100/70 |
| `ring`        | Ring; hover uses ×70/90             |
| `scrollbar`   | Track; thumb uses ×190/50           |
| `inactive`    | Inactive split                      |

Linked fills clamp at `1`. Hint, badge, bar, and preview fills stay opaque;
region edges and heading/search tick marks keep fixed opacity. The automatic
4.5:1 page contrast cap applies to unset cursor, selection, search, landing,
pane/region veil, cursorline, and link fills. Opacity changes preserve source
PDF pixels and cached renders.

## Built-in defaults

The [setting defaults][defaults] (`:help config`) list built-in configuration
values. Use `:theme` followed by Tab to list valid built-in themes.

Your TOML file, saved reading state, and runtime commands can change the active
values. Test those changes in the reader; the manual always describes defaults.

## Errors

A missing discovered file silently uses built-ins. Missing or unreadable
explicit paths disable configuration without warning, including invalid UTF-8.
Other discovered-file read failures and TOML syntax errors use built-ins and
show a startup warning in the status bar, including on the start screen.
Configuration errors never stop startup.

With valid TOML syntax, Vpdf keeps each valid recognized setting and warns about
invalid values and unknown keys. Theme names and files use the `system`
fallback above. Other invalid values keep their built-in defaults at startup
and previous working values on reload. Warnings name the file and affected
settings; syntax errors include parser locations.
Dismiss them using the normal status-message controls.

See [theme examples][theme-demo].

[xdg]: https://specifications.freedesktop.org/basedir/latest/
[keys]: keys.md
[helpers]: install.md#optional-helpers
[state]: reading.md#saved-state-and-reload
[palettes]: https://github.com/letientai299/vpdf/blob/main/docs/licenses/theme-sources.json
[theme-notices]: https://github.com/letientai299/vpdf/blob/main/docs/licenses/themes.txt

<!-- help:defaults=config -->

[defaults]: #built-in-defaults
[theme-files]: ../../etc/themes/
[theme-demo]: ../demos.md#themes
