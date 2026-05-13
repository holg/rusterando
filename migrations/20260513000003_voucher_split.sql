-- Split the voucher_discount_cents column into two parts so the
-- Buchhaltung report can subtract the items-only portion from item
-- revenue (and surface the delivery-fee waiver separately).
--
-- Without the split the report did:
--   Netto = Σ(item line totals) − Σ(voucher_discount_cents)
-- which goes negative whenever a voucher covered delivery fee too,
-- e.g. 25 € voucher on a 14 € cart + 3 € delivery = 17 € total
-- discount but only 14 € of that should reduce item revenue.
--
-- The total `voucher_discount_cents` stays for backward-compat with
-- the kitchen-protocol struct + printer rendering. New columns are
-- additive only; default 0 for historical rows (those are sandbox
-- test orders anyway, no real bookkeeping impact).

ALTER TABLE orders
    ADD COLUMN voucher_discount_items_cents    INTEGER NOT NULL DEFAULT 0;

ALTER TABLE orders
    ADD COLUMN voucher_discount_delivery_cents INTEGER NOT NULL DEFAULT 0;
