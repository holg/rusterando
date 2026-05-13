-- /admin/pdf editor: seed app_settings rows for the menu PDF's
-- variable parts. Lists are stored as newline-separated TEXT so an
-- admin can edit them in a plain <textarea> (one line per entry).
-- Image fields hold the public /img/uploads/<hash>.<ext> path
-- returned by the existing /api/admin/upload_image endpoint.
-- pdf_template_source is the full Typst source; empty falls back
-- to the binary-embedded template (see pdf.rs::TEMPLATE_SRC).

INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('pdf_tagline',            '',          'PDF Untertitel',           'Wird unter dem Shop-Namen auf der PDF angezeigt. Leer = kein Untertitel.'),
  ('pdf_hours',
    'Mo, Di, Do, So: 11:30–14:30 · 17:00–22:00' || char(10) ||
    'Fr, Sa: 16:00–22:00' || char(10) ||
    'Mi Ruhetag',
    'Öffnungszeiten (PDF)',     'Eine Zeile pro Eintrag.'),
  ('pdf_extras_pizza',
    'Krabben 1 €' || char(10) ||
    'Lachs 2 €' || char(10) ||
    'Kräuterbutter 0,60 €' || char(10) ||
    'sonstige Extras 0,70 €',
    'Pizza-Extras (PDF)',       'Eine Zeile pro Extra-Eintrag.'),
  ('pdf_extras_pasta',
    'Krabben 1 €' || char(10) ||
    'Lachs 2 €' || char(10) ||
    'sonstige Extras 0,70 €',
    'Pasta-Extras (PDF)',       'Eine Zeile pro Extra-Eintrag.'),
  ('pdf_cover_image',        '',          'Frontseiten-Bild (PDF)',   'URL/Pfad eines hochgeladenen Bildes. Leer = mitgeliefertes Werks-Bild.'),
  ('pdf_ad_cover_image',     '',          'Anzeige Seite 1 (PDF)',    'Optionales Anzeigenbild auf der Frontseite.'),
  ('pdf_ad_center_image',    '',          'Anzeige Mittelseite (PDF)','Optionales Anzeigenbild zwischen Speisekarte und Beinen.'),
  ('pdf_ad_back_image',      '',          'Anzeige Rückseite (PDF)',  'Optionales Anzeigenbild auf der Rückseite.'),
  ('pdf_template_source',    '',          'Typst-Template (PDF)',     'Voller Typst-Source. Leer = mitgeliefertes Standard-Template. Vor jedem Speichern wird ein Test-Render durchgeführt — fehlerhafter Code wird abgelehnt.');
