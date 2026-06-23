-- Make the giveaway's free article ID admin-editable.
--
-- Until now the free item was the compile-time constant `mi-400`
-- (Pizzabrötchen) in cart.rs. This adds a normal app_settings row so the
-- product can be re-pointed in /admin/settings without a deploy (e.g. swap the
-- promo item, or fix it after a menu reseed changed the ID). The page lists
-- every row generically, so it appears + edits automatically; update_setting
-- in settings.rs gains a validation arm that rejects an ID with no matching
-- menu_item.
--
-- Default `mi-400` preserves today's behaviour exactly. The item's own required
-- option groups (e.g. the Sauce choice attached to mi-400) apply automatically
-- via the existing claim flow — so re-pointing to an item without a sauce group
-- simply offers it with no extra choice.
--
-- INSERT OR IGNORE so an admin who later edits the value is never clobbered on
-- a re-run (same convention as the original giveaway seed). NEVER drop.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('giveaway_item_id', 'mi-400',
   'Gratis-Artikel',
   'Welches Produkt als Gratis-Beigabe hinzugefügt wird (Artikel-ID, z. B. mi-400 = Pizzabrötchen). Die Pflicht-Auswahlen des Artikels (z. B. Sauce) gelten automatisch.');
