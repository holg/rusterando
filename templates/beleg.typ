// Order-Beleg(e) — A4 portrait. Renders EITHER a single order or a batch.
//
// A formal, plain invoice-style document a tax clerk (Steuerberater) can file:
// shop header + address + VAT-ID, order number + date, line items with
// quantity / name / variant / extras / unit price / line total, then the
// sums block (Zwischensumme, Lieferung, Gutschein, enthaltene MwSt., Gesamt)
// and the payment method.
//
// Payload (JSON via `sys.inputs.data`): { shop, summary?, orders: [...] }.
//   * single order  → one entry in `orders`, `summary` is none.
//   * batch (range) → a `summary` cover page first, then one full-page Beleg
//     per order (page break between them).
// This template only lays it out — the server computed every money value so
// the PDF matches every other invoice surface exactly.

#let data = json(bytes(sys.inputs.at("data")))
#let shop = data.shop
#let orders = data.orders
#let summary = data.at("summary", default: none)

// cents (int) -> "1.234,56 €" (German grouping + decimal comma).
#let eur(cents) = {
  let neg = cents < 0
  let c = calc.abs(cents)
  let euros = calc.div-euclid(c, 100)
  let rest = calc.rem-euclid(c, 100)
  let s = str(euros)
  let grouped = ""
  let n = s.len()
  for (i, ch) in s.clusters().enumerate() {
    if i > 0 and calc.rem(n - i, 3) == 0 { grouped += "." }
    grouped += ch
  }
  let cents-str = if rest < 10 { "0" + str(rest) } else { str(rest) }
  (if neg { "−" } else { "" }) + grouped + "," + cents-str + " €"
}

#let doc-title = if summary != none {
  "Belege " + summary.from + " – " + summary.to
} else if orders.len() > 0 {
  "Beleg " + orders.first().number
} else {
  "Belege"
}
#set document(title: doc-title, author: shop.name)
#set page(
  paper: "a4",
  margin: (x: 22mm, y: 20mm),
  footer: context [
    #set text(size: 7.5pt, fill: rgb("#888"))
    #shop.name
    #if shop.vat_id != "" [ · USt-IdNr. #shop.vat_id ]
    #h(1fr)
    Seite #counter(page).display() / #context counter(page).final().first()
  ],
)
#set text(font: "Inter", size: 10pt, fill: rgb("#1a1a1a"))

