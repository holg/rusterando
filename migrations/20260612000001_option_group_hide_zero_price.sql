-- Per-group flag: suppress the "gratis" / 0 € price label on options.
--
-- Most option groups DO want "gratis" shown — a free side, a no-charge
-- dressing. But a pure *variant* picker (e.g. Durstlöscher flavour:
-- Mango / Zitrone / Wassermelone) has every option at 0 € because the
-- price lives in the drink, not the flavour. Showing "gratis" next to
-- each flavour wrongly implies the drink is free.
--
-- When `hide_zero_price = 1`, the customer-facing picker renders no price
-- label for 0 € options in that group (priced options still show their
-- "+X,XX €"). Default 0 = existing behaviour (show "gratis").
--
-- Additive + nullable-free: NOT NULL DEFAULT 0 keeps every existing row
-- valid without a backfill.
ALTER TABLE item_option_groups
    ADD COLUMN hide_zero_price INTEGER NOT NULL DEFAULT 0;
