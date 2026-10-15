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
// The Rust handler passes a Bool; the `typst` CLI (local design iteration via
// `--input show-ingredients=false`) can only pass strings — accept both.
#let show-ingredients = {
  let v = sys.inputs.at("show-ingredients", default: true)
  if type(v) == str { v != "false" and v != "0" } else { v }
}

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
// size-label header leading. `row` is the MINIMUM row gap: the packer widens
// it (feathering) to use up column slack, and scales the whole profile down
// if the menu outgrows the 8 columns (see "Measured column packer").
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
  // `breakable: false` keeps a name and its ingredients line together (the
  // packer never splits inside a unit anyway). No `spacing:` here — the
  // enclosing card fragment sets the row gap, which may be feathered.
  if has-large {
    cells.push(align(right, text(size: sz.price, weight: "bold")[#large]))
    block(breakable: false, grid(columns: (1fr, 12mm, 12mm), gutter: 1mm, ..cells))
  } else {
    block(breakable: false, grid(columns: (1fr, 12mm), gutter: 1mm, ..cells))
  }
}


// ===========================================================================
// Measured column packer
// ===========================================================================
// Typst's `columns()` fills each column to the bottom and dumps the remainder
// into the last one, and a page split guessed from ITEM counts (57% / 45%)
// can't know how tall a row really is (a two-line ingredients line, a wrapped
// title). The result was ragged column bottoms, a half-empty last column on
// page 1 and — in the condensed variant — a page-1 column left entirely blank.
//
// So the geometry layer lays the columns out itself:
//   1. Every card is split into UNITS: a head (title + size labels + first
//      row, never separated) and one unit per further row.
//   2. Each unit is `measure()`d at the card's inner width — real heights,
//      not counts.
//   3. `g-pack` fills the 8 columns (4 per page) first-fit with a height cap
//      T; `g-balance` binary-searches the smallest T that still fits, so all
//      columns end at (nearly) the same height instead of the last one
//      running short.
//   4. Leftover height is first absorbed by feathering the row gap (up to
//      `feather-max`), so the rows breathe a little instead of leaving a void.
//   5. Each column is a fixed-height box and its last card fragment gets
//      `height: 1fr`, so the pink card background runs down to the column
//      end — every column bottom lines up.
//   6. If the menu doesn't fit 8 columns even at full height, the row size
//      profile is scaled down in 5% steps (to 80%) instead of spilling onto a
//      3rd page the fold can't hold.
// Only theme EXPORTS are used (`col-*`, `fmt-eur`, `csv-codes`, `data`,
// `extras-*`, `legend`), so this works against any forked `pdf_themes` row.

#let card-gap = 2mm          // vertical gap between cards in a column
#let card-stroke = 0.5pt
#let n-menu-cols = 8         // 4 per page × 2 pages, both formats
// Row-gap feathering cap (× the profile's `row`). 1.0 = off: slack stays as
// stretched card background only. See `g-feather`.
#let feather-max = 2.0
#let debug-fill = sys.inputs.at("debug", default: "0") == "1"

#let g-title(name, sz) = block(spacing: sz.titlegap, par(leading: sz.titlegap)[
  // Tight leading on the title: a two-line category name ("Pizza
  // Spezialitäten — mit versch. Käsesorten") otherwise gets the theme's
  // body leading between its wrapped lines.
  #text(size: sz.title, weight: "bold", fill: col-red, style: "italic")[#name]
])

