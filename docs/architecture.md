# Reader architecture

The foreground loop owns every tab's reader state and terminal output. Only
the active tab polls its workers and draws. A render coordinator owns a
separate MuPDF document, builds display lists, and accepts the latest viewport
tile set; a process-wide pool of up to four threads draws those lists.
Search starts lazily in another worker with a separate document and bounded
normalized indexes. OCR uses Tesseract in a subprocess. No MuPDF document is
shared concurrently across threads; display lists are immutable and render
on any thread.

## Rendering

Visible tiles and one extra tile row are rasterized off the input thread,
then a screen beyond the view each way, the way of the last scroll first.
Prefetch draws one band at a time while the pool is otherwise idle, so a new
viewport waits for one band at most. Finished tiles stay encoded in memory and
upload when a frame first places them; the terminal retains base images under
a bounded working-set budget. Scrolling
changes crop and placement commands. Cursor, selection, and search highlights
are separate translucent images, so moving the cursor does not rasterize the
page again. Synchronized terminal updates group each frame. Scroll inputs
accumulate a target and animate toward it; cursor motions, jumps, and Escape
interrupt the animation. Both current and target viewports request tiles.
Frames are scheduled at an 8.33 ms interval when work is pending. Idle loops
poll at 8 ms and do not redraw unchanged content.

The coordinator groups requested tiles into bands, runs of adjacent tiles in
one tile row, and the pool draws each band in one pass before cutting and
encoding its tiles; a display list replay costs about the same for a band as
for one tile. Pages with an image of at least 512K pixels render whole, up to
8 MP, and the coordinator keeps the last such page to cut later tiles from.
MuPDF decodes only the part of an image a clip covers and its scaler varies
with the clip origin, so tiles cut from one render match a full-page render,
and the image decodes once instead of once per tile. Larger image pages decode
their full-resolution images whole before rendering bands.

The latest viewport replaces queued render work. Queued bands without a wanted
tile are dropped; tiles already being drawn are not drawn again. An in-flight
band can finish before cancellation is observed. Search checks its generation
between pages and rejects superseded results. A regex backtracking budget bounds
pathological patterns. An individual native PDF interpretation call is not
preemptible through the safe bindings used here.

MuPDF rasterization runs on the CPU. Kitty composites images on its GPU. The
reader does not claim GPU PDF rendering or a fixed frame rate on unmeasured
hardware. The foreground handle and the render coordinator share one
display-list cache per document, so each page is interpreted once for both
its tiles and its text. Text read from a list the coordinator already built
costs about 0.1 ms; a page neither has visited is still interpreted on the
foreground thread, as is initial metadata loading, so documents with expensive
native interpretation can pause these operations.

## Memory and limits

Application text, OCR, search, register, encoded-tile (48 MiB), and
terminal-image caches have explicit budgets. Display lists have a page-count
bound of eight per document; their native memory size is not exposed by the
binding. MuPDF's internal allocations and terminal memory remain separate from
application accounting. These bounds are not a hard process RSS limit.

Extraction and yank limits protect text operations. Search caps matches per
page and per query. OCR raster dimensions, output, and runtime are bounded.
Source documents are never saved by the reader.

## Terminal behavior

Kitty placements use terminal cell metrics and pixel offsets. Tmux output is
wrapped in passthrough sequences. Cursor save, placement, and restore travel
in one passthrough packet; pane and top status-row offsets are refreshed on
resize. Isolated full-window and split-pane Kitty/tmux sessions were tested.
Each reader draws image identifiers from a random per-process block and
deletes only that block, so readers in sibling panes keep each other's images.
Scoped cleanup uses Kitty range deletion, added in Kitty 0.33. The text
fallback retains reader motions, selection, search, and panels. Clipboard
delivery uses OSC 52, which follows terminal policy, and a native clipboard
tool in local sessions. SSH sessions skip the native tool, which would fill the
remote host's clipboard.

Panels, the status line, and text-mode marks use the terminal's 16-color
palette, so they follow the shell theme. Kitty overlays for the cursor,
selection, and matches are RGBA images with fixed colors.

Graphics commands suppress terminal replies. Tmux delivers replies to the
focused pane rather than the sender, where they would arrive as input to an
unrelated program, so the reader cannot track acknowledgements. Kitty silently
evicts older images beyond its 320 MB per-window quota; each reader keeps at
most 64 MiB of decoded tiles so about four readers share a window without
eviction. After another program has the screen, encoded tiles upload again
without rendering. Tabs share that budget: the graphics cache moves to the
active tab and keys tiles by document, so a recently viewed tab redraws from
resident images. Closing or reloading a document deletes only its images.
Resizing or changing zoom requests another viewport. Keep terminal compatibility
testing separate from native rendering and state tests.

## Compatibility

Reader keybindings follow the PDF plugin. Tabs and `:e` stand in for Neovim
buffers and tab pages; splits remain the terminal multiplexer's job. Native
MuPDF glyph boxes can differ from PyMuPDF's corrected boxes, especially for font
ascenders and descenders. Search regexes use a Rust engine with bounded
backtracking, so Python-specific syntax can differ.

OCR character positions interpolate Tesseract word boxes. They support cursor
movement, selection, and search but do not reproduce font-specific glyph
advances. No annotation authoring, form editing, PDF export, note management,
or persistent bookmarks are included; the source plugin does not supply those
features either.
