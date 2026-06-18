-- "Remove ingredient" (ohne X): snapshot of left-off ingredients per cart/
-- order line, mirroring the existing extras_json column. JSON array of
-- CartRemoval { id, label } where label is the BARE ingredient name
-- ("Käse", never "Extra Käse"). Additive, nullable — old rows read as no
-- removals. See models::match_removable_ingredients.
ALTER TABLE cart_items  ADD COLUMN removals_json TEXT;
ALTER TABLE order_items ADD COLUMN removals_json TEXT;
