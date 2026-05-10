-- Customer-facing display names for delivery zones. The original seed used
-- the `postcode` column as a hacky tag because Lüdinghausen, Seppenrade and
-- the surrounding Bauerschaften all share PLZ 59348 — there's no way to
-- distinguish them by postcode alone. We give the customer a dropdown of
-- proper names instead.

ALTER TABLE delivery_zones ADD COLUMN name TEXT;

UPDATE delivery_zones SET name = 'Lüdinghausen'  WHERE id = 'dz-luedinghausen';
UPDATE delivery_zones SET name = 'Seppenrade'    WHERE id = 'dz-seppenrade';
UPDATE delivery_zones SET name = 'Bauerschaften' WHERE id = 'dz-bauerschaften';
