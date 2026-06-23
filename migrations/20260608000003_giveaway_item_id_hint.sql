-- Refresh the giveaway-item hint: the admin may enter EITHER the internal
-- article id OR the menu number (what they see on the menu/receipt). The
-- `update_setting` validation resolves a menu number to the canonical id and
-- stores that. Needed because on imported/reseeded menus the id and number
-- diverge (Davids' `mi-400` ≈ number 400 is a coincidence; flizza's free
-- Pizzabrötchen is id `mc-004-i009`, number `99`).
--
-- UPDATE (not INSERT) so the label_de stays and only the hint changes on the
-- existing row. NEVER drop.
UPDATE app_settings
SET hint_de = 'Welches Produkt als Gratis-Beigabe hinzugefügt wird. Entweder die Artikel-ID '
              || '(z. B. mc-004-i009) ODER die Artikelnummer aus der Speisekarte (z. B. 99). '
              || 'Die Pflicht-Auswahlen des Artikels (z. B. Sauce) gelten automatisch.'
WHERE key = 'giveaway_item_id';
