-- Menu items transcribed from David's Pizzeria print menu (re-opening 01.09.2025).
--
-- Conventions
--   * Pizzas have two prices: small = 22 cm, large = 30 cm.
--   * Non-pizza items put their single price in price_small_cents and leave
--     price_large_cents NULL.
--   * Money in cents (integer). 4,00 € → 400, 7,00 € → 700.
--   * Allergen codes are CSV from allergen_legend (a..n).
--     Additive codes are CSV from additive_legend (1..9, *, **).
--
-- Print-menu typos corrected (verify with kitchen):
--   #27 Frutti di Mare 30cm: printed "950" → 9,50 € assumed (matches column).
--   #28 al Salmone 30cm:    printed "1060" → 10,60 € assumed.
--   #54 Mamma Mia 30cm:     printed "9,50" kept as 9,50 €.
--   #44–#65: 22cm prices use the printed values; gaps in numbering (e.g. #50, #62) are intentional.

-- ---------------------------------------------------------------------------
-- Pizza Spezialitäten — mit versch. Käsesorten
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-001','cat-pizza-kaese',  1,'Pizza Margherita',   'mit Tomaten und Goudakäse',                    'pizza', 400, 700,'22cm','30cm','a,c,g','2,6',0, 1),
('mi-002','cat-pizza-kaese',  2,'Pizza Gorgonzola',   'mit Tomaten und Gorgonzola',                   'pizza', 500, 820,'22cm','30cm','a,c,g','2,6',0, 2),
('mi-003','cat-pizza-kaese',  3,'Pizza Mozzarella',   'mit Tomaten und Mozzarella',                   'pizza', 500, 840,'22cm','30cm','a,c,g','2,6',0, 3),
('mi-004','cat-pizza-kaese',  4,'Pizza Schafskäse',   'mit Tomaten und Schafskäse',                   'pizza', 500, 850,'22cm','30cm','a,c,g','2,6',0, 4),
('mi-005','cat-pizza-kaese',  5,'Quattro Formaggi',   'mit 4 verschiedenen Käsesorten',                'pizza', 500, 940,'22cm','30cm','a,c,g','2,6',0, 5);

-- ---------------------------------------------------------------------------
-- Vegetarische Pizzen
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-006','cat-pizza-vegetarisch',  6,'Pizza Cipolla',         'mit Zwiebeln',                                                            'pizza', 520, 850,'22cm','30cm','a,c,g','2,6',0,  6),
('mi-007','cat-pizza-vegetarisch',  7,'Pizza Funghi',          'mit Pilzen',                                                              'pizza', 520, 850,'22cm','30cm','a,c,g','2,6',0,  7),
('mi-008','cat-pizza-vegetarisch',  8,'Pizza Paprika',         'mit Paprika',                                                             'pizza', 520, 850,'22cm','30cm','a,c,g','2,6',0,  8),
('mi-009','cat-pizza-vegetarisch',  9,'Pizza Siciliana',       'mit Paprika, Zwiebeln, Artischocken, Peperoni und Knoblauch',             'pizza', 600, 940,'22cm','30cm','a,c,g','2,6',0,  9),
('mi-010','cat-pizza-vegetarisch', 10,'Pizza Vesuvio',         'mit Paprika, Artischocken, Pilzen, Oliven und Ei',                        'pizza', 600, 950,'22cm','30cm','a,c,g','2,6',0, 10),
('mi-011','cat-pizza-vegetarisch', 11,'Pizza Popai',           'mit Spinat und Knoblauch',                                                'pizza', 520, 840,'22cm','30cm','a,c,g','2,6',0, 11),
('mi-012','cat-pizza-vegetarisch', 12,'Pizza alla Rucola',     'mit Rucola und Parmesankäse',                                             'pizza', 600, 940,'22cm','30cm','a,c,g','2,6',0, 12),
('mi-014','cat-pizza-vegetarisch', 14,'Pizza Mais',            'mit Mais',                                                                'pizza', 600, 940,'22cm','30cm','a,c,g','2,6',0, 14),
('mi-015','cat-pizza-vegetarisch', 15,'Pizza Broccoli',        'mit Tomaten, Käse und Broccoli',                                          'pizza', 550, 890,'22cm','30cm','a,c,g','2,6',0, 15),
('mi-016','cat-pizza-vegetarisch', 16,'Pizza Artischocken',    'mit Artischocken',                                                        'pizza', 500, 940,'22cm','30cm','a,c,g','2,6',0, 16),
('mi-017','cat-pizza-vegetarisch', 17,'Pizza Veget. Spezial',  'mit Artischocken, Spinat, Broccoli, Spargel und Pilzen',                  'pizza', 550, 960,'22cm','30cm','a,c,g','2,6',0, 17);

