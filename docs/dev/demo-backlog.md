# Demo backlog

The gallery's original captures used `b617f73` on 2026-10-18. Converting those
images to JPEG preserved their scenes and build identity. These later features
need new captures; this list does not claim live capture validation.

## First additions

| Feature           | Scene                                                                           |
| ----------------- | ------------------------------------------------------------------------------- |
| Reference lookup  | Use `gd` on an unlinked figure or section reference; show ambiguous candidates. |
| Continuous text   | Scroll `:text` across a page boundary and select across pages.                  |
| Expanded previews | Scroll a link preview past its target page; middle-click it into a split.       |

Reference lookup resolves document labels, not arbitrary term definitions or
unlinked citations. Use a real PDF containing the relevant reference.

## Further additions

| Feature             | Scene                                                                                                       |
| ------------------- | ----------------------------------------------------------------------------------------------------------- |
| Scrollbar           | Show heading and search-match ticks while scrubbing with the page and section label visible.                |
| Mouse layout        | Middle-click a destination into a split, drag pane and panel dividers, then double-click to restore sizing. |
| Annotation comments | Open a note, show its author and comment, and expose Copy comment in its menu.                              |
| Performance display | Show `:perf` above a reading layout; explain that it measures application output.                           |

Use short recordings for pane resizing, middle-drag panning, selection held at
an edge, and Space/Shift-Space scrolling with two rows of overlap. Include panel
and thumbnail context menus and clickable status items when showing mouse
navigation.

Search-index reuse and stored page OCR output suit a separate recording with
explicit cold and warm conditions. Screenshots cannot establish a speedup.

Keep new media in the [gallery][gallery], with provenance and attribution under
the [capture workflow][capture]. Check each workflow against executable help
before recording.

[gallery]: ../demos.md
[capture]: dev.md#gallery-capture
