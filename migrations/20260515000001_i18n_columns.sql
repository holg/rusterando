-- i18n: add per-locale sibling columns for every public-facing text
-- field that lives in the DB. The existing `name`/`description`/`label`
-- columns stay the canonical-DE source of truth (no rename, no
-- backfill). Each new column is NULL by default; the SSR loader picks
-- the right one via `COALESCE(name_<lang>, name)` based on the active
-- locale.
--
-- Why sibling columns vs a join table:
--   - SELECTs stay single-table. No N+1 risk on the menu list.
--   - Indexable per column if we ever need /search?q=… in a locale.
--   - Admin UI is a per-row form; one INSERT/UPDATE per item still.
--
-- 7 non-default locales × 5 tables × 1-2 text fields each = ~50 cols.
-- All nullable, no defaults. The build only carries these when
-- feature = "i18n" is on (the Rust SELECTs filter at runtime), so a
-- German-only deploy doesn't pay any query cost — just the bytes for
-- empty TEXT cells.

-- menu_categories: name only (no description column).
ALTER TABLE menu_categories ADD COLUMN name_en TEXT;
ALTER TABLE menu_categories ADD COLUMN name_fr TEXT;
ALTER TABLE menu_categories ADD COLUMN name_it TEXT;
ALTER TABLE menu_categories ADD COLUMN name_es TEXT;
ALTER TABLE menu_categories ADD COLUMN name_pt TEXT;
ALTER TABLE menu_categories ADD COLUMN name_ru TEXT;
ALTER TABLE menu_categories ADD COLUMN name_cn TEXT;

-- menu_items: name + description.
ALTER TABLE menu_items ADD COLUMN name_en TEXT;
ALTER TABLE menu_items ADD COLUMN name_fr TEXT;
ALTER TABLE menu_items ADD COLUMN name_it TEXT;
ALTER TABLE menu_items ADD COLUMN name_es TEXT;
ALTER TABLE menu_items ADD COLUMN name_pt TEXT;
ALTER TABLE menu_items ADD COLUMN name_ru TEXT;
ALTER TABLE menu_items ADD COLUMN name_cn TEXT;

ALTER TABLE menu_items ADD COLUMN description_en TEXT;
ALTER TABLE menu_items ADD COLUMN description_fr TEXT;
ALTER TABLE menu_items ADD COLUMN description_it TEXT;
ALTER TABLE menu_items ADD COLUMN description_es TEXT;
ALTER TABLE menu_items ADD COLUMN description_pt TEXT;
ALTER TABLE menu_items ADD COLUMN description_ru TEXT;
ALTER TABLE menu_items ADD COLUMN description_cn TEXT;

-- pizza_extras: label only.
ALTER TABLE pizza_extras ADD COLUMN label_en TEXT;
ALTER TABLE pizza_extras ADD COLUMN label_fr TEXT;
ALTER TABLE pizza_extras ADD COLUMN label_it TEXT;
ALTER TABLE pizza_extras ADD COLUMN label_es TEXT;
ALTER TABLE pizza_extras ADD COLUMN label_pt TEXT;
ALTER TABLE pizza_extras ADD COLUMN label_ru TEXT;
ALTER TABLE pizza_extras ADD COLUMN label_cn TEXT;

-- item_option_groups: label only.
ALTER TABLE item_option_groups ADD COLUMN label_en TEXT;
ALTER TABLE item_option_groups ADD COLUMN label_fr TEXT;
ALTER TABLE item_option_groups ADD COLUMN label_it TEXT;
ALTER TABLE item_option_groups ADD COLUMN label_es TEXT;
ALTER TABLE item_option_groups ADD COLUMN label_pt TEXT;
ALTER TABLE item_option_groups ADD COLUMN label_ru TEXT;
ALTER TABLE item_option_groups ADD COLUMN label_cn TEXT;

-- item_options: label only.
ALTER TABLE item_options ADD COLUMN label_en TEXT;
ALTER TABLE item_options ADD COLUMN label_fr TEXT;
ALTER TABLE item_options ADD COLUMN label_it TEXT;
ALTER TABLE item_options ADD COLUMN label_es TEXT;
ALTER TABLE item_options ADD COLUMN label_pt TEXT;
ALTER TABLE item_options ADD COLUMN label_ru TEXT;
ALTER TABLE item_options ADD COLUMN label_cn TEXT;