-- ---------------------------------------------------------------------------
-- Calzone Pizzen — gefüllte Pizzen (single price, no 22/30 split)
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-018','cat-pizza-calzone', 18,'Pizza Calzone',          'mit Schinken, Salami und Pilzen',                            'calzone', 940, NULL, NULL, NULL, 'a,c,g',  '2,6,*,**', 0, 18),
('mi-019','cat-pizza-calzone', 19,'Pizza Mafiose',          'mit Schinken, Artischocken und Thunfisch',                   'calzone', 940, NULL, NULL, NULL, 'a,c,d,g','2,6,*',    0, 19),
('mi-020','cat-pizza-calzone', 20,'Pizza Speziale',         'mit Schinken, Paprika und Pilzen',                           'calzone', 960, NULL, NULL, NULL, 'a,c,g',  '2,6,*',    0, 20),
('mi-021','cat-pizza-calzone', 21,'Pizza L Piccola',        'mit Schinken, Ananas und Pilzen',                            'calzone', 940, NULL, NULL, NULL, 'a,c,g',  '2,6,*',    0, 21),
('mi-022','cat-pizza-calzone', 22,'Pizza La Mamma',         'mit Thunfisch, Mais und Knoblauch',                          'calzone', 960, NULL, NULL, NULL, 'a,c,d,g','2,6',      0, 22),
-- Note: print menu lists "22a Pizza Lüdinghausen" — a numbered variant. Stored under menu_number 23 won't fit; we keep the printed "22a" semantically by giving it number NULL and a clearer name.
('mi-022a','cat-pizza-calzone',NULL,'Pizza Lüdinghausen (22a)','mit Dönerfleisch, Zwiebeln',                              'calzone', 950, NULL, NULL, NULL, 'a,c,g',  '2,6',      0, 23);

-- ---------------------------------------------------------------------------
-- Pizza Spezialitäten — mit Fleischbelag
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-023','cat-pizza-fleisch', 23,'Pizza Spaghetti',  'mit Fleischsauce, Käse und Spaghetti',                'pizza', 550, 880,'22cm','30cm','a,c,g',     '2,6',     0, 23),
('mi-024','cat-pizza-fleisch', 24,'Pizza Bolognese',  'mit Fleischsauce und Käse',                          'pizza', 520, 880,'22cm','30cm','a,c,g',     '2,6',     0, 24),
('mi-025','cat-pizza-fleisch', 25,'Pizza Speziale',   'mit Fleischsauce, Spaghetti, Maccaroni und Käse',    'pizza', 560, 940,'22cm','30cm','a,c,g',     '2,6',     0, 25),
-- Print: "32a Pizza Sucuk" with a,c,g,1,3,2,6 — numbered out of order on the page.
('mi-032a','cat-pizza-fleisch',NULL,'Pizza Sucuk (32a)','mit türk. Knoblauchwurst, Paprika & Zwiebeln',     'pizza', 600, 940,'22cm','30cm','a,c,g',     '1,2,3,6', 0, 26);

-- ---------------------------------------------------------------------------
-- Pizza Spezialitäten — mit Fischbelag
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-026','cat-pizza-fisch', 26,'Pizza Portofino',   'mit Sardellen, Oliven und Kapern',         'pizza', 650,  940,'22cm','30cm','a,c,d,g','2,6',0, 26),
('mi-027','cat-pizza-fisch', 27,'Pizza Frutti di Mare','mit Meeresfrüchten',                     'pizza', 650,  950,'22cm','30cm','a,c,g,n','2,6',0, 27),
('mi-028','cat-pizza-fisch', 28,'Pizza al Salmone',  'mit Lachs, Spinat und Knoblauch',          'pizza', 660, 1060,'22cm','30cm','a,c,d,g','2,6',0, 28),
('mi-029','cat-pizza-fisch', 29,'Pizza Tonno',       'mit Thunfisch',                            'pizza', 520,  900,'22cm','30cm','a,c,d,g','2,6',0, 29),
('mi-030','cat-pizza-fisch', 30,'Pizza Scampi',      'mit Krabben',                              'pizza', 650,  950,'22cm','30cm','a,b,c,g','2,6',0, 30),
('mi-031','cat-pizza-fisch', 31,'Pizza Napoletana',  'mit Sardellen',                            'pizza', 540,  900,'22cm','30cm','a,c,d,g','2,6',0, 31);

