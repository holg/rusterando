-- The poster cover's own texts, editable in the PDF-Editor (Text-Inhalte):
-- the title lines (storefront lettering) and the slogan under them. Empty
-- = what the poster had. Opening hours, address and phone on the poster
-- come from pdf_hours and the shop settings (templates/cover_live.typ).
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('pdf_cover_title', '', 'Poster-Titel (PDF-Cover)',
   'Eine Zeile pro Titelzeile, z. B. DAVID’S und PIZZERIA. Leer = DAVID’S / PIZZERIA.'),
  ('pdf_cover_slogan', '', 'Poster-Slogan (PDF-Cover)',
   'Die Zeile unter dem Titel. Leer = „Autentica. Fresca. Fatta con amore.“');
