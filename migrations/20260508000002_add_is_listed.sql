-- Add a separate "is_listed" flag for permanent removal of items from the
-- public menu. This is distinct from "is_available" (temporarily sold out).
--
--   is_listed   = 1 (default): item appears on /menu, /menu.pdf and admin
--   is_listed   = 0          : hidden from customers; still kept in the DB
--                              so historical orders & accounting resolve to it
--   is_available = 1         : item is in stock today
--   is_available = 0         : item shows greyed-out as "derzeit nicht verfügbar"

ALTER TABLE menu_items ADD COLUMN is_listed INTEGER NOT NULL DEFAULT 1;

CREATE INDEX idx_menu_items_listed ON menu_items(is_listed);