-- ---------------------------------------------------------------------------
-- Pizza Spezialitäten — mit verschiedenen Belägen
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-032','cat-pizza-belaege', 32,'Pizza Fantasia',          'mit Salami, Thunfisch, Paprika und Zwiebeln',                                  'pizza', 640, 950,'22cm','30cm','a,c,d,g','2,6,**',0, 32),
('mi-033','cat-pizza-belaege', 33,'Pizza Prosciutto',        'mit Tomaten, Käse und Schinken',                                               'pizza', 580, 890,'22cm','30cm','a,c,g',  '2,6,*', 0, 33),
('mi-034','cat-pizza-belaege', 34,'Pizza Salami della Casa', 'mit Tomaten, Käse und Salami',                                                 'pizza', 580, 940,'22cm','30cm','a,c,g',  '2,6,**',0, 34),
('mi-035','cat-pizza-belaege', 35,'Pizza Tonno Cipolla',     'mit Thunfisch und Zwiebeln',                                                   'pizza', 600, 940,'22cm','30cm','a,c,d,g','2,6',   0, 35),
('mi-036','cat-pizza-belaege', 36,'Pizza Primavera',         'mit Salami und Pilzen',                                                        'pizza', 600, 940,'22cm','30cm','a,c,g',  '2,6,**',0, 36),
('mi-037','cat-pizza-belaege', 37,'Pizza Romana',            'mit Tomaten, Käse, Schinken und Pilzen',                                       'pizza', 600, 940,'22cm','30cm','a,c,g',  '2,6,*', 0, 37),
('mi-038','cat-pizza-belaege', 38,'Pizza Hawaii',            'mit Schinken und Ananas',                                                      'pizza', 580, 900,'22cm','30cm','a,c,g',  '2,6,*', 0, 38),
('mi-039','cat-pizza-belaege', 39,'Pizza Capricciosa',       'mit Salami, Schinken, Oliven und Sardellen',                                   'pizza', 590, 940,'22cm','30cm','a,c,d,g','2,6,*,**',0,39),
('mi-040','cat-pizza-belaege', 40,'Pizza Quattro Stagioni',  'mit Schinken, Paprika, Artischocken und Salami',                               'pizza', 600, 940,'22cm','30cm','a,c,g',  '2,6,*,**',0,40),
('mi-041','cat-pizza-belaege', 41,'Pizza des Hauses',        'mit Salami, Schinken, Pilzen und Thunfisch',                                   'pizza', 600, 950,'22cm','30cm','a,c,d,g','2,6,*,**',0,41),
('mi-042','cat-pizza-belaege', 42,'Pizza Mista',             'mit Salami, Schinken und Pilzen',                                              'pizza', 600, 940,'22cm','30cm','a,c,g',  '2,6,*,**',0,42),
('mi-043','cat-pizza-belaege', 43,'Pizza Gustosa',           'mit Salami und Thunfisch',                                                     'pizza', 600, 940,'22cm','30cm','a,c,d,g','2,6,**', 0,43),
('mi-044','cat-pizza-belaege', 44,'Pizza Claudia',           'mit Salami und Paprika',                                                       'pizza', 620, 880,'22cm','30cm','a,c,g',  '2,6,**', 0,44),
('mi-045','cat-pizza-belaege', 45,'Pizza Italia',            'mit Salami, Schinken und Thunfisch',                                           'pizza', 620, 920,'22cm','30cm','a,c,d,g','2,6,*,**',0,45),
('mi-046','cat-pizza-belaege', 46,'Pizza Nini',              'mit Salami, Schinken, Spinat und Knoblauch',                                   'pizza', 620, 940,'22cm','30cm','a,c,g',  '2,6,*,**',0,46),
('mi-047','cat-pizza-belaege', 47,'Pizza Tonnorella',        'mit Thunfisch, Schinken, Salami, Ei und Peperoni',                             'pizza', 640, 920,'22cm','30cm','a,c,d,g','2,6,*,**',0,47),
('mi-048','cat-pizza-belaege', 48,'Pizza Vito',              'mit Schinken und Salami',                                                      'pizza', 620, 890,'22cm','30cm','a,c,g',  '2,6,*,**',0,48),
('mi-049','cat-pizza-belaege', 49,'Pizza Napoli',            'mit Schinken, Spargel und gekochtem Ei',                                       'pizza', 620, 890,'22cm','30cm','a,c,g',  '2,6,*',   0,49),
('mi-051','cat-pizza-belaege', 51,'Pizza Inferno (scharf)',  'mit Schinken, Pilzen, Peperoni, Artischocken und Tabasco',                     'pizza', 640, 940,'22cm','30cm','a,c,g',  '2,6,*',   1,51),
('mi-052','cat-pizza-belaege', 52,'Pizza Contadina',         'mit Schinken, Zwiebeln und Ei',                                                'pizza', 640, 940,'22cm','30cm','a,c,g',  '2,6,*',   0,52),
('mi-053','cat-pizza-belaege', 53,'Pizza Calabrese',         'mit Thunfisch, Paprika und Ei',                                                'pizza', 580, 940,'22cm','30cm','a,c,d,g','2,6',     0,53),
('mi-054','cat-pizza-belaege', 54,'Pizza Mamma Mia',         'mit Schinken, Pilzen und Spargel',                                             'pizza', 620, 950,'22cm','30cm','a,c,g',  '2,6,*',   0,54),
('mi-055','cat-pizza-belaege', 55,'Pizza Messico',           'mit Schinken, Paprika und Mais',                                               'pizza', 590, 940,'22cm','30cm','a,c,g',  '2,6,*',   0,55),
('mi-056','cat-pizza-belaege', 56,'Pizza Rustica',           'mit Schinken, Paprika, Salami und Spargel',                                    'pizza', 590, 940,'22cm','30cm','a,c,g',  '2,6,*,**',0,56),
('mi-057','cat-pizza-belaege', 57,'Pizza Orientale',         'mit Thunfisch und Ananas',                                                     'pizza', 590, 940,'22cm','30cm','a,c,d,g','2,6',     0,57),
('mi-058','cat-pizza-belaege', 58,'Pizza Bellavista',        'mit Schinken, Salami, Paprika und Pilzen',                                     'pizza', 590, 940,'22cm','30cm','a,c,g',  '2,6,*,**',0,58),
('mi-059','cat-pizza-belaege', 59,'Pizza Diavolo',           'mit Thunfisch, Salami, Paprika, Oliven und Knoblauch',                         'pizza', 590, 940,'22cm','30cm','a,c,d,g','2,6,**',  0,59),
('mi-060','cat-pizza-belaege', 60,'Pizza La Luna',           'mit Salami, Schinken, Spinat, Broccoli, Scampi & Knoblauch',                   'pizza', 640,1000,'22cm','30cm','a,b,c,g','2,6,*,**',0,60),
('mi-061','cat-pizza-belaege', 61,'Pizza Prosciutto e Tonno','mit Schinken und Thunfisch',                                                   'pizza', 580, 940,'22cm','30cm','a,c,d,g','2,6,*',   0,61),
('mi-063','cat-pizza-belaege', 63,'Pizza Crossa',            'mit Spinat, Krabben, Sardellen, Oliven, Knoblauch',                            'pizza', 590, 940,'22cm','30cm','a,b,c,d,g','2,6',  0,63),
('mi-064','cat-pizza-belaege', 64,'Pizza Amorito',           'mit Schinken, Pilzen, Paprika, Broccoli, Zwiebeln und Ei',                     'pizza', 600, 940,'22cm','30cm','a,c,g',  '2,6,*',   0,64),
('mi-065','cat-pizza-belaege', 65,'Pizza Parmigiana',        'mit Parmaschinken, Rucola und Parmesankäse',                                   'pizza', 640,1000,'22cm','30cm','a,c,g',  '2,6,*',   0,65);