// Category → units. Head = title + size-label row + first row.
#let g-category-units(cat, show-ing, sz) = {
  let first-with-large = cat.items.find(it => it.at("price_large_cents", default: none) != none)
  let has-large = first-with-large != none

  let header-row = if has-large {
    let small-label = first-with-large.at("size_small_label", default: "")
    let large-label = first-with-large.at("size_large_label", default: "")
    // `set par` rather than `par(..)[grid]`: a grid can't live inside a
    // paragraph — Typst warned and silently dropped the label row.
    block(spacing: 0pt)[
      #set par(leading: sz.lead)
      #grid(
        columns: (1fr, 12mm, 12mm),
        gutter: 1mm,
        [],
        align(right, text(size: sz.label, fill: col-muted)[#small-label]),
        align(right, text(size: sz.label, fill: col-muted)[#large-label]),
      )
    ]
  } else { [] }

  // The head sets its OWN internal spacing so its measured height is exact
  // whatever row gap the fragment around it ends up with.
  let head = block(breakable: false)[
    #set block(spacing: sz.row)
    #g-title(cat.name, sz)
    #header-row
    #v(sz.titlegap)
    #g-item-row(cat.items.first(), has-large, show-ing, sz)
  ]
  let units = (head,)
  for it in cat.items.slice(1) {
    units.push(g-item-row(it, has-large, show-ing, sz))
  }
  units
}

// Extras lines ("Krabben 1 €") → (name, price). Same rule as the theme's
// `extras-card`: the price column starts at the LEFTMOST space after which
// the rest of the line is price-ish (digits , . € spaces) and holds a digit,
// so "Kräuterbutter 0,60 €" splits before "0,60", not between "0,60" and "€".
#let g-parse-extra(line) = {
  let digits = "0123456789"
  let price-chars = digits + ",.€" + "\u{00A0}"
  let chars = line.clusters()
  let split-at = none
  let i = chars.len()
  let all-price = true
  while i > 0 {
    i = i - 1
    let c = chars.at(i)
    if c == " " and all-price {
      let candidate = chars.slice(i + 1).join("")
      let has-digit = candidate.clusters().any(d => digits.contains(d))
      if has-digit { split-at = i }
    } else if not (price-chars.contains(c) or c == " ") {
      all-price = false
    }
  }
  if split-at == none { return (line, "") }
  let name = chars.slice(0, split-at).join("").trim()
  let price = chars.slice(split-at + 1).join("").trim().replace(" ", "\u{00A0}")
  (name, price)
}

#let g-extras-row(name, price, sz) = block(breakable: false, grid(
  columns: (1fr, 14mm),
  gutter: 1mm,
  text(size: sz.name)[#name],
  align(right, text(size: sz.price, weight: "bold")[#price]),
))

// Extras card → units, same shape as a category so it packs + stretches the
// same way (the theme's `extras-card` is one opaque block and couldn't be
// stretched to the column end without a second frame inside it).
#let g-extras-units(title, lines, sz) = {
  let cleaned = lines.map(l => l.trim()).filter(l => l.len() > 0)
  if cleaned.len() == 0 { return () }
  let rows = cleaned.map(raw => {
    let (name, price) = g-parse-extra(raw)
    g-extras-row(name, price, sz)
  })
  let head = block(breakable: false)[
    #set block(spacing: sz.row)
    #g-title(title, sz)
    #v(sz.titlegap)
    #rows.first()
  ]
  (head,) + rows.slice(1)
}

// All cards in flow order. `atomic`: keep the whole card in one column (short
// categories, so a header never strands away from its rows — the "Pizzen —
// mit frischen Tomaten" glitch). `atomic-max` 0 = everything may break.
#let g-cards(show-ing, sz, atomic-max) = {
  let xs = ()
  for cat in data.categories {
    if cat.items.len() == 0 { continue }
    xs.push((units: g-category-units(cat, show-ing, sz), atomic: cat.items.len() <= atomic-max))
  }
  for (title, lines) in (("Pizza-Extras", extras-pizza), ("Pasta-Extras", extras-pasta)) {
    let units = g-extras-units(title, lines, sz)
    if units.len() > 0 {
      xs.push((units: units, atomic: units.len() <= atomic-max))
    }
  }
  xs
}

// Attach measured unit heights. `iw` = inner card width (column minus inset).
// Must run inside `context` (measure needs the live text styles).
#let g-measure-cards(cards, iw) = cards.map(c => (
  ..c,
  heights: c.units.map(u => measure(width: iw, u).height),
))

// Height of the card fragment holding units a..b (b exclusive): inset top +
// bottom, the unit heights, and the row `gap` between adjacent units.
#let g-frag-h(card, a, b, sz, gap) = {
  let h = 2 * sz.inset
  for k in range(a, b) { h += card.heights.at(k) }
  h + (b - a - 1) * gap
}

