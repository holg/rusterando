-- Local address lookup (NRW cadastre) + geometry-based delivery zones.
--
-- geo_streets / geo_addresses: the tenant's imported building addresses,
-- written by `geo-import` (crates/rusterando-geo), read once at boot into
-- the in-memory index. Empty tables = tenant keeps the geocoder path.
--
-- delivery_zones gains geometry: `priority` (lower wins), `geometry_kind`
-- ('none' | 'polygon' | 'radius' | 'default') and `geometry` (GeoJSON
-- Polygon/MultiPolygon, or "lat,lon,metres"). geo_zone_streets holds
-- explicit street → zone assignments that beat any geometry. Precedence
-- lives in rusterando_geo::zones::zone_for.
--
-- orders gets the resolved point + a verified flag so the route optimiser
-- can use the cadastre coordinates and the operator can spot unverified
-- addresses before dispatch.

CREATE TABLE IF NOT EXISTS geo_streets (
  id          INTEGER PRIMARY KEY,
  ags         TEXT NOT NULL,
  street_key  TEXT NOT NULL,
  name        TEXT NOT NULL,      -- display form, original casing and umlauts
  name_norm   TEXT NOT NULL,      -- rusterando_geo::normalize::normalize_street
  postcode    TEXT NOT NULL,
  city        TEXT NOT NULL,
  UNIQUE (ags, street_key)
);
CREATE INDEX IF NOT EXISTS geo_streets_norm ON geo_streets(name_norm);

CREATE TABLE IF NOT EXISTS geo_addresses (
  street_id   INTEGER NOT NULL REFERENCES geo_streets(id) ON DELETE CASCADE,
  number      INTEGER NOT NULL,             -- numeric part
  suffix      TEXT NOT NULL DEFAULT '',     -- lowercase, no spaces ('a', 'b', '')
  lat         REAL NOT NULL,
  lon         REAL NOT NULL,
  PRIMARY KEY (street_id, number, suffix)
);

ALTER TABLE orders ADD COLUMN lat REAL;
ALTER TABLE orders ADD COLUMN lon REAL;
ALTER TABLE orders ADD COLUMN address_verified INTEGER NOT NULL DEFAULT 0;

ALTER TABLE delivery_zones ADD COLUMN priority INTEGER NOT NULL DEFAULT 100;
ALTER TABLE delivery_zones ADD COLUMN geometry_kind TEXT NOT NULL DEFAULT 'none';
ALTER TABLE delivery_zones ADD COLUMN geometry TEXT;

-- Explicit street → zone overrides. number_from/number_to use sentinels
-- (0 / 999999 = open) instead of NULL so the UNIQUE index actually dedupes.
CREATE TABLE IF NOT EXISTS geo_zone_streets (
  id          INTEGER PRIMARY KEY,
  zone_id     TEXT NOT NULL REFERENCES delivery_zones(id) ON DELETE CASCADE,
  street_id   INTEGER NOT NULL,
  number_from INTEGER NOT NULL DEFAULT 0,
  number_to   INTEGER NOT NULL DEFAULT 999999,
  UNIQUE (street_id, number_from, number_to)
);
CREATE INDEX IF NOT EXISTS geo_zone_streets_street ON geo_zone_streets(street_id);

-- Davids' seed (no-op on other tenants): the existing town zone becomes the
-- high-priority centre zone (geometry to be assigned by street or polygon
-- in the dashboard), the Bauerschaften zone becomes the default catch-all.
-- Until the centre is configured every delivery evaluates to the default
-- fee, which matches the previous behaviour and is safe.
UPDATE delivery_zones SET priority = 10 WHERE id = 'dz-luedinghausen';
UPDATE delivery_zones SET priority = 100, geometry_kind = 'default' WHERE id = 'dz-bauerschaften';

-- Admin switch: which address check the checkout uses.
--   'nominatim' — the existing geocoder + suburb routing (default, unchanged)
--   'local'     — the imported cadastre index + geometry zones
-- Only effective when the tenant has an imported index; the zones admin
-- page shows the index status next to the switch.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('address_lookup_mode', 'nominatim', 'Adressprüfung',
   'nominatim = Online-Geocoder (OpenStreetMap) + Stadtteil-Routing; local = lokaler Adressindex (Katasterdaten) + Zonen-Geometrie. Umschalten unter Liefergebiete.');
