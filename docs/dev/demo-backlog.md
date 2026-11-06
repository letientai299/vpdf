# Demo backlog

The [gallery][gallery] and its [capture manifest][manifest] record completed
examples and their executable identities. Capture recipes use the current
[executable key table][keys] and [user lessons][lessons]; historical pixels keep
their original provenance. Publication and recovery use the
[media workflow][capture].

## Current coverage

| Gallery section                | Examples                                                |
| ------------------------------ | ------------------------------------------------------- |
| Three documents in split panes | Outline, preview, search, and OCR in three panes        |
| Open documents                 | Recent previews, page grid, tabs, start screen          |
| Help and discovery             | Manual, help beside a book, next-key clues              |
| Read comfortably               | Crop, zoom, rotation, text, themes, recolor             |
| Find text                      | Results, find/return video, Vietnamese, Chinese         |
| Links and references           | Hints, underlines, `K`, `gd`, hint and reference videos |
| Move through a document        | Marks, jumps, contents tree and fold video, scrollbar   |
| Select and copy                | Context menu and select/capture video                   |
| Figures, regions, and notes    | Caption list, capture, Alt sweep, comments              |
| Panes                          | Pane comparison video                                   |
| Scans and OCR                  | Recognized EDVAC text beside the scan                   |
| macOS app                      | Toolbar with Find, contents sidebar                     |

`gd` resolves supplied PDF links and recognizable document labels. It does not
look up arbitrary words or fetch missing references. Ambiguous matches can
require choosing an outline entry or result; that branch needs its own example.

## Later captures

| Workflow          | Scene                                                            |
| ----------------- | ---------------------------------------------------------------- |
| Preview expansion | Scroll past its target page; middle-click into a split           |
| Mouse layout      | Drag dividers, restore sizing, and middle-click a destination    |
| Pointer selection | Hold selection at an edge and show scrolling                     |
| Page movement     | Space/Shift-Space with retained reading overlap                  |
| Menus and status  | Panel/thumbnail context menus and clickable status actions       |
| OCR to selection  | Recognition mode, empty/cache-hit state, and selection in a clip |
| Continuous text   | Page-boundary movement and selection in a clip                   |
| Performance panel | Explain application-output metrics separately from display time  |

Search-index reuse and stored OCR need explicit cold and warm conditions.
Show waits at normal speed or label every omitted interval. Screenshots and
recording frame rates cannot establish a speedup. Future performance comparisons
need separate matched, repeated measurements.

[gallery]: ../demos.md
[manifest]: ../assets/capture.json
[keys]: ../../crates/ui/src/keys.rs
[lessons]: ../users/readme.md
[capture]: demo-media.md
