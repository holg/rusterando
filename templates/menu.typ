// Davids Pizzeria — printable menu (v1)
// Driven by JSON in `sys.inputs.data`. Build the JSON in Rust; tweak this file
// alone to iterate the visual design.

#let parse-data() = {
  let raw = sys.inputs.at("data", default: none)
  if raw == none {
    // Tiny preview sample so the file renders standalone in a Typst IDE.
    return (
      branding: (
        name: "Mein Restaurant",
        tagline: "",
        address: "Musterstraße 1, 12345 Musterstadt",
        phone: "Tel. 0123-4567890",
        hours_lines: ("Mo–So 11:30–22:00",),
        delivery_lines: ("Stadt ab 15 € + 1,50 €",),
        extras_pizza_lines: ("Krabben 1 €",),
        extras_pasta_lines: ("Krabben 1 €",),
        cover_mode: "text",
      ),
      categories: ((
        id: "demo",
        name: "Pizza Spezialitäten",
        items: ((
          menu_number: "1",
          name: "Pizza Margherita",
          description: "mit Tomaten und Goudakäse",
          allergen_codes: "a,c,g",
          additive_codes: "2,6",
          is_spicy: false,
          price_small_cents: 400,
          price_large_cents: 700,
          size_small_label: "22cm",
          size_large_label: "30cm",
        ),),
      ),),
      allergens: ((code: "a", name_de: "Gluten"),),
      additives: ((code: "2", name_de: "Antioxidationsmittel"),),
      site_url: "https://example.com",
    )
  }
  json(bytes(raw))
}

#let data = parse-data()

// ----- Helpers -------------------------------------------------------------

#let fmt-eur(cents) = {
  if cents == none { return "" }
  let euros = calc.div-euclid(cents, 100)
  let rest = calc.rem-euclid(cents, 100)
  let pad = if rest < 10 { "0" } else { "" }
  // Non-breaking space (\u{00A0}) between amount and € so two-digit
  // prices like "10,00 €" never wrap onto two lines inside the narrow
  // price column.
  str(euros) + "," + pad + str(rest) + "\u{00A0}€"
}

#let csv-codes(s) = {
  if s == none or s == "" { return () }
  s.split(",").map(c => c.trim())
}

// ----- Color palette -------------------------------------------------------
#let col-bg     = white
#let col-text   = rgb("#2a1a0f")
#let col-muted  = rgb("#7a6a5f")
#let col-pink   = rgb("#fce5e0")
#let col-pink-2 = rgb("#f5c8c0")
#let col-red    = rgb("#c8102e")
#let col-side   = rgb("#1a1a1a")
#let col-side-fg = rgb("#fefcf6")
#let col-cyan   = rgb("#1eaedb")
#let col-orange = rgb("#f4a261")

// ----- Theme style wrapper --------------------------------------------------
// THEME CONTRACT: this file is a *styling* module — palette + render fns. It
// does NOT set the page or emit the final layout; the geometry layer
// (geometry.typ, owned by the Rust handler) does that, per `?format=`, and
// imports this module. `#set` rules don't cross an `#import`, so the page text
// styling is exposed as a wrapper fn the geometry layer wraps its content in.
//
// Fonts come from the binary itself (build.rs + pdf.rs) so Mac/Linux render
// identically. Inter handles Latin; Noto Color Emoji covers 🌶 etc.
#let theme-bg = col-bg
#let theme-styles(body) = {
  set text(
    font: ("Inter", "Noto Color Emoji"),
    size: 8pt,
    fill: col-text,
  )
  set par(leading: 0.45em)
  set block(spacing: 0.55em)
  body
}

// ----- Side panel ---------------------------------------------------------

