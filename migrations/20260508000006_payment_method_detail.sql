-- Track which payment instrument actually settled (card brand vs Apple Pay vs
-- Google Pay vs Link, etc.). NULL for cash-on-pickup orders. Used for the
-- invoice line ("Online bezahlt · Apple Pay") and to estimate Stripe fees per
-- method when reconciling against the monthly statement.

ALTER TABLE orders ADD COLUMN payment_method_detail TEXT;
