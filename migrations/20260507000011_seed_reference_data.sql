-- Reference data seeded from the printed menu.

-- ---------------------------------------------------------------------------
-- Allergen legend (EU Regulation 1169/2011)
-- ---------------------------------------------------------------------------
INSERT INTO allergen_legend (code, name_de, name_en) VALUES
    ('a', 'Gluten',           'Gluten'),
    ('b', 'Krebstiere',       'Crustaceans'),
    ('c', 'Eier',             'Eggs'),
    ('d', 'Fische',           'Fish'),
    ('e', 'Erdnüsse',         'Peanuts'),
    ('f', 'Sojabohnen',       'Soy'),
    ('g', 'Milch',            'Milk'),
    ('h', 'Schalenfrüchte',   'Tree nuts'),
    ('i', 'Sellerie',         'Celery'),
    ('j', 'Senf',             'Mustard'),
    ('k', 'Sesamsamen',       'Sesame'),
    ('l', 'Schwefeloxid & Sulfite', 'Sulphur dioxide and sulphites'),
    ('m', 'Lupinen',          'Lupin'),
    ('n', 'Weichtiere',       'Molluscs');

-- ---------------------------------------------------------------------------
-- Additive legend (Zusatzstoffe)
-- ---------------------------------------------------------------------------
INSERT INTO additive_legend (code, name_de, name_en) VALUES
    ('1', 'Formfleisch / Vorderschinken',  'Formed meat / shoulder ham'),
    ('2', 'Antioxidationsmittel',          'Antioxidants'),
    ('3', 'Stabilisator',                  'Stabilisers'),
    ('4', 'Geschmacksverstärker',          'Flavour enhancers'),
    ('5', 'Konservierungsstoffe',          'Preservatives'),
    ('6', 'Säurungsmittel',                'Acidulants'),
    ('7', 'Koffeinhaltig',                 'Caffeinated'),
    ('8', 'Süßungsmittel',                 'Sweeteners'),
    ('9', 'Farbstoffe',                    'Colourings'),
    ('*',  'Schinken',                     'Ham'),
    ('**', 'Salami',                       'Salami');

-- ---------------------------------------------------------------------------
-- Menu categories (in print-menu reading order)
-- ---------------------------------------------------------------------------
INSERT INTO menu_categories (id, name, sort_order, is_active) VALUES
    ('cat-pizza-kaese',       'Pizza Spezialitäten — mit versch. Käsesorten', 10, 1),
    ('cat-pizza-vegetarisch', 'Vegetarische Pizzen',                          20, 1),
    ('cat-pizza-calzone',     'Calzone Pizzen — gefüllte Pizzen',             30, 1),
    ('cat-pizza-fleisch',     'Pizza Spezialitäten — mit Fleischbelag',       40, 1),
    ('cat-pizza-fisch',       'Pizza Spezialitäten — mit Fischbelag',         50, 1),
    ('cat-pizza-belaege',     'Pizza Spezialitäten — mit verschiedenen Belägen', 60, 1),
    ('cat-pizza-tomate',      'Pizzen — mit frischen Tomaten und Mozzarella', 70, 1),
    ('cat-pizza-hollandaise', 'Pizzagerichte — mit Sauce Hollandaise',        80, 1),
    ('cat-pizzabroetchen',    'Pizzabrötchen',                                90, 1),
    ('cat-nudeln-pfanne',     'Nudelgerichte aus der Pfanne',                100, 1),
    ('cat-nudeln-backofen',   'Nudelgerichte aus dem Backofen',              110, 1),
    ('cat-broccoli',          'Broccoli Aufläufe',                           120, 1),
    ('cat-gourmet',           'Gourmet Pasta',                               130, 1),
    ('cat-puten',             'Putengeschnetzeltes',                         140, 1),
    ('cat-schnitzel',         'Schnitzel',                                   150, 1),
    ('cat-salat',             'Salatspezialitäten',                          160, 1),
    ('cat-rucola',            'Rucolasalat — Erfrischungs Salate',           170, 1),
    ('cat-getraenke',         'Getränke',                                    180, 1);

-- ---------------------------------------------------------------------------
-- Opening hours (weekday: 0 = Sunday .. 6 = Saturday)
-- The kitchen has a midday and evening shift on Mo/Di/Do/So — we store both as
-- separate rows so the schema stays simple and the UI can collapse them.
-- ---------------------------------------------------------------------------
INSERT INTO opening_hours (id, weekday, open_time, close_time, is_closed) VALUES
    ('oh-mo-mid', 1, '11:30', '14:30', 0),
    ('oh-mo-eve', 1, '17:00', '22:00', 0),
    ('oh-di-mid', 2, '11:30', '14:30', 0),
    ('oh-di-eve', 2, '17:00', '22:00', 0),
    ('oh-mi',     3, '00:00', '00:00', 1),    -- Mittwoch Ruhetag
    ('oh-do-mid', 4, '11:30', '14:30', 0),
    ('oh-do-eve', 4, '17:00', '22:00', 0),
    ('oh-fr',     5, '16:00', '22:00', 0),
    ('oh-sa',     6, '16:00', '22:00', 0),
    ('oh-so-mid', 0, '11:30', '14:30', 0),
    ('oh-so-eve', 0, '17:00', '22:00', 0);

-- ---------------------------------------------------------------------------
-- Delivery zones — three named zones, minimum order 15 € (1500 cents) for all.
-- Real postcodes per zone should be added once confirmed; the seed inserts the
-- three named zones using representative postcodes so the lookup compiles.
-- ---------------------------------------------------------------------------
INSERT INTO delivery_zones (id, postcode, fee_cents, min_order_cents, eta_minutes, is_active) VALUES
    ('dz-luedinghausen',  '59348', 150, 1500, 35, 1),  -- Lüdinghausen + 1,50 €
    ('dz-seppenrade',     '59348-seppenrade', 250, 1500, 45, 1),  -- Seppenrade + 2,50 €
    ('dz-bauerschaften',  '59348-bauerschaft', 250, 1500, 50, 1); -- Bauerschaften + 2,50 €
