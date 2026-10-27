# Demo backlog

The [gallery][gallery] and its [capture manifest][manifest] record completed
examples and their executable identities. Capture recipes use the current
[executable key table][keys] and [user lessons][lessons]; historical pixels keep
their original provenance. Publication and recovery use the
[media workflow][capture].

## Current coverage

| Workflow           | Example                                       |
| ------------------ | --------------------------------------------- |
| Manual and panes   | Manual document and help beside a book        |
| Reference lookup   | `K` preview and `gd` on a body figure label   |
| Continuous text    | Text crossing a page separator                |
| Notes              | Authored comment, hover author, and actions   |
| Figures and tables | Caption list beside a selected diagram        |
| Marks              | Local/global marks and return context         |
| Reading layout     | Crop, zoom, rotation, and narrow-pane text    |
| Themes             | A small set of readable built-in palettes     |
| Interactions       | Find/return, reference, panes, select/capture |

`gd` resolves supplied PDF links and recognizable document labels. It does not
look up arbitrary words or fetch missing references. Ambiguous matches can
require choosing an outline entry or result; that branch needs its own example.

## Later captures

| Workflow          | Scene                                                            |
| ----------------- | ---------------------------------------------------------------- |
| Preview expansion | Scroll past its target page; middle-click into a split           |
| Scrollbar         | Scrub with heading/match ticks and a visible section label       |
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