#let sum-row(label, value, bold: false) = grid(
  columns: (1fr, auto),
  column-gutter: 6mm,
  align: (left, right),
  if bold { text(weight: "bold")[#label] } else [#label],
  if bold { text(weight: "bold")[#value] } else [#value],
)

// ---- One order's full Beleg (used for single + each batch entry). ----
#let beleg(order) = {
  // Header: shop block (left) + Beleg meta (right).
  grid(
    columns: (1fr, auto),
    align: (left + top, right + top),
    [
      #text(size: 15pt, weight: "bold")[#shop.name]
      #if shop.address != "" [ \ #text(size: 9pt, fill: rgb("#555"))[#shop.address] ]
      #if shop.phone != "" [ \ #text(size: 9pt, fill: rgb("#555"))[#shop.phone] ]
      #if shop.email != "" [ \ #text(size: 9pt, fill: rgb("#555"))[#shop.email] ]
    ],
    [
      #text(size: 13pt, weight: "bold")[Beleg]
      \ #text(size: 9pt)[Nr. *#order.number*]
      \ #text(size: 9pt, fill: rgb("#555"))[Datum: #order.date]
      #if order.fulfillment != "" [ \ #text(size: 9pt, fill: rgb("#555"))[#order.channel_label: #order.fulfillment] ]
    ],
  )

  v(4mm)
  line(length: 100%, stroke: 0.6pt + rgb("#ccc"))
  v(2mm)

  // Customer / delivery block.
  grid(
    columns: (1fr, 1fr),
    column-gutter: 8mm,
    [
      #text(size: 8pt, weight: "bold", fill: rgb("#888"))[KUNDE]
      \ #order.customer_name
      #if order.customer_phone != "" [ \ #text(fill: rgb("#555"))[#order.customer_phone] ]
      #if order.customer_email != "" [ \ #text(fill: rgb("#555"))[#order.customer_email] ]
    ],
    if order.address != "" [
      #text(size: 8pt, weight: "bold", fill: rgb("#888"))[LIEFERADRESSE]
      \ #order.address
    ] else [],
  )

  v(4mm)

  // Line items.
  table(
    columns: (auto, 1fr, auto, auto),
    align: (right, left, right, right),
    stroke: none,
    inset: (x: 3pt, y: 5pt),
    table.header(
      [#text(size: 8pt, weight: "bold", fill: rgb("#888"))[MENGE]],
      [#text(size: 8pt, weight: "bold", fill: rgb("#888"))[ARTIKEL]],
      [#text(size: 8pt, weight: "bold", fill: rgb("#888"))[EINZEL]],
      [#text(size: 8pt, weight: "bold", fill: rgb("#888"))[GESAMT]],
    ),
    table.hline(stroke: 0.6pt + rgb("#ccc")),
    ..order.items.map(it => (
      [#it.quantity#sym.times],
      {
        let prefix = if it.menu_number != "" { it.menu_number + ". " } else { "" }
        let variant = if it.variant != "" { " (" + it.variant + ")" } else { "" }
        [#prefix#it.name#variant]
        let picks = if "selected_options" in it { it.selected_options } else { () }
        let mods = picks + it.extras.map(e => "+ " + e) + it.removals.map(r => "ohne " + r)
        if mods.len() > 0 {
          set text(size: 8.5pt, fill: rgb("#666"))
          set par(leading: 0.4em)
          linebreak()
          mods.join(linebreak())
        }
      },
      [#eur(it.unit_price_cents)],
      [#eur(it.line_total_cents)],
    )).flatten(),
  )

  v(2mm)
  line(length: 100%, stroke: 0.6pt + rgb("#ccc"))
  v(3mm)

  // Sums block (right-aligned).
  align(right, box(width: 70mm)[
    #sum-row("Zwischensumme", eur(order.subtotal_cents))
    #if order.delivery_fee_cents != 0 [ #v(1mm) #sum-row("Lieferung", eur(order.delivery_fee_cents)) ]
    #if order.voucher_discount_cents != 0 [
      #v(1mm)
      #sum-row(
        "Gutschein" + if order.voucher_code != "" { " (" + order.voucher_code + ")" } else { "" },
        "−" + eur(order.voucher_discount_cents),
      )
    ]
    #v(1.5mm)
    #line(length: 100%, stroke: 0.6pt + rgb("#ccc"))
    #v(1.5mm)
    #sum-row("Gesamt", eur(order.total_cents), bold: true)
    #v(1.5mm)
    #if order.tax_note != "" [
      #text(size: 8.5pt, fill: rgb("#666"))[#order.tax_note]
    ]
  ])

  v(6mm)
  text(size: 9pt)[*Zahlung:* #order.payment_label]

  if order.status_note != "" {
    v(2mm)
    text(size: 9pt, fill: rgb("#b00"))[#order.status_note]
  }
}

// ---- Batch summary cover page. ----
#let summary-page(s) = {
  text(size: 15pt, weight: "bold")[#shop.name]
  if shop.address != "" [ \ #text(size: 9pt, fill: rgb("#555"))[#shop.address] ]
  if shop.vat_id != "" [ \ #text(size: 9pt, fill: rgb("#555"))[USt-IdNr. #shop.vat_id] ]

  v(6mm)
  text(size: 18pt, weight: "bold")[Belegübersicht]
  v(1mm)
  text(size: 11pt, fill: rgb("#555"))[Zeitraum #s.from – #s.to]
  v(5mm)
  line(length: 100%, stroke: 0.6pt + rgb("#ccc"))
  v(4mm)

  box(width: 90mm)[
    #sum-row("Bestellungen", str(s.count))
    #v(1.5mm)
    #sum-row("Umsatz (ohne Storno)", eur(s.total_cents), bold: true)
    #if s.cancelled_count > 0 [
      #v(1.5mm)
      #sum-row("davon storniert (nicht im Umsatz)", str(s.cancelled_count))
    ]
  ]

  v(5mm)
  text(size: 8.5pt, fill: rgb("#888"))[
    #if s.include_test [ Enthält Sandbox-/Test-Bestellungen — nicht für den Steuerberater. \ ]
    #if s.include_cancelled [ Stornierte Bestellungen sind mit aufgeführt, zählen aber nicht zum Umsatz. \ ]
    Die folgenden Seiten enthalten je einen Beleg pro Bestellung.
  ]

  if s.truncated_note != "" {
    v(3mm)
    text(size: 9pt, fill: rgb("#b00"), weight: "bold")[#s.truncated_note]
  }
}

// ---- Document body. ----
#if summary != none {
  summary-page(summary)
}
#for (i, order) in orders.enumerate() {
  // Page-break before every order. In batch mode this also breaks after the
  // summary page; in single mode `i == 0` and no summary → no leading break.
  if i > 0 or summary != none { pagebreak() }
  beleg(order)
}