// First-fit packing into `caps.len()` columns, each with its own height cap.
// A card fragment is (card, a, b). Non-atomic cards break between units
// (never inside a unit: the head, or a name + its ingredients line). Returns
// (cols, overflow); when something can't fit even an empty column — or the
// last column is full — it is forced in and `overflow` is set, so the
// caller can pick a smaller size profile instead of dropping items.
#let g-pack(cards, caps, sz, gap) = {
  let ncols = caps.len()
  let eps = 0.5pt              // measure-vs-layout rounding tolerance
  let min-units = 2            // smallest fragment on either side of a break
  let cols = ()
  let col = ()
  let used = 0pt
  let ci = 0
  let overflow = false
  for card in cards {
    let n = card.units.len()
    let a = 0
    while a < n {
      let start = if col.len() == 0 { 0pt } else { used + card-gap }
      let room = caps.at(calc.min(ci, ncols - 1)) - start - eps
      // Extend the fragment one unit at a time while it still fits.
      let b = a
      let h = 2 * sz.inset
      while b < n {
        let step = card.heights.at(b) + (if b > a { gap } else { 0pt })
        if h + step > room { break }
        h += step
        b += 1
      }
      let greedy-b = b
      if not card.atomic and b < n {
        // The card continues in the next column: never leave a lone head (or
        // single row) at a column bottom, and never strand a single row at
        // the top of the next one. Fewer than `min-units` here → move the
        // whole fragment over; a 1-unit remainder → pull one unit back.
        if b - a < min-units {
          b = a
        } else if n - b < min-units {
          // 1-unit remainder: pull one back if this side can spare it,
          // otherwise (a 3-unit card) move the whole card over.
          if b - a > min-units { b -= min-units - (n - b) } else { b = a }
        }
        if b != greedy-b { h = g-frag-h(card, a, b, sz, gap) }
      }
      let last-col = ci >= ncols - 1
      if b == a or (card.atomic and b < n) {
        // Nothing (or not the whole atomic card) fits here.
        if col.len() > 0 and not last-col {
          // Column has content: close it and retry in the next one.
          cols.push(col)
          col = ()
          used = 0pt
          ci += 1
          continue
        }
        // Empty column (or the last one) and it still doesn't fit: force.
        overflow = true
        b = if card.atomic { n } else { calc.max(greedy-b, a + 1) }
        h = g-frag-h(card, a, b, sz, gap)
      }
      col.push((card: card, a: a, b: b))
      used = start + h
      a = b
      if a < n {
        // Card continues in the next column.
        if last-col {
          overflow = true
        } else {
          cols.push(col)
          col = ()
          used = 0pt
          ci += 1
        }
      }
    }
  }
  cols.push(col)
  while cols.len() < ncols { cols.push(()) }
  (cols: cols, overflow: overflow)
}

// Smallest column height T at which everything packs into `ncols` columns.
// The last column also carries the legend (`tail-h`, pinned to its bottom),
// so its cap is `min(T, H - tail-h)`. Binary search — 14 halvings of ~200mm
// is ~0.01mm, far below any visible difference. `fits: false` means even
// T = H overflows; the returned pack is the forced one.
#let g-caps-for(ncols, H, tail-h, T) = range(ncols).map(i =>
  if i == ncols - 1 { calc.min(T, H - tail-h) } else { T }
)

#let g-balance(cards, ncols, H, tail-h, sz, gap) = {
  let best = g-pack(cards, g-caps-for(ncols, H, tail-h, H), sz, gap)
  if best.overflow { return (pack: best, T: H, fits: false) }
  let lo = 0pt
  let hi = H
  for _ in range(14) {
    let mid = (lo + hi) / 2
    let p = g-pack(cards, g-caps-for(ncols, H, tail-h, mid), sz, gap)
    if p.overflow { lo = mid } else { hi = mid; best = p }
  }
  (pack: best, T: hi, fits: true)
}

// Feathering: the largest row gap in [sz.row, feather-max × sz.row] at which
// the menu still fits full-height columns. Slack that would otherwise sit as
// a pink void under the last row of every column becomes a slightly airier
// row rhythm instead; whatever is left after the cap is still stretched.
// Pure arithmetic (unit heights don't depend on the gap) — no re-measuring.
#let g-feather(cards, ncols, H, tail-h, sz) = {
  let caps = g-caps-for(ncols, H, tail-h, H)
  let lo = sz.row
  let hi = sz.row * feather-max
  if not g-pack(cards, caps, sz, hi).overflow { return hi }
  for _ in range(10) {
    let mid = (lo + hi) / 2
    if g-pack(cards, caps, sz, mid).overflow { hi = mid } else { lo = mid }
  }
  lo
}

#let g-scale(sz, f) = sz.pairs().map(((k, v)) => (k, v * f)).to-dict()

// Full layout for one render: measure, balance, and shrink the size profile
// only if the menu can't fit 8 columns at all. Returns
// (cols: 8 arrays of fragments, sz: the profile used, gap: row gap, scale,
// T, fits).
#let g-layout(cw, H, sz, atomic-max) = {
  let tail-h = measure(width: cw, legend).height + card-gap
  let result = none
  for f in (1.0, 0.95, 0.9, 0.85, 0.8) {
    let szf = g-scale(sz, f)
    let cards = g-measure-cards(g-cards(show-ingredients, szf, atomic-max), cw - 2 * szf.inset)
    let bal = g-balance(cards, n-menu-cols, H, tail-h, szf, szf.row)
    let gap = szf.row
    if bal.fits and feather-max > 1.0 {
      gap = g-feather(cards, n-menu-cols, H, tail-h, szf)
      bal = g-balance(cards, n-menu-cols, H, tail-h, szf, gap)
    }
    result = (cols: bal.pack.cols, sz: szf, gap: gap, scale: f, T: bal.T, fits: bal.fits)
    if bal.fits { break }
  }
  result
}

