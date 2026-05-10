-- Per-item flat-price model for extras.
--
-- Background: most menu items use the global pizza_extras catalog
-- (Krabben €1, Lachs €2, sonstige €0.70). But some items have a
-- different deal: the Pizzablech 60×40 cm gets 3 extras included
-- and any further extras at a flat €3 each; the Pizza 36 cm has no
-- includeds but charges a flat €1 each. Until now this was only
-- documented in marketing copy; the cart actually charged the
-- catalog prices. This migration adds the data shape so add_to_cart
-- can honour it.
--
-- Columns added:
--   included_extras_count   how many extras are free on this item.
--                           0 means "no freebies, charge everything".
--   flat_extra_price_cents  per-extra flat charge in cents. NULL means
--                           "fall back to pizza_extras catalog prices"
--                           (existing behaviour for normal pizzas).
--
-- Selection-order: when included_extras_count > 0, the FIRST N extras
-- the customer ticks go free (€0); from the (N+1)th onwards each
-- costs flat_extra_price_cents. Cheapest-first would be friendlier
-- but harder for the customer to predict; selection-order matches
-- what they see ticked in the picker.

ALTER TABLE menu_items
    ADD COLUMN included_extras_count INTEGER NOT NULL DEFAULT 0;

ALTER TABLE menu_items
    ADD COLUMN flat_extra_price_cents INTEGER;

-- Apply the rule to Davids' two seeded items. Both lookups are
-- idempotent: re-running this migration on a fresh DB is fine, and
-- editing the values later via /admin/menu won't be clobbered.

-- Pizzablech 60×40 cm: 3 included, flat €3 thereafter.
UPDATE menu_items
SET included_extras_count   = 3,
    flat_extra_price_cents  = 300
WHERE name LIKE '%Pizzablech%' OR name LIKE '%60%40%';

-- Pizza 36 cm: 0 included, flat €1 each.
UPDATE menu_items
SET included_extras_count   = 0,
    flat_extra_price_cents  = 100
WHERE name LIKE '%Pizza%36%';
