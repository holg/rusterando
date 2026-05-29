-- Free-Pizzabrötchen giveaway (launch promo).
--
-- A qualifying online order can claim ONE free "Pizzabrötchen (6 Stück)"
-- (mi-400) with a customer-chosen sauce. The whole thing is admin-tunable so
-- it can be retired or re-tuned without a deploy:
--   giveaway_enabled         '0'/'1'      — master on/off
--   giveaway_min_order_cents cents        — qualifying subtotal (0 = always)
--   giveaway_order_types     enum         — both | pickup | delivery
-- These are normal app_settings rows, so they appear + edit automatically in
-- /admin/settings (the page lists every row generically). update_setting in
-- settings.rs adds a validation arm per key; settings_admin.rs adds a <select>
-- for the toggle + the order-type enum.
--
-- INSERT OR IGNORE so a re-run / an admin who later edits the value is never
-- clobbered (same convention as the google_site_verification seed). NEVER drop.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('giveaway_enabled', '0',
   'Gratis-Beigabe aktiv',
   'Gratis Pizzabrötchen als Beigabe zur Bestellung. An/Aus. Standard: Aus — im Admin einschalten, wenn die Aktion läuft.'),
  ('giveaway_min_order_cents', '2000',
   'Gratis-Beigabe ab Bestellwert',
   'Mindest-Zwischensumme (in Cent), ab der das gratis Pizzabrötchen angeboten wird. 0 = bei jeder Bestellung. Der Wert der Beigabe selbst zählt nicht mit.'),
  ('giveaway_order_types', 'both',
   'Gratis-Beigabe gültig für',
   'Für welche Bestellart die Beigabe gilt: both = Abholung + Lieferung, pickup = nur Abholung, delivery = nur Lieferung.');

-- The sauce choice. Reuses the existing required-choice option-group system
-- (item_option_groups / item_options / menu_item_option_groups). Fixed IDs so
-- the seed is idempotent and the order flow can reference the group when
-- offering the free item. Both options are 0 € — picking a sauce never adds
-- cost. min=1/max=1 makes the existing OptionGroupsPicker render a REQUIRED
-- radio (red asterisk) and the add-to-cart layer enforce exactly one pick.
-- The admin can rename/retire this group in /admin/options like any other.
INSERT OR IGNORE INTO item_option_groups (id, label, min_select, max_select, sort_order, is_active)
VALUES ('og-sauce-giveaway', 'Sauce', 1, 1, 1000, 1);

INSERT OR IGNORE INTO item_options (id, group_id, label, price_cents, sort_order, is_active) VALUES
  ('opt-sauce-knoblauch', 'og-sauce-giveaway', 'Knoblauchsauce', 0, 10, 1),
  ('opt-sauce-kraeuter',  'og-sauce-giveaway', 'Kräuterbutter',  0, 20, 1);

-- Attach the Sauce group to Pizzabrötchen (6 Stück) so a customer picks the
-- sauce when the free item is added.
INSERT OR IGNORE INTO menu_item_option_groups (menu_item_id, group_id, sort_order)
VALUES ('mi-400', 'og-sauce-giveaway', 0);

-- Mark giveaway lines explicitly rather than inferring "free" from a 0 € price
-- (a 0 € price could mean other things later). Nullable / default 0 keeps every
-- existing row valid. Snapshotted onto the order at placement.
ALTER TABLE cart_items  ADD COLUMN is_giveaway INTEGER NOT NULL DEFAULT 0;
ALTER TABLE order_items ADD COLUMN is_giveaway INTEGER NOT NULL DEFAULT 0;
