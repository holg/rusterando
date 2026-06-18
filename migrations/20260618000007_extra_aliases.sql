-- "aka" aliases per extra: alternate names a menu description may use for
-- an ingredient, across languages and inflections. The "ohne X" matcher
-- tries the extra's label PLUS each alias (whole-word). This replaces the
-- grammar-rule approach: explicit data per ingredient, locale-agnostic.
--   Pilze  -> Pilzen (de dative), Champignons (loanword / fr)
--   Käse   -> Goudakäse, Gouda, Mozzarella, ... (specific cheeses named)
CREATE TABLE IF NOT EXISTS extra_aliases (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    extra_id   TEXT NOT NULL REFERENCES pizza_extras(id) ON DELETE CASCADE,
    alias      TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_extra_aliases_extra ON extra_aliases(extra_id);

-- Seed sensible defaults for the existing catalog so common menus match
-- out of the box. Admin can add more in /admin/extras.
INSERT INTO extra_aliases (extra_id, alias, sort_order) VALUES
  ('ex-pilze',      'Pilzen',       10),
  ('ex-pilze',      'Champignons',  20),
  ('ex-extra-kaese','Goudakäse',    10),
  ('ex-extra-kaese','Gouda',        20),
  ('ex-extra-kaese','Mozzarella',   30),
  ('ex-zwiebeln',   'Zwiebel',      10),
  ('ex-tomaten',    'Tomate',       10),
  ('ex-oliven',     'Olive',        10),
  ('ex-artischocken','Artischocke', 10);

-- The previous grammar-rule mechanism is superseded. The
-- `ingredient_match_rules` table (migration ...000006) is left in place
-- (already applied) but is no longer read; it can be dropped in a later
-- cleanup migration once confirmed unused everywhere.
