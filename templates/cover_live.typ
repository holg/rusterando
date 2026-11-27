// Live fields on poster covers.
//
// A poster cover is artwork from the Cover-Bibliothek with its prices and
// delivery fees left blank; a poster layout writes them in from the menu
// data, so the printed cover always shows what the shop charges today (and
// the staged prices in the "Neue Preise" preview).
//
// Each poster is known by the SHA-256 of its picture — the input
// `cover-sha`, which is also its Cover-Bibliothek file name. Its layout
// fits that artwork only; any other cover is left as it is, and so is the
// text cover. The text takes the theme's fonts, the colours are the
// artwork's own.
//
// The layouts are a shop's own, so they are not part of this tree: the
// server serves `$SITE_OVERLAY_DIR/posters.typ` as `/site_posters.typ`
// (an empty `#let posters = (:)` without an overlay). That module defines
//   #let posters = ("<sha256 of the picture>": (data, W, H) => content, …)
// and may `#import "/cover_live.typ": eur` for the price format.

#let eur(cents, space: "\u{00A0}") = {
  let e = calc.div-euclid(cents, 100)
  let c = calc.rem-euclid(cents, 100)
  let amount = if c == 0 { str(e) } else { str(e) + "," + (if c < 10 { "0" } else { "" }) + str(c) }
  amount + space + "€"
}

// The theme's cover panel, with the live fields on top when the active
// cover is a known poster (and the cover mode is the picture).
#let live-cover(panel, data, posters) = {
  let draw = posters.at(sys.inputs.at("cover-sha", default: ""), default: none)
  if draw == none or data.branding.at("cover_mode", default: "text") != "image" {
    return panel
  }
  box(width: 100%, height: 100%, {
    place(top + left, panel)
    layout(size => draw(data, size.width, size.height))
  })
}
