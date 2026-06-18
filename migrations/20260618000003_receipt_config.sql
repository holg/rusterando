-- Bon-Editor: admin-editable receipt configuration. All additive, all
-- with defaults that reproduce the prior hardcoded behaviour, so existing
-- shops render unchanged until an admin edits them.
--
-- QR mode: 'order-url' encodes <PUBLIC_URL>/orders/<id> — the SAME id as
-- the live order channel, so a customer who scans the receipt lands on a
-- page that can receive the admin's live messages. 'static-url' encodes a
-- fixed URL (e.g. the shop homepage) from receipt_qr_static_url.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('receipt_qr_mode', 'order-url', 'Bon: QR-Code-Ziel',
   'Bestelldetail-URL (Kund:in kann Live-Nachrichten empfangen) oder eine feste URL.'),
  ('receipt_qr_static_url', '', 'Bon: feste QR-URL',
   'Nur bei „feste URL“: vollständige Adresse, z. B. https://davidspizzeria.de'),
  ('receipt_header_text', '', 'Bon: Kopfzeile',
   'Überschreibt den Shop-Namen oben auf dem Bon (leer = Shop-Name).'),
  ('receipt_footer_text', '', 'Bon: Fußzeile',
   'Freitext-Zeile unten auf JEDEM Bon (z. B. Dank/Aktion). Leer = nichts.'),
  ('receipt_show_allergens', '1', 'Bon: Allergen-Hinweis',
   'Den Allergen-Hinweisblock auf dem Bon anzeigen.'),
  ('receipt_show_logo', '1', 'Bon: Logo-Zeile',
   'Die Logo-Zeile (Kanal-Symbol + Marke) oben auf dem Bon anzeigen.');