#let side-panel = rect(
  width: 100%,
  height: 100%,
  fill: col-side,
  stroke: none,
  inset: 12mm,
)[
  #set text(fill: col-side-fg)

  #v(2mm)
  #align(center, text(size: 32pt, weight: "bold")[#data.branding.name])

  #v(4mm)
  #align(center, box(
    width: 86%,
    radius: 3pt,
    clip: true,
  )[
    #image("/img/ladenfront.jpg", width: 100%)
  ])

  #v(6mm)
  #align(center, box(
    fill: col-orange,
    inset: 6pt,
    radius: 3pt,
    width: 78%,
  )[
    #set text(fill: col-text, size: 9pt)
    *Pizzablech 60×40 cm — 28 €* \
    mit 3 Zutaten nach Wahl, +3 € je weitere
  ])

  #v(2mm)
  #align(center, box(
    fill: col-orange,
    inset: 6pt,
    radius: 3pt,
    width: 78%,
  )[
    #set text(fill: col-text, size: 9pt)
    *Pizza 36 cm Goudakäse — 14 €* \
    +1 € je weitere Zutat
  ])

  #v(6mm)
  #align(center)[
    #set text(size: 8pt, weight: "bold")
    Wir liefern \
    #set text(weight: "regular")
    #for line in data.branding.delivery_lines {
      line
      linebreak()
    }
  ]

  #v(4mm)
  #align(center)[
    #set text(size: 8pt, weight: "bold")
    ÖFFNUNGSZEITEN \
    #set text(weight: "regular")
    #for line in data.branding.hours_lines {
      line
      linebreak()
    }
  ]

  #v(1fr)

  #align(center, text(size: 9pt, weight: "bold")[
    #upper(data.branding.address) \
    #data.branding.phone
  ])
]

// ----- One menu item row ---------------------------------------------------