-- ---------------------------------------------------------------------------
-- Pizzen mit frischen Tomaten und Mozzarella
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-066','cat-pizza-tomate', 66,'Pizza La Bamba',  'mit Tomatenscheiben, Mozzarella, Thunfisch, Schinken & Oliven',         'pizza', 660, 960,'22cm','30cm','a,c,d,g','2,6,*',0, 66),
('mi-067','cat-pizza-tomate', 67,'Pizza Veneta',    'mit Tomatenscheiben, Mozzarella, Spinat, Oliven & Knoblauch',           'pizza', 660, 960,'22cm','30cm','a,c,g',  '2,6',   0, 67),
('mi-068','cat-pizza-tomate', 68,'Pizza Alberto',   'mit Tomatenscheiben, Mozzarella, Salami, Spinat, Broccoli',             'pizza', 660, 960,'22cm','30cm','a,c,g',  '2,6,**',0, 68),
('mi-069','cat-pizza-tomate', 69,'Pizza Perino',    'mit Tomatenscheiben, Mozzarella, Salami, Schinken, Sardellen & Knoblauch','pizza',660,990,'22cm','30cm','a,c,d,g','2,6,*,**',0,69);

-- ---------------------------------------------------------------------------
-- Pizzagerichte mit Sauce Hollandaise
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-070','cat-pizza-hollandaise', 70,'Pizza mit Putenstreifen','mit Paprika, Zwiebeln und Sauce Hollandaise', 'pizza', 600, 980,'22cm','30cm','a,c,g','2,6',0, 70),
('mi-071','cat-pizza-hollandaise', 71,'Pizza mit Ananas',       'mit Ananas und Sauce Hollandaise',            'pizza', 600, 840,'22cm','30cm','a,c,g','2,6',0, 71),
('mi-073','cat-pizza-hollandaise', 73,'Pizza mit Döner',        'mit Dönerfleisch und Sauce Hollandaise',      'pizza', 600, 940,'22cm','30cm','a,c,g','2,6',0, 73),
('mi-085','cat-pizza-hollandaise', 85,'Pizza mit Gyros',        'mit Gyros und Sauce Hollandaise',             'pizza', 600, 940,'22cm','30cm','a,c,g','2,6',0, 85);

-- ---------------------------------------------------------------------------
-- Pizzabrötchen (mit Knoblauchsauce oder Kräuterbutter)
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-400','cat-pizzabroetchen', 400,'Pizzabrötchen (6 Stück)',           'mit Knoblauchsauce oder Kräuterbutter',  'side', 600, NULL, NULL, NULL, 'a,c,g','2,6',0, 400),
('mi-401','cat-pizzabroetchen', 401,'Pizzabrötchen (6 Stück) mit Käse',  'gefüllt mit Käse',                       'side', 600, NULL, NULL, NULL, 'a,c,g','2,6',0, 401),
('mi-402','cat-pizzabroetchen', 402,'Pizzabrötchen (6 Stück) mit Belag', 'gefüllt mit 1 Belag nach Wahl',          'side', 660, NULL, NULL, NULL, 'a,c,g','2,6',0, 402);

