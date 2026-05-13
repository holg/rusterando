-- Voucher system phase 1.
--
-- vouchers: admin-created codes (anonymous, shared across customers, or
-- bound to a phone via per_phone_cap=1 + first_order_only). The shape
-- covers % off, fixed € off, and free delivery. The `kind` discriminator
-- decides which numeric field (`percent_off` or `amount_off_cents`) is
-- meaningful; for free_delivery both are NULL.
--
-- voucher_redemptions: append-only log; one row per successful use. Lets
-- us enforce per_phone_cap and global_cap with two cheap COUNT(*)s, and
-- gives the admin a redeem history without scanning orders.

CREATE TABLE vouchers (
    id TEXT PRIMARY KEY,

    -- The code customers type. Stored normalised (UPPER, no spaces).
    code TEXT UNIQUE NOT NULL,

    -- 'percent' | 'fixed' | 'free_delivery'
    kind TEXT NOT NULL,

    -- 0-100, only set when kind='percent'
    percent_off INTEGER,
    -- cents; only set when kind='fixed'
    amount_off_cents INTEGER,

    -- Minimum cart subtotal before this voucher applies. 0 = no floor.
    min_subtotal_cents INTEGER NOT NULL DEFAULT 0,

    -- First-order-only: when true, the redeem check requires
    -- the phone to have zero prior paid orders.
    first_order_only INTEGER NOT NULL DEFAULT 0,

    -- 0 = unlimited; otherwise this many redemptions per phone.
    per_phone_cap INTEGER NOT NULL DEFAULT 0,
    -- 0 = unlimited; otherwise this many total redemptions across all phones.
    global_cap INTEGER NOT NULL DEFAULT 0,

    -- ISO 8601 strings; both optional.
    valid_from TIMESTAMP,
    valid_until TIMESTAMP,

    -- Admin label shown in the admin list + on the customer's
    -- order confirmation. Optional.
    label TEXT,

    -- Soft-disable without losing history.
    active INTEGER NOT NULL DEFAULT 1,

    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_vouchers_code ON vouchers(code);
CREATE INDEX idx_vouchers_active ON vouchers(active);

CREATE TABLE voucher_redemptions (
    id TEXT PRIMARY KEY,
    voucher_id TEXT NOT NULL REFERENCES vouchers(id) ON DELETE CASCADE,
    order_id TEXT NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    -- Phone snapshot so per_phone_cap survives even if the
    -- customer row is later renamed/merged.
    phone TEXT NOT NULL,
    -- Cents actually discounted (signed positive). For free_delivery
    -- this equals the waived delivery fee.
    discount_cents INTEGER NOT NULL,
    redeemed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_vr_voucher ON voucher_redemptions(voucher_id);
CREATE INDEX idx_vr_phone   ON voucher_redemptions(phone);
CREATE INDEX idx_vr_order   ON voucher_redemptions(order_id);

-- Orders carry the resolved voucher snapshot so the receipt + admin
-- detail can show "Code WILLKOMMEN10 (-3.50 €)" without a join.
ALTER TABLE orders ADD COLUMN voucher_id TEXT REFERENCES vouchers(id);
ALTER TABLE orders ADD COLUMN voucher_code TEXT;
ALTER TABLE orders ADD COLUMN voucher_discount_cents INTEGER NOT NULL DEFAULT 0;