// Render one menu item. `has-large` decides whether we draw the second
// price column at all — passed down from the category so every row in
// a card has the same column shape.
#let item-row(it, has-large) = {
  let codes = csv-codes(it.at("allergen_codes", default: ""))
  for c in csv-codes(it.at("additive_codes", default: "")) { codes.push(c) }
  let codes-str = if codes.len() > 0 {
    " " + codes.join(",")
  } else { "" }

  let number = it.at("menu_number", default: none)
  let num-str = if number == none { "" } else { str(number) + ". " }

  let small = fmt-eur(it.at("price_small_cents", default: 0))
  let large = if it.at("price_large_cents", default: none) != none {
    fmt-eur(it.price_large_cents)
  } else { "" }

  let spicy-mark = if it.at("is_spicy", default: false) {
    [ #text(fill: col-red)[🌶]]
  } else { [] }

  let cells = (
    {
      text(size: 7pt, weight: "bold")[#num-str#it.name#spicy-mark]
      text(size: 5pt, fill: col-muted)[#codes-str]
      // Descriptions intentionally omitted from the PDF so we can
      // afford generous line+card spacing instead. Web menu keeps them.
    },
    align(right, text(size: 7pt, weight: "bold")[#small]),
  )
  // Inter-row spacing tuned to use the leftover vertical room in
  // the new tri-fold layout (4 menu sub-cols × 2 pages). 0.6em
  // gives readable airy rows without overflowing onto a 3rd page.
  if has-large {
    cells.push(align(right, text(size: 7pt, weight: "bold")[#large]))
    block(spacing: 0.6em, grid(
      columns: (1fr, 12mm, 12mm), gutter: 1mm, ..cells
    ))
  } else {
    block(spacing: 0.6em, grid(
      columns: (1fr, 12mm), gutter: 1mm, ..cells
    ))
  }
}

// ----- One category card ---------------------------------------------------
//
// Column shape adapts to the items in this category:
//   - if no item has price_large_cents, the second column is hidden
//     (drinks, salads, desserts — anything one-priced).
//   - if at least one item has it, both columns render and the headers
//     come from that item's size_small_label / size_large_label, which
//     keeps the labels honest per-category instead of hard-coding
//     pizza-only values like "22cm" / "30cm".

#let category-card(cat) = {
  // Find the first item with a large price; that item supplies the
  // size labels for the whole card. If no item has one, no header row.
  let first-with-large = cat.items.find(it => it.at("price_large_cents", default: none) != none)
  let has-large = first-with-large != none

  let header-row = if has-large {
    let small-label = first-with-large.at("size_small_label", default: "")
    let large-label = first-with-large.at("size_large_label", default: "")
    grid(
      columns: (1fr, 12mm, 12mm),
      gutter: 1mm,
      [],
      align(right, text(size: 5.5pt, fill: col-muted)[#small-label]),
      align(right, text(size: 5.5pt, fill: col-muted)[#large-label]),
    )
  } else { [] }

  // Restored generous card padding + spacing (was tightened to 3mm/1mm
  // for the single-page squeeze; the new 2-page tri-fold has room).
  block(
    fill: col-pink,
    stroke: 0.5pt + col-pink-2,
    radius: 2pt,
    inset: 4mm,
    breakable: true,
    width: 100%,
    spacing: 2mm,
  )[
    #text(size: 11pt, weight: "bold", fill: col-red, style: "italic")[#cat.name]
    #v(1mm)
    #header-row
    #v(0.5mm)
    #for it in cat.items {
      item-row(it, has-large)
    }
  ]
}

// ----- Extras card ---------------------------------------------------------
//
// Free-text lines from app_settings (one extra per line, e.g.
// "Krabben 1 €" or "Lachs 2,50 €"). We split each line on the last
// space and treat the trailing chunk as the price column if it looks
// like one; otherwise the whole line is the name and the price column
// stays empty. Matches the visual language of `category-card` so the
// extras blocks slot into the bottom of the menu grid.

#let extras-card(title, lines) = {
  // Filter out blank lines from sloppy textarea editing.
  let cleaned = lines.map(l => l.trim()).filter(l => l.len() > 0)
  if cleaned.len() == 0 { return [] }

  let parse-line(line) = {
    // Walk the clusters and find the *leftmost* space such that
    // everything from there to the end of the line is "price-ish"
    // (digits, comma/period, currency symbol, whitespace). That
    // boundary is the start of the price column. Naïvely splitting
    // on the last space breaks on "Krabben 1 €" — last space sits
    // between "1" and "€", leaving "1" stranded in the name.
    let digits = "0123456789"
    let price-chars = digits + ",.€" + "\u{00A0}"
    let is-price-char(c) = price-chars.contains(c) or c == " "
    let chars = line.clusters()
    let split-at = none
    let i = chars.len()
    let all-price = true
    while i > 0 {
      i = i - 1
      let c = chars.at(i)
      if c == " " and all-price {
        // Only accept this split if the price part actually contains
        // at least one digit (otherwise the line was all-text and
        // we'd amputate the last word into a fake price column).
        let candidate = chars.slice(i + 1).join("")
        let has-digit = false
        for d in candidate.clusters() {
          if digits.contains(d) { has-digit = true }
        }
        if has-digit { split-at = i }
      } else if not is-price-char(c) {
        all-price = false
      }
    }
    if split-at == none {
      return (line, "")
    }
    let head = chars.slice(0, split-at).join("").trim()
    let tail = chars.slice(split-at + 1).join("").trim()
    // Non-breaking space inside the price so "0,60 €" doesn't wrap.
    (head, tail.replace(" ", "\u{00A0}"))
  }

  block(
    fill: col-pink,
    stroke: 0.5pt + col-pink-2,
    radius: 2pt,
    inset: 4mm,
    breakable: true,
    width: 100%,
    spacing: 2mm,
  )[
    #text(size: 11pt, weight: "bold", fill: col-red, style: "italic")[#title]
    #v(1mm)
    #for raw in cleaned {
      let (name, price) = parse-line(raw)
      block(spacing: 0.4em, grid(
        columns: (1fr, 14mm),
        gutter: 1mm,
        text(size: 7pt)[#name],
        align(right, text(size: 7pt, weight: "bold")[#price]),
      ))
    }
  ]
}

// ----- Allergen / additive legend ------------------------------------------

#let legend = block(
  fill: rgb("#f8efe5"),
  inset: 3mm,
  radius: 2pt,
  width: 100%,
)[
  #set text(size: 5.5pt, fill: col-muted)
  *Zusatzstoffe:* #{
    let parts = data.additives.map(a => a.code + " = " + a.name_de)
    parts.join(" · ")
  }

  *Allergene:* #{
    let parts = data.allergens.map(a => a.code + ") " + a.name_de)
    parts.join(" · ")
  }
]

// ----- QR scan panel (right third of page 1) -------------------------------
//
// Was a standalone last page; now squeezed into a 148mm-wide tri-fold
// panel. Strips the protocol prefix for cleaner display. Expression
// must stay on a single line (or be parenthesised) — bare line
// continuations after `#let foo = …` are interpreted as content,
// which is how we ended up with literal ".replace(…)" text in the
// rendered PDF in an earlier version.

#let display-url = (
  data.site_url.replace("https://", "").replace("http://", "").trim("/")
)

#let qr-panel = rect(
  width: 100%,
  height: 100%,
  fill: col-bg,
  stroke: none,
  inset: 8mm,
)[
  #set text(fill: col-text)
  #align(center + horizon)[
    #text(size: 24pt, weight: "bold", fill: col-red)[Hier scannen,]
    #v(-3mm)
    #text(size: 24pt, weight: "bold", fill: col-red, style: "italic")[später bestellen]

    #v(5mm)

    #box(
      width: 70mm,
      height: 70mm,
      inset: 2mm,
      fill: white,
      stroke: 1pt + col-pink-2,
      radius: 3pt,
    )[
      #image("/img/qr.svg", width: 100%, height: 100%, fit: "contain")
    ]

    #v(3mm)
    #text(size: 14pt, weight: "bold")[#display-url]

    #v(2mm)
    #text(size: 8pt, fill: col-muted)[
      Online bestellen · Speisekarte als PDF \
      Lieferung & Abholung
    ]

    #v(6mm)
    #text(size: 8pt, fill: col-muted)[
      #data.branding.address \
      #data.branding.phone
    ]
  ]
]

// ----- Cover panel (left third of page 1) ----------------------------------
//
// The outside-front of the folded leporello (Wickelfalz: this left panel of
// the print sheet is the visible cover after the right panel folds in first).
//
// Two modes, chosen by `data.branding.cover_mode` (the `pdf_cover_mode`
// admin setting):
//   "image" — full-bleed photo from `/img/cover.jpg` (the admin Cover-
//             Bibliothek, falling back to the bundled cover in pdf.rs).
//   "text"  — a typeset text cover built from the shop's branding (name,
//             address, phone, hours). No photo — so every tenant gets a
//             correct, own-branded cover with nothing to upload.
// Anything other than "image" is treated as "text" (text is the safe default
// for a fresh tenant with no cover photo).

#let cover-image-panel = box(
  width: 100%,
  height: 100%,
  clip: true,
)[
  #image("/img/cover.jpg", width: 100%, height: 100%, fit: "cover")
]