// One card fragment: the same frame as the theme's `category-card`. The last
// fragment of a column is stretched (`1fr`) to the column bottom.
#let g-fragment(frag, sz, gap, stretch) = block(
  fill: col-pink,
  stroke: card-stroke + col-pink-2,
  radius: 2pt,
  inset: sz.inset,
  width: 100%,
  height: if stretch { 1fr } else { auto },
  breakable: false,
)[
  #set block(spacing: gap)
  #for k in range(frag.a, frag.b) { frag.card.units.at(k) }
]

// One fixed-height column. `tail` (the legend) sits at the very bottom; the
// last fragment stretches to meet it.
#let g-column(frags, cw, H, sz, gap, tail) = block(width: cw, height: H, breakable: false)[
  #set block(spacing: 0pt)
  #for (i, fr) in frags.enumerate() {
    if i > 0 { v(card-gap) }
    g-fragment(fr, sz, gap, i == frags.len() - 1)
  }
  #if tail != none {
    if frags.len() > 0 { v(card-gap) }
    tail
  }
]

// 4 columns of one page. `first` = index of the first column in the 8-col
// flow; the legend goes into the last column of the last page.
#let g-menu-area(L, first, cw, H, gutter, with-legend) = grid(
  columns: (cw,) * 4,
  gutter: gutter,
  ..range(4).map(i => g-column(
    L.cols.at(first + i), cw, H, L.sz, L.gap,
    if with-legend and i == 3 { legend } else { none },
  )),
)

// Tiny corner note for local tuning (`--input debug=1`), never set by Rust.
#let g-debug(L) = if debug-fill {
  place(top + left, dx: 1mm, dy: 0.5mm, text(size: 4pt, fill: col-muted)[
    fill: scale #L.scale · gap #calc.round(L.gap / 1pt, digits: 2)pt · T #calc.round(L.T / 1mm, digits: 1)mm · fits #L.fits
  ])
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
  //
  // Each A5 menu PANEL holds 2 columns → 4 columns per side, 8 total — the
  // same column count as the trifold at a comparable width (~70mm), so the
  // same `sz` profile serves both. The packer balances all 8 columns across
  // both sides; small categories (≤9 items) snap whole so no header strands
  // away from its rows.
  let pad-x = 4mm
  let pad-y = 6mm
  let gutter = 4mm
  let cw = (2 * a5-panel-w - 2 * pad-x - 3 * gutter) / 4
  let H = a5-trim-h - 2 * pad-y

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

  // FRONT (page 1): cover | [menu columns 1–4 over 2 panels].
  context {
    let L = g-layout(cw, H, sz, 9)
    grid(
      columns: (a5-panel-w, 2 * a5-panel-w),
      gutter: 0mm,
      cover-panel,
      pad(x: pad-x, y: pad-y, g-menu-area(L, 0, cw, H, gutter, false)),
    )
    g-debug(L)
  }
  marks

  pagebreak()

  // BACK (page 2): QR (placed left so it folds to the outer back) | [menu
  // columns 5–8, legend at the bottom of the last]. With the duplex long-edge
  // flip the QR panel lands behind the cover.
  context {
    let L = g-layout(cw, H, sz, 9)
    grid(
      columns: (a5-panel-w, 2 * a5-panel-w),
      gutter: 0mm,
      qr-panel,
      pad(x: pad-x, y: pad-y, g-menu-area(L, 4, cw, H, gutter, true)),
    )
  }
  marks
} else {
  // ===== Tri-fold (default): the two-page 443×210 layout ===================
  // Page 1: cover (left panel) + menu columns 1–4 over the right two panels.
  // Page 2: QR (left panel) + menu columns 5–8, legend at the bottom of the
  // last. The packer balances all 8 columns, so page 2's QR panel and legend
  // no longer need a guessed item-count bias. atomic-max = 0: the trifold is
  // at the 2-page edge, so every category may break at a column boundary.
  let pad-x = 3mm
  let pad-y = 6mm
  let gutter = 3mm
  let cw = (2 * panel-w - 2 * pad-x - 3 * gutter) / 4
  let H = 210mm - 2 * pad-y

  context {
    let L = g-layout(cw, H, sz, 0)
    grid(
      columns: (panel-w, 2 * panel-w),
      gutter: 0mm,
      cover-panel,
      pad(x: pad-x, y: pad-y, g-menu-area(L, 0, cw, H, gutter, false)),
    )
    g-debug(L)
  }

  pagebreak()

  context {
    let L = g-layout(cw, H, sz, 0)
    grid(
      columns: (panel-w, 2 * panel-w),
      gutter: 0mm,
      pad(x: pad-x, y: pad-y)[#qr-panel],
      pad(x: pad-x, y: pad-y, g-menu-area(L, 4, cw, H, gutter, true)),
    )
  }
}
