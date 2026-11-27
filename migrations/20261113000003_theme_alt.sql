-- A second site theme visitors can switch to with the sun/moon button in
-- the header (e.g. "warm" with "tafel"). Empty = no switcher.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('theme_alt', '', 'Umschalt-Theme (Hell/Dunkel)',
   'Das zweite Thema, zu dem Besucher mit dem Sonne/Mond-Knopf oben wechseln können (z. B. warm ↔ tafel). Leer = kein Umschalter. Die Wahl merkt sich der Browser.');
