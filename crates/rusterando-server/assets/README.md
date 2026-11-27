# PDF assets

- `cover.jpg`, `ladenfront.jpg` — the default cover and shop-front photo.
- `tafel.jpg` — the "Tafel" theme's blackboard (466×232 mm at 200 dpi: the
  largest page, the A5 Zickzack with crop marks; smaller pages crop it).
  Generated, no third-party image — slate, wiped-chalk swirls, dust:

  ```bash
  python3 -I scripts/gen_blackboard.py crates/rusterando-server/assets/tafel.jpg 466 232 200 11
  ```

- `fonts/` — see fonts/README.md.
- `motifs/` — line drawings the "Tafel" theme puts big and faint behind each
  category card (cheese for the cheese pizzas, fish for fish, …), served to
  Typst as `/img/motif/<name>.svg`. From Tabler Icons 3.51.0 (outline set),
  MIT licence, © Paweł Kuna — see `motifs/LICENSE`. Unchanged files; the
  theme recolours them when it reads them. To add one, drop the SVG from
  <https://tabler.io/icons> in here and add its name to `MOTIFS` in
  `src/pdf.rs`.
- `trattoria.jpg` — the "Trattoria" theme's dark paper (466×232 mm at 170
  dpi). Generated, no third-party image:

  ```bash
  python3 -I scripts/gen_paper.py crates/rusterando-server/assets/trattoria.jpg 466 232 170 5
  ```

- Poster covers (a shop's own artwork with empty price/delivery slots) are
  not part of this tree: `templates/cover_live.typ` fills them from the menu
  data through the shop's `$SITE_OVERLAY_DIR/posters.typ`. Upload the
  artwork in the PDF-Editor's Cover-Bibliothek and activate it.
