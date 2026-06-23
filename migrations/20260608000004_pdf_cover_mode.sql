-- PDF front-cover mode: text vs image.
--
-- The leporello front cover was hardwired to a PHOTO (`/img/cover.jpg`), which
-- fell back to the bundled davidspizzeria cover when a tenant had no upload —
-- so every shop's PDF showed Davids' storefront and there was no way to get a
-- neutral, own-branded cover. This adds an admin-editable mode:
--   pdf_cover_mode = 'text'   typeset cover from the shop's branding
--                             (name / address / phone / hours). DEFAULT.
--   pdf_cover_mode = 'image'  the photo path (Cover-Bibliothek / bundled).
-- The template (menu.typ) branches on it via data.branding.cover_mode.
--
-- INSERT OR IGNORE so an admin who later picks 'image' is never clobbered on a
-- re-run. Default 'text' is the safe choice for a fresh tenant.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('pdf_cover_mode', 'text',
   'PDF-Titelseite',
   'Titelseite der Speisekarte (PDF): „text" = gesetzte Textseite aus den Shop-Daten (Name, Adresse, Telefon, Öffnungszeiten) — kein Foto nötig. „image" = Foto aus der Cover-Bibliothek.');

-- Reset the active PDF theme to the factory template. The stored theme rows
-- (pdf_themes) are COPIES of the old menu.typ that hardcoded the image cover,
-- so they'd override the updated code template and never show the text cover.
-- Clearing the active pointer makes rendering fall back to the bundled
-- TEMPLATE_SRC (the new menu.typ with the cover toggle). The pdf_themes rows
-- themselves are KEPT (never dropped) — an admin can re-activate a custom theme
-- later; this only clears the *pointer*. Only resets if the row exists.
UPDATE app_settings SET value = '' WHERE key = 'pdf_theme_active_id';
