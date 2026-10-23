#let book = json("book.json")
#let fonts = json("renderer.json").fonts
#set document(title: "Vpdf help", author: "Vpdf", date: none)
#let background = rgb(6.25%, 12.5%, 18.75%)
#let body = rgb(12.5%, 12.5%, 18.75%)
#let heading-color = rgb(18.75%, 12.5%, 18.75%)
#let key-color = rgb(25%, 12.5%, 18.75%)
#let link-color = rgb(31.25%, 12.5%, 18.75%)
#let command-color = rgb(37.5%, 12.5%, 18.75%)
#let example-color = rgb(50%, 12.5%, 18.75%)
#set page(width: 612pt, height: 792pt, margin: 48pt, fill: background,
  footer: context align(center)[#counter(page).display()])
#set text(font: fonts.first(), size: 11pt, fallback: false, fill: body,
  top-edge: 0.8em, bottom-edge: -0.2em)
#show raw: set text(size: 1em / 0.8)
#let code(value) = text(fill: if value.starts-with(":") {
  command-color
} else if value.len() > 1 and (value.starts-with("/") or value.starts-with("?")) {
  example-color
} else { key-color }, raw(value))
#show heading: set text(fill: heading-color)
#show heading.where(level: 1): set block(above: 0pt, below: 16pt)
#show heading.where(level: 2): set block(above: 22pt, below: 10pt)
#show link: it => text(fill: link-color, underline(it))
#set par(leading: 7pt, spacing: 13pt)
#set list(tight: false, spacing: 6pt)
#set enum(tight: false, spacing: 6pt)
#set heading(numbering: none)

#let topic(id, title) = {
  [#heading(level: 1, outlined: true, text(title)) #label(id)]
  context {
    let pos = query(label(id)).first().location().position()
    [#metadata((id: id, page: pos.page - 1,
      x: pos.x / 1pt, y: pos.y / 1pt)) <help-topic>]
  }
}
#let part(body) = heading(level: 2, body)
#let code-block(value) = block(width: 100%, inset: 8pt, above: 12pt, below: 12pt, {
  set text(size: 9pt, fill: example-color)
  set par(leading: 5pt)
  raw(value, block: true)
})
#let local-link(id, title) = link(label(id), text(title + " |" + id + "|"))
#let reference(id, title) = link("vpdf-help:" + id, text(title + " |" + id + "|"))

HELP_CHAPTERS

#set page(footer: none)
= Font notices
#set text(size: 8pt)
#for name in ("libertinus.txt", "dejavu.txt") {
  text(read("notices/" + name))
  parbreak()
}
