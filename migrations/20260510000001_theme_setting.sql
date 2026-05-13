-- Site-wide theme switch. Two values today: 'warm' (cream + brand red,
-- the default look) and 'dark' (dark with red accents — David's mockup).
-- Editable from /admin/settings; SSR reads it from a cached AppState
-- handle so the very first byte of every response carries the right
-- <html class="theme-…"> class (no FOUC).

INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de)
VALUES (
    'theme',
    'warm',
    'Design-Thema',
    'warm = beige/rot (Standard) · dark = dunkel mit roten Akzenten'
);