#let cover-text-panel = rect(
  width: 100%,
  height: 100%,
  fill: col-side,
  stroke: none,
  inset: 14mm,
)[
  #set text(fill: col-side-fg)
  #set align(center)

  #v(1fr)

  #text(size: 40pt, weight: "bold")[#data.branding.name]

  #if data.branding.at("tagline", default: "") != "" [
    #v(3mm)
    #text(size: 13pt, fill: col-orange, style: "italic")[#data.branding.tagline]
  ]

  #v(8mm)
  #line(length: 40%, stroke: 0.5pt + col-side-fg)
  #v(8mm)

  #text(size: 11pt)[
    #upper(data.branding.address)
  ]

  #v(4mm)
  #text(size: 12pt, weight: "bold")[#data.branding.phone]

  #v(10mm)

  #if data.branding.hours_lines.len() > 0 [
    #text(size: 8pt, weight: "bold")[ÖFFNUNGSZEITEN]
    #v(2mm)
    #set text(size: 9pt)
    #for line in data.branding.hours_lines {
      line
      linebreak()
    }
  ]

  #v(1fr)

  #text(size: 8pt, fill: col-orange)[SPEISEKARTE]
]

#let cover-panel = if data.branding.at("cover_mode", default: "text") == "image" {
  cover-image-panel
} else {
  cover-text-panel
}

