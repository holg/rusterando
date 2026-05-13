CREATE TABLE menu_categories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1
);

-- Pizzas have two sizes (22 cm + 30 cm) with separate prices.
-- Non-pizza items use price_small_cents only and leave price_large_cents NULL.
CREATE TABLE menu_items (
    id TEXT PRIMARY KEY,
    category_id TEXT NOT NULL REFERENCES menu_categories(id),
    menu_number INTEGER UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    image_url TEXT,
    item_type TEXT NOT NULL,                    -- 'pizza','calzone','pasta','oven','meat','salad','drink','side'
    price_small_cents INTEGER NOT NULL,
    price_large_cents INTEGER,
    size_small_label TEXT,                      -- e.g. '22cm'
    size_large_label TEXT,                      -- e.g. '30cm'
    allergen_codes TEXT,                        -- CSV: 'a,c,g'
    additive_codes TEXT,                        -- CSV: '2,6,*'
    is_spicy INTEGER NOT NULL DEFAULT 0,
    is_available INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_menu_items_category ON menu_items(category_id);
CREATE INDEX idx_menu_items_number ON menu_items(menu_number);

CREATE TABLE menu_options (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES menu_items(id) ON DELETE CASCADE,
    group_name TEXT NOT NULL,                   -- 'extra','crust','sauce','topping'
    label TEXT NOT NULL,
    price_delta_cents INTEGER NOT NULL DEFAULT 0,
    is_default INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_menu_options_item ON menu_options(item_id);

-- EU 1169/2011 allergen legend (letters a..n)
CREATE TABLE allergen_legend (
    code TEXT PRIMARY KEY,
    name_de TEXT NOT NULL,
    name_en TEXT
);

-- Zusatzstoffe legend (numbers 1..9 plus * and **)
CREATE TABLE additive_legend (
    code TEXT PRIMARY KEY,
    name_de TEXT NOT NULL,
    name_en TEXT
);
