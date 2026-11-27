// The Tagesangebot for the 7.3" colour e-ink frame (Waveshare 7.3 inch
// e-Paper (E), Spectra 6): 800 × 480 points rendered at one pixel per
// point, drawn only in the panel's six colours — black, white, yellow, red,
// blue, green — so the server maps each pixel to a panel colour without
// dithering and the frame shows it crisp. The look is the website's Tafel
// theme: chalk on a (here black) board.
//
// Two marker colours stand for textures the server lays out of the six
// panel colours (pdf.rs `quantize_eink`): BOARD — a dark green slate (black
// with a fine green pattern, chalk dust), WOOD — the frame (red, yellow
// and black: brown). The writing in the panel colours gets a chalk grain
// there (some of its pixels left out).
//
// Input `data` (JSON): name, detail, price, regular (may be ""), motif (a
// /img/motif/<name>.svg drawing), shop, url, phone, and `offer` (false: no
// Tagesangebot today — the shop's card instead).

#let data = json(bytes(sys.inputs.data))

#let K = rgb(0, 0, 0)
#let W = rgb(255, 255, 255)
#let Y = rgb(255, 255, 0)
#let R = rgb(255, 0, 0)
#let G = rgb(0, 255, 0)
#let B = rgb(0, 0, 255)
#let BOARD = rgb(0, 40, 0)
#let WOOD = rgb(120, 60, 0)

// The wooden frame, the slate inside it.
#set page(width: 800pt, height: 480pt, margin: 0pt, fill: WOOD)
#place(top + left, dx: 20pt, dy: 20pt, rect(width: 760pt, height: 440pt, fill: BOARD, stroke: none, radius: 4pt))
#set text(font: ("Kalam", "Inter"), fill: W, size: 22pt)

#let chalk(size, body, fill: W) = text(font: ("Cabin Sketch", "Kalam"), weight: "bold", size: size, fill: fill, body)

// A chalk line just inside the frame, the corners a little uneven.
#place(top + left, dx: 34pt, dy: 34pt,
  rect(width: 732pt, height: 412pt, stroke: 2pt + W, radius: (top-left: 14pt, top-right: 6pt, bottom-right: 12pt, bottom-left: 8pt)))

// The item's drawing, big on the right, in white chalk lines.
#if data.at("motif", default: "") != "" {
  let svg = read("/img/motif/" + data.motif + ".svg")
    .replace("stroke=\"currentColor\"", "stroke=\"#ffffff\"")
    .replace("stroke-width=\"2\"", "stroke-width=\"1.4\"")
  place(top + right, dx: -62pt, dy: 66pt, rotate(-8deg, image(bytes(svg), format: "svg", width: 220pt)))
}

#let flag = box(width: 46pt, height: 7pt, fill: G) + box(width: 46pt, height: 7pt, fill: W) + box(width: 46pt, height: 7pt, fill: R)

#if data.at("offer", default: true) {
  // Tag, name, tricolour, detail: the left column.
  place(top + left, dx: 58pt, dy: 52pt, block(width: 460pt, {
    text(font: "Kalam", weight: "bold", size: 32pt, fill: R, tracking: 0.1em)[TAGESANGEBOT]
    v(-4pt)
    line(length: 236pt, stroke: 3pt + R)
    v(14pt)
    let n = data.name
    let size = if n.len() > 34 { 38pt } else if n.len() > 20 { 46pt } else { 56pt }
    par(leading: 0.35em, chalk(size, n))
    v(12pt)
    flag
    v(10pt)
    text(size: 24pt, data.detail)
  }))
  // The price, big in yellow chalk, the regular one struck through.
  place(bottom + left, dx: 58pt, dy: -84pt, {
    text(font: "Kalam", weight: "bold", size: 96pt, fill: Y, data.price)
    if data.at("regular", default: "") != "" {
      h(18pt)
      text(size: 34pt, [statt #strike(stroke: 2.5pt + W, data.regular)])
    }
  })
} else {
  place(top + left, dx: 58pt, dy: 66pt, block(width: 460pt, {
    chalk(64pt, data.shop)
    v(14pt)
    flag
    v(18pt)
    text(size: 30pt)[Heute frisch aus dem Ofen —]
    linebreak()
    text(size: 30pt, fill: Y)[jetzt online bestellen!]
  }))
}

// The shop, the web address and the phone at the bottom.
#place(bottom + left, dx: 58pt, dy: -44pt,
  text(size: 22pt)[#data.shop #h(10pt) · #h(10pt) #text(fill: Y, data.url) #h(10pt) · #h(10pt) #data.phone])
