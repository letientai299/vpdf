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

## Toolbar

The adapter attaches an `NSToolbar` to Eframe's window without a full-size
content view, so AppKit insets the Eframe view: layout, hit testing, captures,
and IME positions need no toolbar offsets. Buttons and the Fit pull-down send
menu entry indexes, so they share menu commands, targets, and enablement.
Snapshots add the page count and open panel; the toolbar shows the page and
retitles Show Sidebar. Customization is off. A user default keeps visibility
without saving item lists that would hide later items.

The page field sends `Input::Page`, which validates its command like Paste.
While the field edits, Edit menu entries send their standard actions to the
field editor. Any other menu or toolbar command first returns focus to the
document view.

## Window sheets

The adapter borrows Eframe's window handle on the main thread and retains its
window. Open uses AppKit's [asynchronous sheet API][sheet]. Completion queues
paths to the worker. Cancellation queues an empty result. Shutdown cancels the
sheet; destruction disables callbacks and removes only the installed menu.

The panel selects one or more PDFs. The worker queues them and opens one per
session turn, so the first document draws before the rest load. Each file uses
the same reference handling as Finder and drop opens. The reader's terminal
picker remains. Source documents remain unchanged.

## Documents and recents

`remember_open` emits a recent-document effect for successful opens, including
focusing an open file or splitting beside it; help, failed opens, and the
terminal frontend emit none. The adapter passes it to
`NSDocumentController`, which also feeds the Dock menu. Open Recent uses the
adapter's own selectors and menu delegate, so AppKit never routes a recent
item through `NSDocument`. Its items enqueue ordinary open inputs. The submenu
is enabled only while the worker offers Open, so it cannot replace the target
of a pending sheet.

Clear Menu clears only the system list. The Recent view and its tombstones stay
separate, and macOS has no per-item removal API.

Settings… in the application menu and `:config edit` share one worker path. The
worker creates an empty config file when missing and returns its path as an
effect; the adapter
opens it in the default plain-text editor, or TextEdit before macOS 12, and
logs launch failures. Without a config source the command is unavailable.
`native app menu` lists the application menu; probes do not open the editor,
which would take focus.

Each changed snapshot sets the window's represented URL from its absolute
document path, and clears it for Home and help. Reveal in Finder is a worker
command that returns the focused path as an effect.

A minimized or hidden window runs no Eframe frames. The Finder inbox therefore
wakes the adapter, which drains it on the next main run-loop turn through a
delayed selector. Opens go straight to the worker through a cloned input
sender; the window is deminiaturized only for an active app, so background
`open -g` requests stay minimized and still open. A quit Apple event calls
`NSApplication terminate:`. Winit turns `applicationWillTerminate:` into
`LoopExiting`, and Eframe calls `on_exit`, which closes the bridge and saves
state without a frame.

## Window and session restoration

AppKit's frame autosave keeps the window size and position under the `Vpdf`
name. At launch the adapter reapplies it, then centers and shrinks the window
onto the main display when no display shows its title bar.

[`restore.rs`][restore] keeps tabs, split shapes, pane views, and full-screen
state in `native-session.json` beside the reading state. Reading state still
owns marks, outlines, and sidebars; the session file holds paths and pane views.
Each pane records the reading-state visit it belongs to. When another reader has
opened the document since, reading state places its first pane; later panes keep
their saved views. A tab with no reopened file stays saved, counting launches
that fail; the third drops it. Locked tabs drop at once, and a split drops only
its missing panes. Writes are atomic, coalesce two seconds after input, and run
again at shutdown. Unknown versions and corrupt files read as no session.

An exclusive lock on `native-session.lock` makes one instance the session
owner. Later concurrent instances neither restore nor save it, and the
terminal reader never touches it.

`reading.restore_session` defaults to the macOS `NSQuitAlwaysKeepsWindows`
preference; an unset preference reopens. The GUI drains Finder launch events
before creating the bridge, so they count as explicit launch files. The worker
waits for the first window size, opens the saved active tab, then one saved tab
per loop turn between inputs. Restored tabs keep their saved order around
explicit opens without taking focus. Missing, locked, or unreadable files are
skipped and reported once. A shutdown before restoration finishes saves the
unopened tabs in place.

Closing the window still quits: Eframe exits with its root viewport, so the
app cannot stay open without a window and recreate it from the Dock. Restoring
on relaunch covers that case. Bridge shutdown joins the worker after it leaves
its current loop turn; background search, render, and OCR threads are detached
and stop with the process.

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
[restore]: ../../src/session/native/restore.rs
