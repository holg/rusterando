-- Homepage: the printed menu's front cover beside the hero title. The server
-- writes the picture together with the menu's static PDF cache, only for a
-- poster cover with live prices (templates/cover_live.typ), and names it in
-- `home_cover_file` (written by the server, not listed in the settings).
-- This switch lets a shop hide it again.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('home_show_cover', '1', 'Titelblatt auf der Startseite',
   'An: das Titelblatt der PDF-Speisekarte (mit den aktuellen Preisen) steht oben auf der Startseite neben dem Titel. Nur bei einem Poster-Titelblatt mit Live-Preisen.');
