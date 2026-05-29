// Geometry layer for the printable menu.
//
// This is the Typst MAIN source (set by the Rust handler). It owns the PAGE
// geometry + fold layout, and imports the active *styling* theme
// (`/theme.typ` — the embedded menu.typ or an admin pdf_themes override) for
// the palette + render functions. Keeping geometry here, out of the theme,
// means `?format=` works with ANY theme and page-layout changes never need to
// be re-ported into a forked theme.
//
// `format` (from sys.inputs, set by the Rust handler):
//   "trifold"     — original 443×210 landscape, folded in thirds, 2 pages
//                   (cover + 4-col menu / QR + 4-col menu). DEFAULT.
//   "a5-zickzack" — DIN A5 Hochformat Zickzackfalz: 3 A5-portrait panels per
//                   side (148×210mm each → 444×210 finished), printed 2-sided,
//                   +3mm bleed + fold/crop marks, 4/4 for the print shop.
//                   Folds so the COVER is the outer front, the QR the outer
//                   back.

#import "/theme.typ": *

#let format = sys.inputs.at("format", default: "trifold")
#let is-a5 = format == "a5-zickzack"

#let a5-panel-w = 148mm                // A5 portrait panel width
#let a5-panels = 3                     // panels per side
#let a5-bleed = 3mm
#let a5-trim-w = a5-panel-w * a5-panels // 444mm finished width
#let a5-trim-h = 210mm                  // finished height

// Typst 0.14 `page` has no native `bleed`/TrimBox. So we do bleed manually:
// the a5 page is the finished size PLUS 3mm on every edge (450×216), the
// background fill covers the whole page (extends into the bleed = no white
// sliver after trimming), content sits inset by the bleed, and crop + fold
// marks are drawn in the bleed margin. The printer trims 3mm off each edge to
// the finished 444×210. Trifold is unchanged (443×210, no bleed).
#set page(
  width: if is-a5 { a5-trim-w + 2 * a5-bleed } else { 443mm },
  height: if is-a5 { a5-trim-h + 2 * a5-bleed } else { 210mm },
  margin: if is-a5 { a5-bleed } else { 0mm }, // content inset = bleed on a5
  fill: theme-bg,
)

// All content runs under the theme's text/par/block styling.
#show: theme-styles

// ===========================================================================
// Geometry-owned menu cards
// ===========================================================================
// The DEFAULT menu now shows each item's ingredients (description); the
// `?condensed=1` variant drops them for the airy title+price-only in-house
// menu. The theme's own `item-row` hard-omits descriptions, and it lives in
// the admin's forked `pdf_themes` row (which drifts and once broke prod). So
// we render rows HERE, in the geometry layer, using only the theme's exposed
// palette + helpers (`col-*`, `fmt-eur`, `csv-codes`, `data`, `extras-*`).
// That keeps the ingredients feature working against ANY theme without ever
// re-porting into a fork.
//
//   show-ingredients : append the muted ingredients line under each name.
//   sz               : the row size profile (tiny squeeze vs airy original).
#let show-ingredients = sys.inputs.at("show-ingredients", default: true)

// Row size profiles. Picked once per render from `show-ingredients`:
//   ingr      — WITH ingredients (the default hand-out). 160 items + an
//               ingredients line each must fit 8 columns (2 pages / one fold
//               sheet), so the rows go quite small. Legible up close — the
//               deliberate trade to keep the hand-out a single foldable sheet.
//   condensed — `?condensed=1`, no ingredients: the airy title+price-only
//               in-house menu (the original look).
// `row` and `lead` are ABSOLUTE pt (not em): em is measured against the
// theme's 11.5pt body, which made the gaps balloon and differ between the
// first item (grouped with the title under the theme's block-spacing) and the
// rest. Absolute pt pins every gap to the row size for uniform spacing.
// 7.25pt name / 5.75pt ingredients — the ceiling that still holds the full
// 160-item menu in 2 pages (one fold sheet) AFTER tightening the title +
// size-label header leading. 7.5pt spills to a 3rd page.
#let sz-ingr = (
  name: 7.25pt, price: 7.25pt, code: 5.25pt, ing: 5.75pt,
  row: 2.6pt, lead: 4.2pt, title: 10pt, inset: 2.5mm, label: 6pt, titlegap: 1.4pt,
)
#let sz-condensed = (
  name: 7pt, price: 8pt, code: 6pt, ing: 5.5pt,
  row: 5pt, lead: 6pt, title: 11pt, inset: 4mm, label: 7pt, titlegap: 3pt,
)
// WITH ingredients → the tiny squeeze that holds the full menu in 2 pages;
// condensed → the airy original sizing. Same profile drives trifold + A5.
#let sz = if show-ingredients { sz-ingr } else { sz-condensed }

