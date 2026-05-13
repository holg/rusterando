-- Snapshot the menu number ("22a", "51", "1bcd") onto order_items so the
-- kitchen ticket and the customer's invoice can read "22a Pizza Sucuk"
-- instead of just "Pizza Sucuk". Snapshotted (not joined live) so renumbering
-- a menu item later doesn't rewrite history.
--
-- Backfill from the current menu_items row; for legacy rows whose menu_item
-- has already been re-keyed, the column stays NULL and the UI just renders
-- the name without a leading number.

ALTER TABLE order_items ADD COLUMN menu_number_snapshot TEXT;

UPDATE order_items
   SET menu_number_snapshot = (
       SELECT menu_number FROM menu_items WHERE menu_items.id = order_items.menu_item_id
   )
 WHERE menu_number_snapshot IS NULL;
