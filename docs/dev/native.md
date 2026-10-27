# Native integration

The [session bridge][bridge] owns PDF work on its session thread. AppKit owns
menus and sheets on the main thread through the [macOS adapter][adapter]. Winit
retains its application and window delegates; the existing Finder inbox retains
Apple-event delivery. The adapter appends File, Edit, View, Window, and Help
menus.

## Commands and state

The [command contract][commands] publishes an immutable snapshot with each
frame. Unchanged snapshots reuse their allocation. Native callbacks read the
last published snapshot and enqueue typed inputs; they never load PDFs, extract
text, or rasterize pages. Benchmark observations remain separate.

A target epoch changes when the active pane, document generation, or input
owner changes. The worker refreshes and validates it before dispatch. A dialog
request has its own ID and retains the originating target. Completion consumes
that request once; stale targets and duplicate replies do no document work.
An untaken frame passes its effects to the next frame in order.

AppKit owns menu key equivalents. After menu installation, the Egui adapter
reserves those raw chords. Semantic Copy and Cut events use the same typed
commands. Paste carries the originating target and clipboard text; the worker
checks focus before inserting it. Worker validation remains authoritative if
menu state lags. The system application menu retains Quit and Hide.

Whole-document selection keeps the reading position and highlights visible
text without extracting hidden pages. Copy extracts one page per session turn,
yields for input, and rejects changed targets. Extraction or size-limit errors
do not deliver partial text. Further reader input cancels the copy.

Command-W closes the active tab and all its splits. The last tab returns Home;
closing empty Home retains Eframe's root-window exit behavior. Terminal close
behavior is unchanged.

## Window sheets

The adapter borrows Eframe's window handle on the main thread and retains its
window. Open uses AppKit's [asynchronous sheet API][sheet]. Completion queues
paths to the worker. Cancellation queues an empty result. Shutdown cancels the
sheet; destruction disables callbacks and removes only the installed menu.

The initial integration selects one file. PDF-only filtering, multiple-file
selection, replacing the existing reader picker, and system recents remain
separate work. Source documents remain unchanged.

## Unsafe boundary

The adapter is the only new workspace crate with Objective-C unsafe code.
Shared reader crates keep their unsafe-code prohibition. The adapter denies
unsafe operations inside unsafe functions and documents each FFI invariant.
It has no PDF dependency and exposes no AppKit types to the session bridge.

Its retained handler outlives the menu target. Its borrowed handle guarantees a
live view during window lookup. Sheet completion holds a weak panel reference,
so retaining the callback cannot form a panel cycle. AppKit access remains on
the main thread, enforced by retained main-thread-only types.

## Display preferences

The [display observer][display] reads workspace preferences on the main thread.
The workspace notification wakes Egui without polling; dropping the observer
removes its retained token. Reduce Motion settles pending scrolls on the session
thread. Contrast changes affect interface text and overlay outlines, preserving
PDF pixels and explicit theme backgrounds. The canvas stays opaque.

UI text size uses configuration reload. Frames carry the requested font size;
the frontend requests new cell geometry before painting the changed text.
Clipboard and native effects still drain while geometry catches up.

## Verification

Use the [development tasks][tasks] for package tests and combined validation.
The benchmark feature adds `native menu`, `native key`, `native inspect`, and
`native cancel` to the existing event injector. These exercise AppKit action
routing and sheet lifecycle without activating the application. Append a menu
title to `native menu` or `native key` to probe another command. Inspection
reports delegate preservation, application-menu preservation, sheet parent,
and command availability, in that order.

VoiceOver and document accessibility are outside the native roadmap's current
scope. These menu and sheet probes make no accessibility claim.

[bridge]: ../../src/session/native.rs
[commands]: ../../src/session/native/commands.rs
[adapter]: ../../crates/vpdf-macos/src/lib.rs
[tasks]: ../../.config/mise.toml
[sheet]: https://developer.apple.com/documentation/appkit/nssavepanel/beginsheetmodal(for:completionhandler:)

[display]: ../../crates/vpdf-macos/src/display.rs
