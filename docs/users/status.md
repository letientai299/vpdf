# Status and progress

<!-- help-topic:status -->

The status line identifies the active reading mode or temporary interaction.
It also shows the current section, physical page counter, optional printed
label, tab names, pending keys, and messages. An inactive pane can retain a
selection while another surface owns keyboard input. Narrow panes show only
the first letter of the mode label to leave room for the section name.

## Reading labels

NORMAL means motions scroll the viewport. CURSOR means they move the glyph
cursor. VISUAL selects characters; V-LINE selects complete extracted lines.
Press Escape to leave a selection for its starting mode, and again from Cursor
to return to Normal. YANK means a copy operator is pending. A pending `y` waits
for a motion or object; a count, register quote, mark prefix, or window prefix
waits for its continuation. Escape clears unfinished input.

REGION edits a capture rectangle; ACTIONS chooses its operation. Enter confirms
or runs the selected action; `R` returns from actions to rectangle editing.
Escape closes capture. PROMPT edits text instead of moving the document. HINT
waits for a link label; PEEK owns a keyboard preview; MENU owns the right-click
menu; PAGES owns thumbnail overview; RESIZE waits for pane-edge movements.
Escape dismisses these interactions; `q` also closes a preview or focused panel.
A panel title such as CONTENTS, RESULTS, or ANNOTATIONS identifies its list; Tab
changes content/panel focus. See [Keys and input modes][keys].

The page counter uses physical pages; a bracketed label is the printed label.
Click the counter for page overview or the section name for the outline.
A search counter with `+` is still growing. In Normal it reports matches;
in Cursor or Visual it reports the position among known matches. The document
percentage describes progress to the bottom of the view, not jobs completed.
Messages and errors stay until the next key or click. When no message takes
priority, pointing at a link, note, clipped row, or thumbnail shows its detail.

## Background work

Search can add results while pages remain unscanned. Outline, figure, and
annotation preparation can also be pending after the first view appears.
OCR supplies text only after recognition completes. A region action shows
Running while work is pending. Wait for completion or inspect the reported
error before treating a copy or export as successful.

`:cancel` stops the focused pane's search, OCR, outline, and region jobs.
It suppresses automatic OCR until reload and stops automatic document
preparation until explicitly requested. Open a panel, start a new search, or
request `:ocr` to retry the corresponding work. Native work already executing
can finish before cancellation is observed. Escape from a prompt and
`:nohlsearch` are not job cancellation commands.

## Performance display

`:perf` toggles session-wide statistics. `:perf on` and `:perf off` set an
explicit value. The toggle survives restarts in saved state. The display wraps
above panes when the window narrows; short windows can clip statistic rows.

FPS counts changing content frames per second over a rolling one-second
window, shortened at startup. It falls to zero at rest. It is not monitor
refresh rate. CPU is process CPU time as a percentage of wall time over about
one second; multiple cores can exceed 100 percent. It excludes the terminal
and separate helper processes. RSS is the latest resident process memory,
displayed as M in units of 1,048,576 bytes; it is not PDF cache size.

Out is terminal output in K/s, using 1,024-byte units over a rolling one-second
window. It excludes performance-overlay repaints. Upload in the native frontend
measures uploaded texture bytes in the same K/s units and one-second window; it
is not terminal traffic. An em dash means a resource sample is not available
yet.

Sampling slows to once per second at rest. The overlay still adds work, so
compare enabled and disabled runs before diagnosing idle overhead. Application
frames, uploads, and output flushes do not measure GPU completion, terminal
painting, physical input latency, or input-to-photon latency.

[keys]: keys.md
