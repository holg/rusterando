-- Convert menu_items.menu_number from INTEGER to TEXT so alphanumeric
-- catalog keys ("22a", "32a", "32b", ...) work.
--
-- SQLite has no ALTER COLUMN. The safe rebuild order with FK-bearing
-- children (cart_items, order_items, menu_options) is:
--   1. defer_foreign_keys ON  (checks happen at COMMIT)
--   2. rename the existing table out of the way
--   3. recreate `menu_items` with the new schema (FKs in children now
--      reference a table with the right name and the same row IDs after
--      the upcoming INSERT)
--   4. copy data, casting the integer column to TEXT
--   5. drop the renamed old table
-- All inside one transaction (sqlx wraps every migration in one), so the
-- COMMIT-time FK check sees fully consistent state.

-- legacy_alter_table=ON makes RENAME TO leave child FK references untouched
-- (otherwise SQLite "helpfully" rewrites every child's REFERENCES menu_items
-- into REFERENCES menu_items_old, which is exactly what we don't want).
PRAGMA legacy_alter_table = ON;
PRAGMA defer_foreign_keys = ON;

ALTER TABLE menu_items RENAME TO menu_items_old;

CREATE TABLE menu_items (
    id TEXT PRIMARY KEY,
    category_id TEXT NOT NULL REFERENCES menu_categories(id),
    menu_number TEXT UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    image_url TEXT,
    item_type TEXT NOT NULL,
    price_small_cents INTEGER NOT NULL,
    price_large_cents INTEGER,
    size_small_label TEXT,
    size_large_label TEXT,
    allergen_codes TEXT,
    additive_codes TEXT,
    is_spicy INTEGER NOT NULL DEFAULT 0,
    is_available INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO menu_items (
    id, category_id, menu_number, name, description, image_url, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, is_available, sort_order,
    created_at, updated_at
)
SELECT
    id, category_id,
    CASE WHEN menu_number IS NULL THEN NULL ELSE CAST(menu_number AS TEXT) END,
    name, description, image_url, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, is_available, sort_order,
    created_at, updated_at
FROM menu_items_old;

DROP TABLE menu_items_old;

CREATE INDEX idx_menu_items_category ON menu_items(category_id);
CREATE INDEX idx_menu_items_number ON menu_items(menu_number);

-- Restore the alphanumeric catalog keys that previously had to be NULL.
UPDATE menu_items SET menu_number = '22a' WHERE id = 'mi-022a' AND menu_number IS NULL;
UPDATE menu_items SET menu_number = '32a' WHERE id = 'mi-032a' AND menu_number IS NULL;

-- Names previously carried the suffix in parentheses; menu_number now does.
UPDATE menu_items SET name = 'Pizza Lüdinghausen' WHERE id = 'mi-022a';
UPDATE menu_items SET name = 'Pizza Sucuk' WHERE id = 'mi-032a';
