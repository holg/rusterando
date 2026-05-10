-- Extrawünsche (per-item add-ons) and special formats from the print menu.
--
-- Print-menu rules (top of page 2):
--   Pizza extras:  Krabben +1,00 €, Lachs +2,00 €, Kräuterbutter/Sauce +0,60 €, sonstige Extras +0,70 €
--   Salat & Pasta: Krabben +1,00 €, Lachs +2,00 €, sonstige Extras +0,70 €
--
-- We attach four canonical option rows to every pizza/calzone/pasta/salad item.
-- "Sonstige Extras" is a single placeholder — the customer picks the actual extra
-- ingredient at checkout from a free-text field (validated against an allow-list
-- the kitchen maintains in admin).

-- ---------------------------------------------------------------------------
-- Per-pizza extras: Krabben, Lachs, Kräuterbutter/Sauce, Sonstige
-- ---------------------------------------------------------------------------
INSERT INTO menu_options (id, item_id, group_name, label, price_delta_cents, is_default, sort_order)
SELECT 'mo-' || mi.id || '-krabben',   mi.id, 'extra', 'Krabben',              100, 0, 1 FROM menu_items mi WHERE mi.item_type IN ('pizza','calzone')
UNION ALL
SELECT 'mo-' || mi.id || '-lachs',     mi.id, 'extra', 'Lachs',                200, 0, 2 FROM menu_items mi WHERE mi.item_type IN ('pizza','calzone')
UNION ALL
SELECT 'mo-' || mi.id || '-kbutter',   mi.id, 'extra', 'Kräuterbutter/Sauce',   60, 0, 3 FROM menu_items mi WHERE mi.item_type IN ('pizza','calzone')
UNION ALL
SELECT 'mo-' || mi.id || '-sonstige',  mi.id, 'extra', 'Sonstige Extras',       70, 0, 4 FROM menu_items mi WHERE mi.item_type IN ('pizza','calzone');

-- ---------------------------------------------------------------------------
-- Per-pasta/salad extras: Krabben, Lachs, Sonstige (no Kräuterbutter)
-- ---------------------------------------------------------------------------
INSERT INTO menu_options (id, item_id, group_name, label, price_delta_cents, is_default, sort_order)
SELECT 'mo-' || mi.id || '-krabben',  mi.id, 'extra', 'Krabben',         100, 0, 1 FROM menu_items mi WHERE mi.item_type IN ('pasta','oven','salad','meat')
UNION ALL
SELECT 'mo-' || mi.id || '-lachs',    mi.id, 'extra', 'Lachs',           200, 0, 2 FROM menu_items mi WHERE mi.item_type IN ('pasta','oven','salad','meat')
UNION ALL
SELECT 'mo-' || mi.id || '-sonstige', mi.id, 'extra', 'Sonstige Extras',  70, 0, 3 FROM menu_items mi WHERE mi.item_type IN ('pasta','oven','salad','meat');

-- ---------------------------------------------------------------------------
-- Salate: optional Joghurt / Knoblauch / Cocktail-Dressing / Essig-Öl (gratis)
-- The print menu says: "Unsere Salate können Sie auf Wunsch mit Joghurt, Knoblauch,
-- Cocktail-Dressing oder Essig-Öl genießen." — included, no extra charge.
-- ---------------------------------------------------------------------------
INSERT INTO menu_options (id, item_id, group_name, label, price_delta_cents, is_default, sort_order)
SELECT 'mo-' || mi.id || '-dressing-joghurt',   mi.id, 'sauce', 'Joghurt-Dressing',    0, 0, 10 FROM menu_items mi WHERE mi.item_type = 'salad'
UNION ALL
SELECT 'mo-' || mi.id || '-dressing-knoblauch', mi.id, 'sauce', 'Knoblauch-Dressing',  0, 0, 11 FROM menu_items mi WHERE mi.item_type = 'salad'
UNION ALL
SELECT 'mo-' || mi.id || '-dressing-cocktail',  mi.id, 'sauce', 'Cocktail-Dressing',   0, 0, 12 FROM menu_items mi WHERE mi.item_type = 'salad'
UNION ALL
SELECT 'mo-' || mi.id || '-dressing-essigoel',  mi.id, 'sauce', 'Essig-Öl',            0, 1, 13 FROM menu_items mi WHERE mi.item_type = 'salad';

-- ---------------------------------------------------------------------------
-- Special formats — listed prominently on page 1 outside the numbered catalog
-- ---------------------------------------------------------------------------

-- Pizzablech 60×40 cm: 28 € with 3 toppings of choice; +3 € per additional topping.
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-special-blech', 'cat-pizza-belaege', NULL,
 'Pizzablech 60×40 cm',
 '28 € mit 3 Zutaten nach Wahl, jede weitere Zutat 3 €',
 'pizza', 2800, NULL, '60x40cm', NULL, 'a,c,g', '2,6', 0, 999);

-- The "extra topping" rule modelled as a single repeatable option (+3,00 €).
INSERT INTO menu_options (id, item_id, group_name, label, price_delta_cents, is_default, sort_order) VALUES
('mo-blech-extra', 'mi-special-blech', 'topping', 'Weitere Zutat (jeweils)', 300, 0, 1);

-- Pizza 36 cm mit Goudakäse: 14 €, +1 € pro weiterer Zutat.
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-special-36cm', 'cat-pizza-kaese', NULL,
 'Pizza 36 cm mit Goudakäse',
 '14 € Grundpreis, jede weitere Zutat 1 €',
 'pizza', 1400, NULL, '36cm', NULL, 'a,c,g', '2,6', 0, 998);

INSERT INTO menu_options (id, item_id, group_name, label, price_delta_cents, is_default, sort_order) VALUES
('mo-36cm-extra', 'mi-special-36cm', 'topping', 'Weitere Zutat (jeweils)', 100, 0, 1);