-- ---------------------------------------------------------------------------
-- Nudelgerichte aus der Pfanne
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-075','cat-nudeln-pfanne', 75,'Spaghetti Bolognese',         'mit Fleischsauce',                                              'pasta', 800, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 75),
('mi-076','cat-nudeln-pfanne', 76,'Spaghetti Carbonara',         'mit Schinken, Sahnesauce und Ei',                               'pasta', 900, NULL, NULL, NULL, 'a,c,g',     '*',   0, 76),
('mi-077','cat-nudeln-pfanne', 77,'Spaghetti alla Panna',        'mit Schinken und Sahnesauce',                                   'pasta', 850, NULL, NULL, NULL, 'a,c,g',     '*',   0, 77),
('mi-078','cat-nudeln-pfanne', 78,'Spaghetti Frutti di Mare',    'mit Meeresfrüchten, Tomatensauce und Knoblauch',                'pasta', 900, NULL, NULL, NULL, 'a,c,g,n',   NULL,  0, 78),
('mi-079','cat-nudeln-pfanne', 79,'Spaghetti Roma',              'mit Tomatensauce',                                              'pasta', 800, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 79),
('mi-080','cat-nudeln-pfanne', 80,'Spaghetti della Casa',        'mit Schinken, Pilzen, Erbsen und Tomatensauce',                 'pasta', 950, NULL, NULL, NULL, 'a,c,g',     '*',   0, 80),
('mi-081','cat-nudeln-pfanne', 81,'Spaghetti Vegetaria',         'mit Erbsen, Broccoli und Sahnesauce',                           'pasta', 950, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 81),
('mi-082','cat-nudeln-pfanne', 82,'Spaghetti n. Art des Hauses', 'mit Maccaroni, Spaghetti, Broccoli und Fleischsauce',           'pasta', 950, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 82),
('mi-083','cat-nudeln-pfanne', 83,'Spaghetti Vulcano (scharf)',  'mit gegr. Spaghetti mit Oliven, Peperoni und Knoblauch in Tomatensauce','pasta',950,NULL,NULL,NULL,'a,c,g',NULL, 1, 83),
('mi-084','cat-nudeln-pfanne', 84,'Makkaroni Bolognese',         'mit Fleischsauce',                                              'pasta', 800, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 84),
('mi-086','cat-nudeln-pfanne', 86,'Makkaroni alla Panna',        'mit Schinken und Sahnesauce',                                   'pasta', 850, NULL, NULL, NULL, 'a,c,g',     '*',   0, 86),
('mi-087','cat-nudeln-pfanne', 87,'Makkaroni Vegetaria',         'mit Erbsen, Broccoli und Sahnesauce',                           'pasta', 950, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 87),
('mi-088','cat-nudeln-pfanne', 88,'Makkaroni Napolitana',        'mit Fleischsauce, frischen Pilzen und Broccoli',                'pasta', 950, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 88),
('mi-089','cat-nudeln-pfanne', 89,'Makkaroni 4 Formaggi',        'mit 4 verschiedenen Käsesorten',                                'pasta', 940, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 89),
('mi-090','cat-nudeln-pfanne', 90,'Makkaroni Roma',              'mit Tomatensauce',                                              'pasta', 800, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 90),
('mi-091','cat-nudeln-pfanne', 91,'Spaghetti Aglio Olio e Peperonico','mit Knoblauch, Olivenöl, Peperoni und Parmesan',           'pasta', 850, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 91),
('mi-092','cat-nudeln-pfanne', 92,'Makkaroni della Chef',        'mit frischen Tomaten, Krabben und Knoblauch',                   'pasta',1000, NULL, NULL, NULL, 'a,b,c,g',   NULL,  0, 92),
('mi-093','cat-nudeln-pfanne', 93,'Makkaroni Gorgonzola',        'mit Gorgonzola und Sahnesauce',                                 'pasta', 950, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 93),
('mi-094','cat-nudeln-pfanne', 94,'Makkaroni e Spinata',         'mit Gorgonzola, Spinat und Sahnesauce',                         'pasta',1000, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 94),
('mi-097','cat-nudeln-pfanne', 97,'Tortellini Bolognese',        'mit Fleischsauce',                                              'pasta', 850, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 97),
('mi-098','cat-nudeln-pfanne', 98,'Tortellini Roma',             'mit Tomatensauce',                                              'pasta', 850, NULL, NULL, NULL, 'a,c,g',     NULL,  0, 98),
('mi-099','cat-nudeln-pfanne', 99,'Tortellini alla Panna',       'mit Schinken und Sahnesauce',                                   'pasta', 900, NULL, NULL, NULL, 'a,c,g',     '*',   0, 99);

