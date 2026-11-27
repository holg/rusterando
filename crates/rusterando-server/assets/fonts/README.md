# Bundled fonts

`NotoColorEmoji-subset.ttf` — Noto Color Emoji (Google, SIL Open Font
License 1.1), subset to the food/status glyphs a printed menu uses
(🌶 🌱 🔥 🧀 🍕 🥗 … ~50 code points). The full font is 10 MB; this is
108 KB, and it is parsed straight from the binary's bytes at first PDF
render (no copy).

Regenerate (fonttools):

```bash
pyftsubset NotoColorEmoji.ttf --output-file=NotoColorEmoji-subset.ttf \
  --unicodes="U+1F336,U+1F331,U+1F525,U+1F9C0,U+1F355,…" --no-hinting --layout-features='*'
```

Inter (Regular/Bold/Italic) is fetched by `build.rs` from the upstream
release zip into `OUT_DIR/fonts` at compile time.

## "Tafel" theme (templates/tafel.typ)

`CabinSketch-{Regular,Bold}-subset.ttf` (chalk lettering, category titles)
and `Kalam-{Regular,Bold}-subset.ttf` (chalk handwriting, everything
else) — both SIL Open Font License 1.1 (`CabinSketch-OFL.txt`,
`Kalam-OFL.txt`), from github.com/google/fonts (ofl/cabinsketch,
ofl/kalam). Subset to Latin, German punctuation and € (Kalam goes from
427 KB to 40 KB — the rest is Devanagari):

```bash
pyftsubset Kalam-Regular.ttf --output-file=Kalam-Regular-subset.ttf \
  --unicodes="U+0020-007E,U+00A0-00FF,U+0100-017F,U+2013-2014,U+2018-201E,U+2022,U+2026,U+20AC,U+00D7,U+2212" \
  --layout-features='kern,liga,calt' --no-hinting
```

## Montserrat ExtraBold (poster cover title)

The storefront-banner lettering on poster covers (a shop's poster layout,
see `templates/cover_live.typ`). SIL OFL 1.1 (`Montserrat-OFL.txt`), static TTF
from https://github.com/JulietaUla/Montserrat (tag v7.222), subset like the
chalk fonts:

```bash
pyftsubset Montserrat-ExtraBold.ttf \
  --unicodes="U+0020-007E,U+00A0-00FF,U+2013-2014,U+2018-201F,U+20AC" \
  --layout-features='kern,liga' --output-file=Montserrat-ExtraBold-subset.ttf
```
