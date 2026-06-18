-- Ingredient-match rules: language declension as DATA, not code. The
-- "ohne X" matcher (models::match_removable_ingredients) tries the catalog
-- name plus, for each rule whose `when_suffix` the name ends with, the name
-- with that suffix replaced by `add_suffix`. So "Pilze" matches a German
-- dative "Pilzen" via a rule, not a hardcoded grammar branch. `locale`
-- scopes rules per language (empty = all locales). Admin-editable.
CREATE TABLE IF NOT EXISTS ingredient_match_rules (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    locale      TEXT NOT NULL DEFAULT '',
    when_suffix TEXT NOT NULL DEFAULT '',
    add_suffix  TEXT NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0
);

-- Seed reproduces the previous hardcoded German behaviour exactly:
--   "e"  -> "en"  (Pilze -> Pilzen)
--   "n"  -> ""    (Pilzen -> Pilze, defensive)
INSERT INTO ingredient_match_rules (locale, when_suffix, add_suffix, sort_order) VALUES
  ('de', 'e', 'en', 10),
  ('de', 'n', '',   20);