-- ---------------------------------------------------------------------------
-- Nudelgerichte aus dem Backofen (typisch italienisch)
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-102','cat-nudeln-backofen',102,'Lasagne',                       'mit Schinken und Fleischsauce',                                 'oven',1000, NULL, NULL, NULL, 'a,c,g','*', 0,102),
('mi-107','cat-nudeln-backofen',107,'Maccaroni de la Casa Al Forno','mit Schinken, Erbsen, Champignons, Sahnesauce und Käse',         'oven', 950, NULL, NULL, NULL, 'a,c,g','*', 0,107),
('mi-108','cat-nudeln-backofen',108,'Maccaroni Al Forno',            'mit Schinken, Fleischsauce und Käse',                           'oven', 950, NULL, NULL, NULL, 'a,c,g','*', 0,108),
('mi-109','cat-nudeln-backofen',109,'Maccaroni Roma',                'mit Tomatensauce',                                              'oven', 850, NULL, NULL, NULL, 'a,c,g',NULL,0,109),
('mi-110','cat-nudeln-backofen',110,'Tortellini de la Casa Al Forno','mit Schinken, Erbsen, Champignons in Sahnesauce & Käse',         'oven', 980, NULL, NULL, NULL, 'a,c,g','*', 0,110),
('mi-111','cat-nudeln-backofen',111,'Maccaroni Alla Panna',          'mit Schinken, Sahnesauce und Käse',                             'oven', 900, NULL, NULL, NULL, 'a,c,g','*', 0,111),
('mi-113','cat-nudeln-backofen',113,'Maccaroni Vegetaria',           'mit Erbsen, Broccoli, Sahnesauce und Käse',                     'oven', 950, NULL, NULL, NULL, 'a,c,g',NULL,0,113),
('mi-114','cat-nudeln-backofen',114,'Spaghetti Al Forno',            'mit Schinken, Sahnesauce und Käse',                             'oven', 900, NULL, NULL, NULL, 'a,c,g','*', 0,114),
('mi-115','cat-nudeln-backofen',115,'Spaghetti Roma',                'mit Tomatensauce und Käse',                                     'oven', 850, NULL, NULL, NULL, 'a,c,g',NULL,0,115),
('mi-116','cat-nudeln-backofen',116,'Spaghetti Alla Panna',          'mit Schinken, Sahnesauce und Käse',                             'oven', 900, NULL, NULL, NULL, 'a,c,g','*', 0,116),
('mi-119','cat-nudeln-backofen',119,'Spaghetti Vegetaria',           'mit Erbsen, Broccoli, Sahnesauce und Käse',                     'oven', 950, NULL, NULL, NULL, 'a,c,g',NULL,0,119),
('mi-123','cat-nudeln-backofen',123,'Tortellini Al Forno',           'mit Fleischsauce, Schinken und Käse',                           'oven', 950, NULL, NULL, NULL, 'a,c,g','*', 0,123),
('mi-125','cat-nudeln-backofen',125,'Tortellini Alla Panna',         'mit Schinken, Sahnesauce und Käse',                             'oven', 950, NULL, NULL, NULL, 'a,c,g','*', 0,125),
('mi-128','cat-nudeln-backofen',128,'Tortellini alla Roß',           'mit Schinken, Erbsen, Broccoli, Sahnesauce und Käse',           'oven',1050, NULL, NULL, NULL, 'a,c,g','*', 0,128),
('mi-129','cat-nudeln-backofen',129,'Tortellini Amore',              'Tortellini, Maccaroni, Spaghetti, Schinken, Sahnesauce und Käse','oven', 940, NULL, NULL, NULL, 'a,c,g','*', 0,129);

-- ---------------------------------------------------------------------------
-- Broccoli Aufläufe
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-130','cat-broccoli',130,'Broccoli alla Panna','mit Broccoli, Spargel, Sahnesauce und Käse','oven', 950, NULL, NULL, NULL, 'g','2,6',0,130),
('mi-131','cat-broccoli',131,'Broccoli Roma',     'mit Broccoli, Spargel Tomatensauce und Käse','oven', 950, NULL, NULL, NULL, 'g','2,6',0,131);

