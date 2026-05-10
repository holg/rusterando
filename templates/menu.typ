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
  str(euros) + "," + pad + str(rest) + " €"
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

// ----- Page setup ----------------------------------------------------------
#set page(
  width: 444mm,
  height: 210mm,
  margin: 0mm,
  fill: col-bg,
)

// Fonts come from the binary itself (build.rs + pdf.rs) so Mac/Linux render
// identically. Inter handles Latin; Noto Color Emoji covers 🌶 etc.
#set text(
  font: ("Inter", "Noto Color Emoji"),
  size: 8pt,
  fill: col-text,
)
#set par(leading: 0.45em)
#set block(spacing: 0.55em)

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
      let desc = it.at("description", default: none)
      // Descriptions already start with "mit …" in the seed data, so we
      // render them as-is without a redundant prefix.
      if desc != none and desc != "" [
        \ #text(size: 6pt, fill: col-muted)[#desc]
      ]
    },
    align(right, text(size: 7pt, weight: "bold")[#small]),
  )
  if has-large {
    cells.push(align(right, text(size: 7pt, weight: "bold")[#large]))
    block(spacing: 0.4em, grid(
      columns: (1fr, 9mm, 9mm), gutter: 1mm, ..cells
    ))
  } else {
    block(spacing: 0.4em, grid(
      columns: (1fr, 9mm), gutter: 1mm, ..cells
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
      columns: (1fr, 9mm, 9mm),
      gutter: 1mm,
      [],
      align(right, text(size: 5.5pt, fill: col-muted)[#small-label]),
      align(right, text(size: 5.5pt, fill: col-muted)[#large-label]),
    )
  } else { [] }

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

// ----- Page layout ---------------------------------------------------------

#grid(
  columns: (145mm, 1fr),
  gutter: 0mm,
  side-panel,
  pad(x: 6mm, y: 6mm)[
    #columns(4, gutter: 4mm)[
      #for cat in data.categories {
        category-card(cat)
        v(2mm)
      }
      #v(4mm)
      #legend
    ]
  ],
)

// ----- Final page: scan-to-order QR ----------------------------------------
//
// Customers can pull this PDF up on their phone or print it as a flyer; the
// QR makes it trivial to land back on the live ordering site.
//
// We strip the protocol prefix from the URL for cleaner display. The
// expression must stay on a single line (or be parenthesised) — bare line
// continuations after `#let foo = …` are interpreted as content, which is
// how we ended up with literal ".replace(…)" text in the rendered PDF.

#let display-url = (
  data.site_url.replace("https://", "").replace("http://", "").trim("/")
)

// Use a fresh page with side margins so the panel sits centred horizontally
// without inheriting the 0mm margin from the dense menu pages.
#set page(margin: (x: 18mm, y: 14mm))
#pagebreak()

#align(center + horizon)[
  #set text(fill: col-text)

  #text(size: 34pt, weight: "bold", fill: col-red)[Hier scannen,]
  #v(-4mm)
  #text(size: 34pt, weight: "bold", fill: col-red, style: "italic")[später bestellen]

  #v(6mm)

  #box(
    width: 80mm,
    height: 80mm,
    inset: 3mm,
    fill: white,
    stroke: 1pt + col-pink-2,
    radius: 3pt,
  )[
    #image("/img/qr.svg", width: 100%, height: 100%, fit: "contain")
  ]

  #v(4mm)
  #text(size: 20pt, weight: "bold")[#display-url]

  #v(2mm)
  #text(size: 10pt, fill: col-muted)[
    Online bestellen · Speisekarte als PDF · Lieferung & Abholung
  ]

  #v(8mm)
  #text(size: 9pt, fill: col-muted)[
    #data.branding.address · #data.branding.phone
  ]
]
