-- Printer-theme selector for the kitchen receipt layout.
--
-- 'rusterando-default' = the established text-only layout.
-- 'rusterando-rando'   = adds a big bitmap logo at the very top of the
--                        receipt (a bag for pickup, a scooter/driver for
--                        delivery) so the kitchen sees the channel at a
--                        glance. The actual rendering lives on the Pi
--                        (rusterando-printer); the server just forwards
--                        the chosen theme name with each order.
--
-- Admin-editable via /admin/settings (dropdown, like 'theme'). INSERT OR
-- IGNORE so a re-run never clobbers an admin's choice. NEVER drop.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('printer_theme',
   'rusterando-default',
   'Drucker-Layout (Bon)',
   'Layout des Küchen-Bons. "rusterando-default" = klassisch (nur Text). "rusterando-rando" = mit grossem Symbol oben (Tüte = Abholung, Roller = Lieferung). Wirkt erst nach Drucker-Update auf dem Pi.');