-- ---------------------------------------------------------------------------
-- Gourmet Pasta
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-300','cat-gourmet',300,'Spaghetti Al Pesto',         'Spaghetti mit hausgemachter Pestosauce & Knoblauch',                     'pasta', 950,  NULL, NULL, NULL, 'a,c,g','2,6',0,300),
('mi-301','cat-gourmet',301,'Spaghetti Caprese',          'mit frischen Tomaten, Knoblauch, Basilikum und Mozzarella',              'pasta', 980,  NULL, NULL, NULL, 'a,c,g','2,6',0,301),
('mi-302','cat-gourmet',302,'Maccaroni in Lachs',         'mit Lachs-Sahnesauce',                                                   'pasta', 980,  NULL, NULL, NULL, 'a,c,d,g','2,6',0,302),
('mi-303','cat-gourmet',303,'Spaghetti Scampi',           'mit Scampi, frischen Tomaten, Basilikum und Knoblauch',                  'pasta',1140,  NULL, NULL, NULL, 'a,c,g,n','2,6',0,303),
('mi-304','cat-gourmet',304,'Spaghetti dello Chef',       'mit Gambas, Knoblauch, getrockneten Tomaten in einer scharfen Sauce',    'pasta',1350,  NULL, NULL, NULL, 'a,b,c,g','2,6',1,304),
('mi-305','cat-gourmet',305,'Tagliatelle al Salmone',     'mit Lachs, Krabben in Sahnesauce',                                       'pasta', 930,  NULL, NULL, NULL, 'a,b,c,d,g','2,6',0,305),
('mi-306','cat-gourmet',306,'Tagliatelle alla Nerone',    'mit Gorgonzolasauce und Spinatsauce',                                    'pasta', 890,  NULL, NULL, NULL, 'a,c,g','2,6',0,306),
('mi-307','cat-gourmet',307,'Tagliatelle',                'mit Steinpilze, frische Tomaten, Knoblauch, fr. Kräuter in Sahnesauce',  'pasta',1000,  NULL, NULL, NULL, 'a,c,g','2,6',0,307),
('mi-308','cat-gourmet',308,'Gnocchi alla Bolognese',     'mit Fleischsauce',                                                       'pasta', 950,  NULL, NULL, NULL, 'a,c,g','2,6',0,308),
('mi-309','cat-gourmet',309,'Gnocchi al Pomodoro',        'mit Tomatensauce',                                                       'pasta', 800,  NULL, NULL, NULL, 'a,c,g','2,6',0,309),
('mi-311','cat-gourmet',311,'Gnocchi al Gorgonzola',      'mit Gorgonzolasauce',                                                    'pasta', 890,  NULL, NULL, NULL, 'a,c,g','2,6',0,311);

-- ---------------------------------------------------------------------------
-- Putengeschnetzeltes
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-134','cat-puten',134,'Putengeschnetzeltes mit Pilzen',     'mit Maccaroni und frischen Pilzen in Sahnesauce','meat', 940, NULL, NULL, NULL, 'c,g','2,6',0,134),
('mi-135','cat-puten',135,'Putengeschnetzeltes mit Currysauce', 'mit Maccaroni und Currysauce',                  'meat', 940, NULL, NULL, NULL, 'c,g','2,6',0,135),
('mi-136','cat-puten',136,'Putengeschnetzeltes mit Pilzen frisch','mit Maccaroni, frischen Pilzen und frischen Tomaten in Sahnesauce','meat',940,NULL,NULL,NULL,'c,g','2,6',0,136);

-- ---------------------------------------------------------------------------
-- Schnitzel
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-160','cat-schnitzel',160,'Schnitzel',           'mit Champignon-Sahnesauce dazu Nudeln oder kl. Salat','meat',1240, NULL, NULL, NULL, 'a,c,g','2,6',0,160),
('mi-161','cat-schnitzel',161,'Putengeschnetzeltes', 'mit Zitronensauce dazu kl. Salat',                    'meat',1240, NULL, NULL, NULL, 'a,c,g','2,6',0,161);