// ----- Page 1 + 2: cover + QR/ads + menu flow ------------------------------
//
// Layout (one A3 sheet folded into 3 panels per side, 6 panels total):
//
//   PAGE 1 (outside, folded shut shows leftmost panel):
//     [ COVER IMAGE | menu start | menu cont. ]
//        ~148mm        ~148mm       ~148mm
//
//   PAGE 2 (inside spread, opened up):
//     [ QR + addr/hours | menu cont. | menu cont. + legend ]
//        ~148mm            ~148mm        ~148mm
//
// To get the menu flowing across all 4 sub-columns while reserving
// the first sub-column of each page for cover/QR, we partition the
// categories list manually and lay each page out as a top-level grid.

// Two-page tri-fold layout. Each page has 3 fold-panels (sub-pages).
// 4 of the 6 panels carry menu items, each panel laid out as 2 inner
// columns. Total: 8 menu sub-columns + 1 cover panel + 1 QR-and-ads
// panel.
//
//   PAGE 1 (outside, folded shut shows leftmost panel):
//     [ COVER IMAGE | menu 2-col | menu 2-col ]
//       fold-panel    fold-panel   fold-panel
//
//   PAGE 2 (inside spread, opened up):
//     [ QR + ads/addr | menu 2-col | menu 2-col + legend ]
//       fold-panel       fold-panel   fold-panel

#let panel-w = (443mm - 12mm) / 3  // ~143.7mm per fold panel

// Pre-render menu cards so we can manually partition them across
// the 4 menu panels. Card sizes vary wildly (some categories have
// 33 items, others 2), so partitioning by ITEM count is essential —
// partitioning by card count puts all pizzas on one panel.
#let extras-pizza = data.branding.at("extras_pizza_lines", default: ())
#let extras-pasta = data.branding.at("extras_pasta_lines", default: ())

// Build a flat list of (card, item-count) tuples. `item-count` for
// extras-cards is approximated from line count (used for size only).
#let card-list = {
  let xs = ()
  for cat in data.categories {
    xs.push((card: category-card(cat), n: cat.items.len()))
  }
  if extras-pizza.len() > 0 {
    xs.push((card: extras-card("Pizza-Extras", extras-pizza), n: extras-pizza.len()))
  }
  if extras-pasta.len() > 0 {
    xs.push((card: extras-card("Pasta-Extras", extras-pasta), n: extras-pasta.len()))
  }
  xs
}

#let total-items = card-list.fold(0, (acc, c) => acc + c.n)
// Page 1 should take a bit MORE than half because page 2 also carries
// the QR-panel overhead (panel 1) AND the legend at the end of the
// menu flow, which together eat ~30mm of column space. Aim for ~57%
// of items on page 1, ~43% on page 2.
#let per-page-target = total-items * 0.57

#let split-at = {
  let acc = 0
  let idx = 0
  for c in card-list {
    if acc + c.n > per-page-target { break }
    acc += c.n
    idx += 1
  }
  idx
}

#let p1-cards = card-list.slice(0, split-at).map(c => c.card)
#let p2-cards = card-list.slice(split-at).map(c => c.card)

#let cards-to-content(cards) = {
  for c in cards [
    #c
    #v(2mm)
  ]
}

// THEME CONTRACT ENDS HERE.
//
// This module exposes (via `#import "/theme.typ": *`) everything the geometry
// layer needs to arrange the page(s) per `?format=`:
//   styling : theme-styles(body), theme-bg, col-* palette
//   pieces  : cover-panel, side-panel, qr-panel, legend
//   menu    : card-list (all category/extras cards + item counts),
//             p1-cards / p2-cards (the trifold 57/43 split),
//             cards-to-content(cards), panel-w
//   data    : data, display-url
//
// It deliberately does NOT call `#set page` or emit the final layout — that's
// the geometry layer's job (templates/geometry.typ), so page geometry / fold
// format is independent of the styling theme.