// One menu item row. Mirrors the theme's visual language (number + name +
// spicy mark + allergen/additive codes + 1–2 price columns) and adds the
// ingredients line when `show-ing`.
#let g-item-row(it, has-large, show-ing, sz) = {
  let codes = csv-codes(it.at("allergen_codes", default: ""))
  for c in csv-codes(it.at("additive_codes", default: "")) { codes.push(c) }
  let codes-str = if codes.len() > 0 { " " + codes.join(",") } else { "" }

  let number = it.at("menu_number", default: none)
  let num-str = if number == none { "" } else { str(number) + ". " }

  let small = fmt-eur(it.at("price_small_cents", default: 0))
  let large = if it.at("price_large_cents", default: none) != none {
    fmt-eur(it.price_large_cents)
  } else { "" }

  let spicy-mark = if it.at("is_spicy", default: false) {
    [ #text(fill: col-red)[🌶]]
  } else { [] }

  let desc = it.at("description", default: none)
  // Establish a LOCAL paragraph context so the name→ingredients line gap is
  // tight and scales with this row's small font. The theme's global
  // `par(leading: 0.75em)` is measured against the 11.5pt body, which on a
  // ~5pt row leaves a gap ~2× the font — the "much too much line spacing".
  // `sz.lead` (absolute pt) pins it to the actual row size instead.
  let name-cell = {
    set par(leading: sz.lead, spacing: sz.lead)
    text(size: sz.name, weight: "bold")[#num-str#it.name#spicy-mark]
    text(size: sz.code, fill: col-muted)[#codes-str]
    if show-ing and desc != none and desc != "" {
      linebreak()
      text(size: sz.ing, fill: col-muted, style: "italic")[#desc]
    }
  }

  let cells = (
    name-cell,
    align(right, text(size: sz.price, weight: "bold")[#small]),
  )
  // `breakable: false` keeps a name and its ingredients line together: at a
  // column boundary the whole row moves to the next column instead of leaving
  // the name stranded at the bottom with its "mit …" line orphaned on top of
  // the next column.
  if has-large {
    cells.push(align(right, text(size: sz.price, weight: "bold")[#large]))
    block(spacing: sz.row, breakable: false, grid(
      columns: (1fr, 12mm, 12mm), gutter: 1mm, ..cells
    ))
  } else {
    block(spacing: sz.row, breakable: false, grid(
      columns: (1fr, 12mm), gutter: 1mm, ..cells
    ))
  }
}

// One category card. Same frame/colors as the theme's `category-card`.
// `atomic-max`: categories with this many items or fewer are kept whole
// (breakable: false) so a short category can't be split across a column
// boundary — which strands its header in one column and its rows in the next,
// out of reading order (the "Pizzen — mit frischen Tomaten" glitch). Larger
// categories stay breakable (the 33/22/18-item lists are taller than a column
// and MUST flow). Pass 0 to keep everything breakable (the trifold, which is
// at the 2-page edge and can't spare the wasted column bottoms).
#let g-category-card(cat, show-ing, sz, atomic-max) = {
  let first-with-large = cat.items.find(it => it.at("price_large_cents", default: none) != none)
  let has-large = first-with-large != none

  let header-row = if has-large {
    let small-label = first-with-large.at("size_small_label", default: "")
    let large-label = first-with-large.at("size_large_label", default: "")
    // Tight local leading so the size-label row hugs the title instead of
    // inheriting the theme's 0.75em-of-11.5pt line height.
    block(spacing: 0pt, par(leading: sz.lead)[
      #grid(
        columns: (1fr, 12mm, 12mm),
        gutter: 1mm,
        [],
        align(right, text(size: sz.label, fill: col-muted)[#small-label]),
        align(right, text(size: sz.label, fill: col-muted)[#large-label]),
      )
    ])
  } else { [] }

  block(
    fill: col-pink,
    stroke: 0.5pt + col-pink-2,
    radius: 2pt,
    inset: sz.inset,
    breakable: cat.items.len() > atomic-max,
    width: 100%,
    spacing: 2mm,
  )[
    // One local block-spacing for the WHOLE card so the gap before the first
    // row matches the gap between every later row. (Previously the first row
    // lived inside a separate breakable:false block and inherited the theme's
    // 0.79em block-spacing → the inconsistent extra gap on the first entry.)
    #set block(spacing: sz.row)
    // Keep the title + first row together so the title never orphans at a
    // column bottom, but space them with the SAME `sz` gaps as everything else.
    #block(breakable: false, spacing: sz.row)[
      // Tight leading on the title too: a two-line category name (e.g.
      // "Pizza Spezialitäten — mit versch. Käsesorten") was the worst offender,
      // getting 0.75em-of-11.5pt between its wrapped lines. Pin it to the
      // title size so the header block hugs the first row.
      #block(spacing: sz.titlegap, par(leading: sz.titlegap)[
        #text(size: sz.title, weight: "bold", fill: col-red, style: "italic")[#cat.name]
      ])
      #header-row
      #v(sz.titlegap)
      #g-item-row(cat.items.first(), has-large, show-ing, sz)
    ]
    #for it in cat.items.slice(1) {
      g-item-row(it, has-large, show-ing, sz)
    }
  ]
}

// Extras card: reuse the theme's `extras-card` verbatim — it carries no
// per-item description, so it's identical in both variants. We only adjust
// nothing here; the theme binding handles the free-text extras lines.

// Flat (card, item-count) list, geometry-rendered. Mirrors the theme's
// `card-list` (categories + Pizza/Pasta extras) but with our rows.
#let g-card-list(show-ing, sz, atomic-max) = {
  let xs = ()
  for cat in data.categories {
    xs.push((card: g-category-card(cat, show-ing, sz, atomic-max), n: cat.items.len()))
  }
  if extras-pizza.len() > 0 {
    xs.push((card: extras-card("Pizza-Extras", extras-pizza), n: extras-pizza.len()))
  }
  if extras-pasta.len() > 0 {
    xs.push((card: extras-card("Pasta-Extras", extras-pasta), n: extras-pasta.len()))
  }
  xs
}

#let g-cards-to-content(cards) = {
  for c in cards [
    #c
    #v(2mm)
  ]
}

#if is-a5 {
  // ===== DIN A5 Zickzackfalz: 3 panels × 2 sides ===========================
  // Reading order of the 6 finished panels: P1 cover, P2–P5 menu, P6 QR.
  // Duplex with a long-edge flip folds so the cover is the outer front and the
  // QR the outer back. That requires the BACK side's panels to be placed
  // right-to-left relative to reading order:
  //   FRONT (page 1) L→R:  cover | menuA | menuB
  //   BACK  (page 2) L→R:  QR    | menuD | menuC   (reversed)
  // After the flip, P4(menuC) backs P3(menuB), P5(menuD) backs P2(menuA),
  // P6(QR) backs P1(cover).

  // Each A5 menu PANEL holds 2 columns → 2 panels per side = 4 columns/side,
  // 8 menu columns total across both sides. That's the SAME column count as
  // the trifold (which fits this menu well), so we reuse the trifold's
  // per-row text sizing instead of shrinking — the column width (~70mm) is
  // comparable to the trifold's, so a category never gets clipped. No height
  // box / no clipping: the menu flows + balances across the 4 columns per
  // side exactly like the trifold's `columns(4, …)`.
  //
  // Split front/back by item count. Both sides have equal menu room (4 cols),
  // but the BACK column flow ends with the allergen legend, and category
  // breaks are whole-category, so an even 50/50 spills the front past its 4
  // columns onto a 3rd page. Bias the cut to the back (front ~45%) so each
  // side's 4 columns hold their half with no overflow. The `sz` profile keeps
  // both variants to 2 pages (one fold sheet). The A5 has column headroom, so
  // small categories (≤9 items) snap whole — no orphaned headers / broken
  // reading order across the 4-column flow.
  let cards = g-card-list(show-ingredients, sz, 9)
  let total = cards.fold(0, (a, c) => a + c.n)
  let half = total * 0.45
  let front = ()
  let back = ()
  let acc = 0
  for c in cards {
    if acc < half { front.push(c.card) } else { back.push(c.card) }
    acc += c.n
  }
  // 4 columns across the 2-panel menu area = 2 columns per folded panel.
  let menu-area(cards, tail) = pad(x: 4mm, y: 6mm,
    columns(4, gutter: 4mm, g-cards-to-content(cards) + tail),
  )

  // Fold + crop marks for the current page (2 interior seams + 4 corners),
  // drawn in the bleed margin so they're trimmed away. The full-bleed design
  // fills the page black, so the marks are WHITE (black-on-black would be
  // invisible to both screen and plate). 0.4pt so the print shop sees them.
  let marks = {
    let mk = 0.4pt + white
    for i in range(1, a5-panels) {
      let x = a5-panel-w * i
      place(top + left, dx: x, dy: -a5-bleed,
        line(start: (0pt, 0pt), end: (0pt, a5-bleed), stroke: mk))
      place(top + left, dx: x, dy: a5-trim-h,
        line(start: (0pt, 0pt), end: (0pt, a5-bleed), stroke: mk))
    }
    let corners = (
      (0mm, 0mm, -1, -1), (a5-trim-w, 0mm, 1, -1),
      (0mm, a5-trim-h, -1, 1), (a5-trim-w, a5-trim-h, 1, 1),
    )
    for (cx, cy, sx, sy) in corners {
      place(top + left, dx: cx, dy: cy,
        line(start: (0pt, 0pt), end: (sx * a5-bleed, 0pt), stroke: mk))
      place(top + left, dx: cx, dy: cy,
        line(start: (0pt, 0pt), end: (0pt, sy * a5-bleed), stroke: mk))
    }
  }

  // FRONT (page 1): cover | [front menu over 2 panels = 4 cols].
  grid(
    columns: (a5-panel-w, 2 * a5-panel-w),
    gutter: 0mm,
    cover-panel,
    menu-area(front, []),
  )
  marks

  pagebreak()

  // BACK (page 2): QR (placed left so it folds to the outer back) | [back menu
  // over 2 panels = 4 cols, legend at the end]. With the duplex long-edge flip
  // the QR panel lands behind the cover.
  grid(
    columns: (a5-panel-w, 2 * a5-panel-w),
    gutter: 0mm,
    qr-panel,
    menu-area(back, v(2mm) + legend),
  )
  marks
} else {
  // ===== Tri-fold (default): the two-page 443×210 layout ===================
  // Geometry-rendered rows so the DEFAULT carries ingredients. Page 1 takes a
  // bit more than half because page 2 also carries the QR panel + the legend,
  // which together eat column space — aim ~57% of items on page 1. The
  // ingredients menu is denser per row, so the trifold uses the same `sz`
  // squeeze (kept to 2 pages; `?condensed=1` strips ingredients for the airy
  // in-house menu). atomic-max = 0: the trifold is at the 2-page edge, so
  // every category stays breakable (snapping any whole costs a 3rd page —
  // measured). Its last column has some harmless slack instead.
  let cards = g-card-list(show-ingredients, sz, 0)
  let total = cards.fold(0, (a, c) => a + c.n)
  let target = total * 0.57
  let acc = 0
  let split-at = 0
  for c in cards {
    if acc + c.n > target { break }
    acc += c.n
    split-at += 1
  }
  let p1 = cards.slice(0, split-at).map(c => c.card)
  let p2 = cards.slice(split-at).map(c => c.card)

  // Page 1: cover image (left panel) + menu over the right two panels as one
  // balanced 4-column flow.
  grid(
    columns: (panel-w, 2 * panel-w),
    gutter: 0mm,
    cover-panel,
    pad(x: 3mm, y: 6mm,
      columns(4, gutter: 3mm, g-cards-to-content(p1)),
    ),
  )

  pagebreak()

  // Page 2: QR (left panel) + the rest of the menu as a 4-column flow, legend
  // appended so it lands in the last column.
  grid(
    columns: (panel-w, 2 * panel-w),
    gutter: 0mm,
    pad(x: 3mm, y: 6mm)[#qr-panel],
    pad(x: 3mm, y: 6mm,
      columns(4, gutter: 3mm, g-cards-to-content(p2) + v(2mm) + legend),
    ),
  )
}