-- ---------------------------------------------------------------------------
-- Salatspezialitäten
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-137','cat-salat',137,'Insalata Mista',        'mit Salat, Gurken und Tomaten',                                                    'salad', 550, NULL, NULL, NULL, NULL,    NULL, 0,137),
('mi-138','cat-salat',138,'Insalata Verde',        'grüner Salat',                                                                     'salad', 450, NULL, NULL, NULL, NULL,    NULL, 0,138),
('mi-139','cat-salat',139,'Insalata di Pomodori',  'mit Tomaten',                                                                      'salad', 450, NULL, NULL, NULL, NULL,    NULL, 0,139),
('mi-140','cat-salat',140,'Insalata di Cetrioli',  'Gurkensalat',                                                                      'salad', 450, NULL, NULL, NULL, NULL,    NULL, 0,140),
('mi-141','cat-salat',141,'Insalata Napoli',       'Tomaten, Gurken, Thunfisch, Käse, Ei und Zwiebeln',                                'salad', 900, NULL, NULL, NULL, 'c,d,g','2,6', 0,141),
('mi-142','cat-salat',142,'Insalata Italia',       'Tomaten, Gurken, Thunfisch, Schinken, Artischocken u. Käse',                       'salad', 900, NULL, NULL, NULL, 'c,d,g','2,6,*',0,142),
('mi-143','cat-salat',143,'Insalata Primavera',    'Tomaten, Gurken, Thunfisch, Schinken, Salami und Käse',                            'salad', 900, NULL, NULL, NULL, 'c,d,g','2,6,*,**',0,143),
('mi-144','cat-salat',144,'Insalata Amore',        'Tomaten, Gurken, Thunfisch, Paprika, Zwiebeln, Artischocken, Oliven und Käse',     'salad', 950, NULL, NULL, NULL, 'd,g',  '2,6', 0,144),
('mi-145','cat-salat',145,'Insalata Siciliana',    'Tomaten, Gurken, Artischocken, Zwiebeln, Peperoni, Oliven u. Käse',                'salad', 950, NULL, NULL, NULL, 'g',    '2,6', 0,145),
('mi-146','cat-salat',146,'Überraschungssalat',    NULL,                                                                                'salad',1000, NULL, NULL, NULL, NULL,    NULL, 0,146),
('mi-147','cat-salat',147,'Insalata Roma',         'Tomaten, Gurken, Artischocken, Zwiebeln, Paprika, Mais u. Käse',                   'salad', 950, NULL, NULL, NULL, 'g',    '2,6', 0,147),
('mi-148','cat-salat',148,'Insalata La Luna',      'Tomaten, Gurken, Schafskäse, Paprika, Schinken, Zwiebeln u. Möhren',               'salad',1000, NULL, NULL, NULL, 'g',    '2,6,*',0,148),
('mi-149','cat-salat',149,'Insalata Frutti di Mare','Tomaten, Gurken, Thunfisch, Schafskäse, Krabben und Käse',                         'salad', 980, NULL, NULL, NULL, 'b,d,g,n','2,6',0,149),
('mi-150','cat-salat',150,'Insalata Hawaii',       'Tomaten, Gurken, Thunfisch, Schinken, Ananas u. Schafskäse',                       'salad', 980, NULL, NULL, NULL, 'd,g',  '2,6,*',0,150),
('mi-151','cat-salat',151,'Insalata Mamma Mia',    'Tomaten, Gurken, Schafskäse, Zwiebeln, Ei, Artischocken, Möhren u. Spargel',       'salad',1000, NULL, NULL, NULL, 'c,g',  '2,6', 0,151),
('mi-152','cat-salat',152,'Insalata des Hauses',   'Tomaten, Gurken, Schafskäse, Mais, Möhren und Zwiebeln',                           'salad', 900, NULL, NULL, NULL, 'g',    '2,6', 0,152),
('mi-153','cat-salat',153,'Insalata Caprese',      'mit Salat, Tomaten, Mozzarella und Basilikum',                                     'salad', 950, NULL, NULL, NULL, 'g',    '2,6', 0,153),
('mi-155','cat-salat',155,'Insalata della Chef',   'Tomaten, Gurken, Putenfleisch in Sahne-Curry-Sauce',                                'salad',1000, NULL, NULL, NULL, 'g',    '2,6', 0,155);

-- ---------------------------------------------------------------------------
-- Rucolasalat / Erfrischungs Salate
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-156','cat-rucola',156,'Rucolasalat mit Champignons',  'mit Parmesanstreifen und frischen Champignons',                'salad', 900, NULL, NULL, NULL, 'g','2,6',0,156),
('mi-157','cat-rucola',157,'Rucolasalat mit Mozzarella',   'mit Mozzarella und Tomaten',                                   'salad', 950, NULL, NULL, NULL, 'g','2,6',0,157),
('mi-158','cat-rucola',158,'Rucolasalat mit Thunfisch',    'mit Thunfisch, Oliven, Zwiebeln, Mozzarella, Sardellen & Ei',  'salad',1000, NULL, NULL, NULL, 'c,d,g','2,6',0,158),
('mi-159','cat-rucola',159,'Rucolasalat mit Lachsstücken', 'mit gebratenen Lachsstücken',                                  'salad',1050, NULL, NULL, NULL, 'd,g','2,6',0,159);

-- ---------------------------------------------------------------------------
-- Getränke (no menu_number on print menu — sort_order keeps display order)
-- ---------------------------------------------------------------------------
INSERT INTO menu_items (id, category_id, menu_number, name, description, item_type,
    price_small_cents, price_large_cents, size_small_label, size_large_label,
    allergen_codes, additive_codes, is_spicy, sort_order) VALUES
('mi-drk-vitamalz',     'cat-getraenke', NULL, 'Vitamalz',           '0,33 l', 'drink', 220, NULL, '0,33l', NULL, NULL, NULL, 0, 900),
('mi-drk-cocacola',     'cat-getraenke', NULL, 'Coca Cola',          '0,33 l', 'drink', 220, NULL, '0,33l', NULL, NULL, '7,9',0, 901),
('mi-drk-cocacolazero', 'cat-getraenke', NULL, 'Coca Cola Zero',     '0,33 l', 'drink', 220, NULL, '0,33l', NULL, NULL, '7,8,9',0, 902),
('mi-drk-fanta',        'cat-getraenke', NULL, 'Fanta / Exotic / Lemon','0,33 l','drink',220,NULL, '0,33l', NULL, NULL, '9',  0, 903),
('mi-drk-sprite',       'cat-getraenke', NULL, 'Sprite',             '0,33 l', 'drink', 220, NULL, '0,33l', NULL, NULL, '9',  0, 904),
('mi-drk-wasser-still', 'cat-getraenke', NULL, 'Wasser still',       '0,33 l', 'drink', 150, NULL, '0,33l', NULL, NULL, NULL, 0, 905),
('mi-drk-durstloescher','cat-getraenke', NULL, 'Durstlöscher',       '0,50 l', 'drink', 200, NULL, '0,50l', NULL, NULL, NULL, 0, 906);
