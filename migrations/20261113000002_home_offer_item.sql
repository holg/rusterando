-- Homepage offer cards linked to a menu item: the card's price and the
-- "mit N Zutaten nach Wahl" / "+ jede weitere Zutat X €" lines then come
-- live from the item (pages/home_content.rs), so they can't go stale. The
-- fixed price_label / add_on texts stay for cards without an item.
ALTER TABLE home_offers ADD COLUMN menu_item_id TEXT;

-- The existing cards: to the listed item of the same name (case, spaces
-- and × ignored), preferring one with a menu number.
UPDATE home_offers SET menu_item_id = (
    SELECT i.id FROM menu_items i JOIN menu_categories c ON c.id = i.category_id
    WHERE i.is_listed = 1 AND c.is_active = 1
      AND lower(replace(replace(i.name, ' ', ''), '×', 'x'))
        = lower(replace(replace(home_offers.title, ' ', ''), '×', 'x'))
    ORDER BY i.menu_number IS NULL, i.sort_order
    LIMIT 1
)
WHERE menu_item_id IS NULL;

-- A short text that only repeats the item's own "mit N Zutaten nach Wahl"
-- goes: the card shows that line live now.
UPDATE home_offers SET blurb = ''
WHERE menu_item_id IS NOT NULL AND lower(blurb) LIKE 'mit % zutaten nach wahl';
