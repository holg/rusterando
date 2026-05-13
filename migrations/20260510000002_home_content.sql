-- Editable home-page content. Today everything on / is hard-coded in
-- pages/home.rs; this migration moves it into the DB so David can edit
-- copy and photos via /admin/home without a deploy.
--
-- Seeds match the existing hard-coded content so the site renders the
-- same on first run after the migration applies.

-- Hero (top of /). Single row keyed by id=1 to keep the API trivial.
CREATE TABLE home_hero (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    title       TEXT NOT NULL,
    subtitle    TEXT NOT NULL,
    image_path  TEXT,
    updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT OR IGNORE INTO home_hero (id, title, subtitle, image_path)
VALUES (
    1,
    'Davids Pizzeria',
    'Frische Pizza, Pasta und Salate — direkt aus Lüdinghausen.',
    '/img/pizza-1.jpg'
);

-- Promo cards under the hero. `image_path` is optional — when null the
-- card falls back to the warm-yellow text-only style. `active=0` hides
-- the card without deleting (handy for seasonal pushes). `position`
-- gives a deterministic order without relying on insert time.
CREATE TABLE home_offers (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    price_label TEXT NOT NULL,
    blurb       TEXT NOT NULL,
    add_on      TEXT,
    image_path  TEXT,
    active      INTEGER NOT NULL DEFAULT 1,
    position    INTEGER NOT NULL DEFAULT 100,
    updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT INTO home_offers (id, title, price_label, blurb, add_on, position) VALUES
    ('seed-blech',  'Pizzablech 60×40 cm',          '28 €', 'mit 3 Zutaten nach Wahl',
                    '+ jede weitere Zutat 3 €', 10),
    ('seed-36cm',   'Pizza 36 cm mit Goudakäse',    '14 €', 'perfekt für die kleine Runde',
                    '+ jede weitere Zutat 1 €', 20);

-- Photo gallery. All three current photos are seeded; David can replace
-- them via /admin/home.
CREATE TABLE home_gallery (
    id          TEXT PRIMARY KEY,
    image_path  TEXT NOT NULL,
    caption     TEXT NOT NULL,
    position    INTEGER NOT NULL DEFAULT 100,
    updated_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT INTO home_gallery (id, image_path, caption, position) VALUES
    ('seed-laden', '/img/ladenfront.jpg', 'Unser Laden in Lüdinghausen',                         10),
    ('seed-theke', '/img/theke.jpg',      'Direkt von der Theke',                                20),
    ('seed-blech', '/img/pizzablech.jpg', 'Pizzablech 60×40 cm — perfekt für Familien & Büro',   30);
