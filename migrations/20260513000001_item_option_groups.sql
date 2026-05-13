-- Option groups: required-choice extras (e.g. salad dressing).
--
-- pizza_extras stays as the free-form additive multi-pick model
-- (you can tick 5 extras on a pizza). Some items also need
-- mandatory single-choice picks: a salad must come with exactly one
-- dressing. We model both with the same item_options system but
-- the per-group min_select / max_select bounds enforce the
-- "exactly one of N" rule.
--
-- Composition with pizza_extras: option_groups are layered ON TOP
-- of (not instead of) pizza_extras. A salad item can have both:
-- ticked extras (Tabasco, extra cheese) AND a chosen dressing.
-- Cart line snapshots them separately so the kitchen receipt can
-- print "1× Caesar-Salat + Joghurt-Dressing, extra Käse".

CREATE TABLE item_option_groups (
    id          TEXT PRIMARY KEY,
    label       TEXT NOT NULL,            -- shown to customer: "Dressing"
    -- min/max bounds on how many options a customer must pick from
    -- this group. The classic single-required-choice case is
    -- min=1, max=1. A "pick up to 2 sides" group would be min=0,
    -- max=2. min > max is rejected at the admin layer.
    min_select  INTEGER NOT NULL DEFAULT 1,
    max_select  INTEGER NOT NULL DEFAULT 1,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    is_active   INTEGER NOT NULL DEFAULT 1,
    created_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_option_groups_active ON item_option_groups(is_active, sort_order);

CREATE TABLE item_options (
    id          TEXT PRIMARY KEY,
    group_id    TEXT NOT NULL REFERENCES item_option_groups(id) ON DELETE CASCADE,
    label       TEXT NOT NULL,            -- "Joghurt-Dressing"
    price_cents INTEGER NOT NULL DEFAULT 0,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    is_active   INTEGER NOT NULL DEFAULT 1,
    created_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_item_options_group  ON item_options(group_id, sort_order);
CREATE INDEX idx_item_options_active ON item_options(is_active);

-- Join table: which option-groups apply to which menu items.
-- A single group ("Dressing") attaches to many items (every salad
-- on the menu shares the same dressing list — the admin defines it
-- once, then ticks the box on each salad item to attach it).
CREATE TABLE menu_item_option_groups (
    menu_item_id  TEXT NOT NULL REFERENCES menu_items(id) ON DELETE CASCADE,
    group_id      TEXT NOT NULL REFERENCES item_option_groups(id) ON DELETE CASCADE,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (menu_item_id, group_id)
);

CREATE INDEX idx_miog_item  ON menu_item_option_groups(menu_item_id, sort_order);
CREATE INDEX idx_miog_group ON menu_item_option_groups(group_id);

-- Cart + order snapshot: chosen options per line. Same shape as
-- the existing extras_json column on cart_items/order_items but
-- for options. Stored as a JSON array of CartSelectedOption objects
-- (group_id, group_label, option_id, option_label, price_cents) so
-- a later admin label change doesn't rewrite history.
ALTER TABLE cart_items  ADD COLUMN selected_options_json TEXT;
ALTER TABLE order_items ADD COLUMN selected_options_json TEXT;
