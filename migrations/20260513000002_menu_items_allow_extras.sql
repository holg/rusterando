-- Per-item opt-out for the global extras list (pizza_extras).
--
-- Default is "extras allowed" so existing pizza items keep working
-- without admin intervention. Items that shouldn't show extras at
-- all (e.g. Insalata Mista, drinks) get this flag flipped to 0
-- via /admin/menu. The public checkout-modal hides the extras
-- block entirely when allow_extras = 0.

ALTER TABLE menu_items ADD COLUMN allow_extras INTEGER NOT NULL DEFAULT 1;
