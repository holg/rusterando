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
