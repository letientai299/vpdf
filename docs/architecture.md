# Reader architecture

The foreground loop owns reader state and terminal output. A render worker
owns a separate MuPDF document and accepts the latest viewport tile set.
Search starts lazily in another worker with a separate document and bounded
normalized indexes. OCR uses Tesseract in a subprocess. No MuPDF document is
shared concurrently across threads.

## Rendering

Visible tiles and one extra tile row are rasterized off the input thread.
The terminal retains base images under a bounded working-set budget. Scrolling
changes crop and placement commands. Cursor, selection, and search highlights
are separate translucent images, so moving the cursor does not rasterize the
page again. Synchronized terminal updates group each frame. Scroll inputs
accumulate a target and animate toward it; cursor motions, jumps, and Escape
interrupt the animation. Both current and target viewports request tiles.
Frames are scheduled at an 8.33 ms interval when work is pending. Idle loops
poll at 8 ms and do not redraw unchanged content.

The latest viewport replaces queued render work. An in-flight native tile can
finish before cancellation is observed. Search checks its generation between
pages and rejects superseded results. A regex backtracking budget bounds
pathological patterns. An individual native PDF interpretation call is not
preemptible through the safe bindings used here.

MuPDF rasterization runs on the CPU. Kitty composites images on its GPU. The
reader does not claim GPU PDF rendering or a fixed frame rate on unmeasured
hardware. Cold text extraction and initial metadata loading still happen on
the foreground thread; documents with expensive native interpretation can
pause these operations.

## Memory and limits

Application text, OCR, search, register, and terminal-image caches have
explicit budgets. Display lists have a page-count bound; their native memory
size is not exposed by the binding. MuPDF's internal allocations and terminal
memory remain separate from application accounting. These bounds are not a
hard process RSS limit.

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
delivery uses OSC 52 and therefore follows terminal policy.

Terminal-image acknowledgement and loss recovery through every multiplexer
are not yet qualified. Resizing or changing zoom requests another viewport;
reloading invalidates all image state. Keep terminal compatibility testing
separate from native rendering and state tests.

## Compatibility

Reader keybindings follow the PDF plugin. Editor window management is outside
this process. Native MuPDF glyph boxes can differ from PyMuPDF's corrected
boxes, especially for font ascenders and descenders. Search regexes use a Rust
engine with bounded backtracking, so Python-specific syntax can differ.

OCR character positions interpolate Tesseract word boxes. They support cursor
movement, selection, and search but do not reproduce font-specific glyph
advances. No annotation authoring, form editing, PDF export, note management,
or persistent bookmarks are included; the source plugin does not supply those
features either.
