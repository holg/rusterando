// Kurzliste / Einzelauflistung — A4 portrait. A compact, one-line-per-order
// statement modelled on the Lieferando "Einzelauflistung": a summary header
// (Gesamt / Bei Auslieferung bezahlt / Online bezahlt) followed by a dense
// two-column table of Datum · # · € with a "*" marking online-paid orders.
//
// Unlike the Beleg (one full page per order), this is a bookkeeping overview:
// no line items, no addresses — just the money per order so the whole period
// fits on as few pages as possible.
//
// Payload (JSON via `sys.inputs.data`): { shop, period, rows: [...] }.
//   period: { from, to, count_total, sum_total_cents,
//             count_cash, sum_cash_cents, count_online, sum_online_cents,
//             count_voucher, sum_voucher_cents,
//             count_cancelled, include_test, include_cancelled, note }
//   rows:   [ { date, number, total_cents, marker } ]
//           marker: "" cash · "*" online (paid) · "†" Gutschein (voucher)

#let data = json(bytes(sys.inputs.at("data")))
#let shop = data.shop
#let period = data.period
#let rows = data.rows

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

#set document(
  title: "Einzelauflistung " + period.from + " – " + period.to,
  author: shop.name,
)
#set page(
  paper: "a4",
  margin: (x: 20mm, y: 18mm),
  footer: context [
    #set text(size: 7.5pt, fill: rgb("#888"))
    #shop.name
    #if shop.vat_id != "" [ · USt-IdNr. #shop.vat_id ]
    #h(1fr)
    Seite #counter(page).display() / #context counter(page).final().first()
  ],
)
#set text(font: "Inter", size: 10pt, fill: rgb("#1a1a1a"))

// ---- Header: title + shop ----
#grid(
  columns: (1fr, auto),
  align: (left + top, right + top),
  [
    #text(size: 16pt, weight: "bold")[Einzelauflistung]
    #v(1mm)
    #text(size: 10pt, fill: rgb("#555"))[Restaurant: #shop.name]
    #if shop.address != "" [ \ #text(size: 9pt, fill: rgb("#777"))[#shop.address] ]
    \ #text(size: 10pt, fill: rgb("#555"))[
      Zeitraum: #period.from bis einschl. #period.to
    ]
  ],
  [],
)

#v(4mm)
#line(length: 100%, stroke: 0.6pt + rgb("#ccc"))
#v(3mm)

// ---- Summary block ----
// `mark` appends a legend symbol ("*" online, "†" Gutschein) after the label.
#let summary-row(label, count, sum, mark: none) = grid(
  columns: (auto, 1fr, auto),
  column-gutter: 6mm,
  align: (left, left, right),
  [#label#if mark != none [ #text(fill: rgb("#888"))[#mark]]],
  [#str(count) Bestellungen im Wert von],
  [#text(weight: "bold")[#eur(sum)]],
)

#summary-row("Gesamt", period.count_total, period.sum_total_cents)
#v(1.5mm)
#summary-row("Bei Auslieferung bezahlt", period.count_cash, period.sum_cash_cents)
#v(1.5mm)
#summary-row("Online bezahlt", period.count_online, period.sum_online_cents, mark: "*")
#if period.count_voucher > 0 [
  #v(1.5mm)
  #summary-row("Per Gutschein bezahlt", period.count_voucher, period.sum_voucher_cents, mark: "†")
]

#if period.include_cancelled and period.count_cancelled > 0 [
  #v(1.5mm)
  #grid(
    columns: (auto, 1fr, auto),
    column-gutter: 6mm,
    align: (left, left, right),
    text(fill: rgb("#a00"))[Storniert],
    text(fill: rgb("#a00"))[#str(period.count_cancelled) Bestellungen (nicht im Umsatz)],
    [],
  )
]

#v(3mm)
#line(length: 100%, stroke: 0.6pt + rgb("#ccc"))
#v(2mm)
#text(size: 8pt, fill: rgb("#888"))[
  \* Online im Voraus bezahlt (Karte, Apple Pay …).
  #if period.count_voucher > 0 [ † Vollständig per Gutschein bezahlt (0 €). ]
  #if not period.include_test [ Nur Live-Bestellungen. ]
]

#if period.note != "" [
  #v(1.5mm)
  #text(size: 8.5pt, fill: rgb("#a00"))[#period.note]
]

#v(5mm)

// ---- Two-column dense order table ----
// Split the rows into two halves so they print side by side, like the
// Lieferando statement. Each half is a 3-col table (Datum · # · €).
#let half-table(slice) = table(
  columns: (auto, auto, 1fr),
  align: (left, left, right),
  stroke: none,
  inset: (x: 3pt, y: 3.2pt),
  table.header(
    text(size: 8pt, weight: "bold", fill: rgb("#888"))[Datum],
    text(size: 8pt, weight: "bold", fill: rgb("#888"))[\#],
    text(size: 8pt, weight: "bold", fill: rgb("#888"))[€],
  ),
  table.hline(stroke: 0.6pt + rgb("#ccc")),
  ..slice.map(r => (
    text(size: 9pt)[#r.date],
    text(size: 9pt)[#r.number],
    text(size: 9pt)[#eur(r.total_cents)#if r.marker != "" [ #text(fill: rgb("#888"))[#r.marker]]],
  )).flatten()
)

#let n = rows.len()
#if n == 0 [
  #text(fill: rgb("#888"))[Keine Bestellungen im gewählten Zeitraum.]
] else {
  let mid = calc.ceil(n / 2)
  let left = rows.slice(0, mid)
  let right = if mid < n { rows.slice(mid, n) } else { () }
  grid(
    columns: (1fr, 1fr),
    column-gutter: 10mm,
    half-table(left),
    if right.len() > 0 { half-table(right) } else { [] },
  )
}
