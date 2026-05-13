-- Phone-bound vouchers: an optional, normalised customer phone the
-- voucher is restricted to. NULL = anonymous code (anyone may redeem,
-- subject to the other rules). A non-NULL value is matched against
-- the customer's normalised phone in vouchers::ssr::redeem; mismatch
-- → hard rejection.

ALTER TABLE vouchers ADD COLUMN customer_phone TEXT;
CREATE INDEX idx_vouchers_customer_phone ON vouchers(customer_phone);
