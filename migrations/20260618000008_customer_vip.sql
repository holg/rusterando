-- VIP customers may chat back on their order page. When a customer's
-- `vip` flag is set, the public `/orders/{id}` page shows a reply box (the
-- order id stays the only secret — same trust model as the page itself).
-- Non-VIP customers see the message log read-only, as before.
ALTER TABLE customers ADD COLUMN vip INTEGER NOT NULL DEFAULT 0;
