-- Local address lookup: the municipality list becomes a per-tenant setting
-- (overrides the GEO_AGS / GEO_POSTCODE_MAP env defaults when non-empty), so
-- a shop can be moved to another town online and re-import from the
-- dashboard. Same string formats as the env keys.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('geo_ags', '', 'Gemeinden (AGS)',
   'Amtliche Gemeindeschlüssel, kommagetrennt (z. B. 05558024,05558028). Leer = Wert aus der .env. Pflegen unter Liefergebiete → Adressprüfung.'),
  ('geo_postcode_map', '', 'Gemeinden → PLZ',
   'AGS:PLZ:Ort, kommagetrennt (z. B. 05558024:59348:Lüdinghausen). Leer = Wert aus der .env.');
