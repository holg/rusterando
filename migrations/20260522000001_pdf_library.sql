-- PDF cover-image library + Typst theme library.
--
-- Before this migration the admin /admin/pdf page exposed two single-
-- value overrides:
--   * `pdf_cover_image`     — a single uploaded JPEG path
--   * `pdf_template_source` — a single inline Typst source string
--
-- We promote both to full libraries (CRUD via /admin/pdf) so the
-- admin can keep multiple covers + multiple .typ themes around and
-- flip between them without losing the previous one. Each library
-- has an "active-pointer" row in app_settings; the renderer reads
-- the active row at render time.
--
-- Migration is idempotent. Existing single-value settings are
-- imported into the new tables on first run; subsequent runs
-- short-circuit because the libraries are no longer empty.

CREATE TABLE IF NOT EXISTS pdf_cover_images (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    label       TEXT NOT NULL,
    filename    TEXT NOT NULL,         -- bare name; resolved under data/uploads/covers/
    mime_type   TEXT NOT NULL,         -- "image/jpeg" | "image/png"
    size_bytes  INTEGER NOT NULL DEFAULT 0,
    created_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS pdf_themes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    source      TEXT NOT NULL,
    created_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Active-pointer settings. Values stored as TEXT (the existing
-- app_settings.value column type); parsed to i64 at read time.
-- Empty string == nothing active → renderer falls back to the
-- compiled-in factory defaults (TEMPLATE_SRC / ASSET_COVER).
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('pdf_cover_active_id', '', 'Aktives PDF-Cover',
   'ID des aktiven Cover-Bilds aus der Bibliothek. Leer = Werks-Bild.'),
  ('pdf_theme_active_id', '', 'Aktive PDF-Vorlage',
   'ID der aktiven Typst-Vorlage aus der Bibliothek. Leer = Werks-Vorlage.');

-- One-time import of the legacy pdf_cover_image setting.
-- Guard: legacy setting is non-empty AND library is still empty.
INSERT INTO pdf_cover_images (label, filename, mime_type, size_bytes)
SELECT
    'Importiertes Cover',
    REPLACE(value, '/img/uploads/', ''),
    CASE
        WHEN LOWER(value) LIKE '%.png' THEN 'image/png'
        WHEN LOWER(value) LIKE '%.webp' THEN 'image/webp'
        ELSE 'image/jpeg'
    END,
    0
FROM app_settings
WHERE key = 'pdf_cover_image'
  AND TRIM(value) != ''
  AND value LIKE '/img/uploads/%'
  AND NOT EXISTS (SELECT 1 FROM pdf_cover_images);

-- If we just imported, activate it.
UPDATE app_settings
SET value = (SELECT id FROM pdf_cover_images ORDER BY id LIMIT 1)
WHERE key = 'pdf_cover_active_id'
  AND TRIM(value) = ''
  AND EXISTS (SELECT 1 FROM pdf_cover_images);

-- One-time import of the legacy pdf_template_source setting.
INSERT INTO pdf_themes (name, source)
SELECT 'Aktive Vorlage', value
FROM app_settings
WHERE key = 'pdf_template_source'
  AND TRIM(value) != ''
  AND NOT EXISTS (SELECT 1 FROM pdf_themes);

UPDATE app_settings
SET value = (SELECT id FROM pdf_themes ORDER BY id LIMIT 1)
WHERE key = 'pdf_theme_active_id'
  AND TRIM(value) = ''
  AND EXISTS (SELECT 1 FROM pdf_themes);

-- Legacy settings rows intentionally kept (NOT deleted) so a future
-- rollback can re-read them. A follow-up migration can prune them
-- once the new feature has stabilised in production.
