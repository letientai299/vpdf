# Reader architecture

vpdf separates reading state, PDF interpretation, and terminal presentation.
The foreground owns interaction and terminal output; workers prepare results
that it can accept, replace, or discard.

## Project principles

- Keep reading keyboard-driven, with composable motions and explicit focus.
- Preserve source PDFs. Store reading state outside the document.
- Keep input responsive by moving rasterization and document scans off the
  foreground thread where possible.
- Reuse page interpretation and pixels before doing more work.
- Bound application caches and background work without promising a hard process
  memory limit.
- Keep extracted-text presentation and make OCR and desktop integrations
  optional.

Use `vpdf --help` and `:help` for the current interface. These principles guide
changes; they do not enumerate the feature set.

## Runtime ownership

A [session][session] owns tabs, prompt history, saved state, and terminal
output. Each tab contains a split layout and panes; each [pane][app] owns an
independent reader, document handle, view, and worker handles. Two panes showing
the same file still have independent foreground documents and display-list
caches.

```mermaid
flowchart TD
    Input[Terminal input] --> Session[Foreground session]
    Session --> Tabs[Tabs and split layouts]
    Tabs --> Pane[Pane: reader, document, view]
    Pane --> Coordinator[Render coordinator: own document]
    Pane --> Search[Search worker: own document]
    Pane --> Outline[Outline job: own document]
    Pane --> OCR[OCR job: own document and Tesseract child]
    Coordinator --> Pool[Process-wide raster pool]
    Pane --> Graphics[Shared graphics cache]
    Pool --> Results[Prepared tile results]
    Results --> Session
    Session --> Output[Terminal output]
```

Every pane of the active tab receives worker updates and draws. Keyboard input
belongs to the focused pane; pointer routing can target another visible pane.
The focused pane holds the shared graphics cache and lends it to other panes
while they draw. Tabs and panes share this image budget.

See [sessions and state][sessions] for focus, hidden panes, and persistence.

## Native document boundary

Each [MuPDF handle][document] stays on its owning thread. Workers open separate
handles rather than concurrently using the foreground document. The foreground
and its render coordinator share immutable display lists through a synchronized
cache. The [raster pool][worker] replays these lists with thread-local native
contexts, without taking ownership of the source document.

Cached display lists let text extraction and rasterization reuse page
interpretation. Eviction and concurrent cache misses can require interpretation
again; this is a reuse strategy, not an exactly-once guarantee.

Opening a document still reads page metadata on the foreground thread. Text
needed by a motion may also build a display list there on a cache miss. A slow
native interpretation call can therefore pause interaction. The safe bindings
do not provide preemption of an individual native call.

## Work and resource lifetimes

The latest viewport replaces pending render requests. Search generations keep
superseded queries from changing the current results. Dropping job handles
signals cancellation; an in-flight native call may finish first.

Hidden panes release rebuildable text, display lists, search workers, and render
coordinators. OCR text and view state remain. The shared graphics cache can keep
their tiles, subject to eviction. Search drops its native handle after idling;
a render coordinator periodically reopens its handle after broad page traversal
to release native resources. The policy and thresholds live in the
[worker implementations][worker] and [search worker][search-worker].

## Design details

- [Rendering][rendering] explains scheduling, transport, and cache accounting.
- [Text and search][text] explains semantic positions and background scans.
- [Development][development] distinguishes CPU measurements from terminal paint
  and display latency.

[session]: ../../src/session.rs
[app]: ../../src/app.rs
[sessions]: sessions.md
[document]: ../../src/document.rs
[worker]: ../../src/render_worker.rs
[search-worker]: ../../src/search_worker.rs
[rendering]: render.md
[text]: text.md
[development]: dev.md
