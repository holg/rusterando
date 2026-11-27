// Example site poster layouts (see site/README.md, templates/cover_live.typ).
//
// Each entry: the SHA-256 of the cover picture in the Cover-Bibliothek
// (its file name there) → a function (data, W, H) that draws the live
// fields onto the panel, W × H being the cover panel's size.

#import "/cover_live.typ": eur

#let example-poster(data, W, H) = {
  // The cheapest item of the first category, bottom left of the artwork.
  let items = data.categories.at(0, default: (items: ())).items
  if items.len() == 0 { return }
  let item = items.sorted(key: it => it.price_small_cents).first()
  place(bottom + left, dx: 0.08 * W, dy: -0.08 * H,
    text(size: 0.05 * H, weight: "bold", fill: white)[#item.name — #eur(item.price_small_cents)])
}

#let posters = (
  // "<sha256 of the cover picture>": example-poster,
)
