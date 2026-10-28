#let fonts = json("renderer.json").fonts
#let type = json("typography.json")
#set document(title: "Vpdf help", author: "Vpdf", date: none)
#let background = rgb(6.25%, 12.5%, 18.75%)
#let body = rgb(12.5%, 12.5%, 18.75%)
#let heading-color = rgb(18.75%, 12.5%, 18.75%)
#let key-color = rgb(25%, 12.5%, 18.75%)
#let link-color = rgb(31.25%, 12.5%, 18.75%)
#let command-color = rgb(37.5%, 12.5%, 18.75%)
#let example-color = rgb(50%, 12.5%, 18.75%)
// Body text sits four cells right of headings.
#let indent = 4 * type.body * 0.5pt
#set page(width: 612pt, height: 792pt, fill: background,
  margin: (left: 48pt + indent, rest: 48pt),
  footer: context pad(left: -indent,
    align(center)[#counter(page).display()]))
#set text(font: fonts.first(), size: type.body * 1pt, fallback: false, fill: body,
  top-edge: 0.8em, bottom-edge: -0.2em, ligatures: false)
#set raw(theme: "help.tmTheme")
#show raw: set text(font: fonts.first(), size: 1em / 0.8)
#let code(value) = text(fill: if value.starts-with(":") {
  command-color
} else if value.len() > 1 and (value.starts-with("/") or value.starts-with("?")) {
  example-color
} else { key-color }, raw(value))
#show heading: set text(fill: heading-color)
// Headings stick to the text below instead of ending a page.
#let heading-spacing = ((22pt, 12pt), (16pt, 8pt), (12pt, 6pt))
#show heading: set block(above: 0pt, below: 0pt)
#show heading: it => {
  let (above, below) = heading-spacing.at(calc.min(it.level, 3) - 1)
  block(sticky: true, above: above, below: below, pad(left: -indent, it))
}
// A hyphen would split one link into two hint targets.
#show link: it => text(fill: link-color, hyphenate: false, underline(it))
#set par(justify: true, leading: type.leading * 1pt, spacing: (type.leading + type.paragraph) * 1pt)
#set list(tight: false, spacing: 6pt)
#set enum(tight: false, spacing: 6pt)
#set heading(numbering: none)
#set table(inset: (x: 6pt, y: 5pt),
  stroke: (x, y) => (bottom: 0.35pt + luma(78%)))
#set table.hline(stroke: 0.6pt + luma(65%))
#show table: set text(size: type.table * 1pt)
#show table: set par(justify: false, leading: 3.5pt)
#show table.cell.where(y: 0): strong
#show figure.where(kind: table): set block(breakable: true)
#show raw.where(block: true): it => block(
  width: 100%, inset: 8pt, fill: luma(96%),
  above: 8pt, below: 8pt, {
    set text(size: type.code * 1pt, fill: example-color)
    set par(justify: false, leading: 3pt)
    it
  })

#let topic(id, title) = {
  [#heading(level: 1, outlined: true, text(title)) #label(id)]
  context {
    let pos = query(label(id)).first().location().position()
    [#metadata((id: id, page: pos.page - 1,
      x: pos.x / 1pt, y: pos.y / 1pt)) <help-topic>]
  }
}
HELP_CHAPTERS

#pagebreak()
#set page(footer: none, margin: 48pt)
#include "legal.typ"
