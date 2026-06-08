-- Delivery-area configuration + address-rejection audit log.
--
-- ## Two-layer service-area model
--
-- A customer address is accepted for delivery if EITHER:
--
--   (A) Nominatim's resolved municipality (city/town/municipality field)
--       matches one of the entries under `served_municipalities` —
--       implements the public promise ("wir liefern in Lüdinghausen"
--       means every Bauerschaft that OSM tags under Lüdinghausen counts,
--       even if it sits at the geographic edge), OR
--
--   (B) The resolved (lat, lon) falls inside one of the explicit circles
--       under `extra_circles` — for outliers David serves by choice
--       (e.g. selected Nordkirchen-Bauerschaften) but where Nominatim
--       does NOT return "Lüdinghausen" as the municipality.
--
-- Plus a 30 km sanity wall (in code, not config) that guards against
-- Nominatim mis-resolving to a homonym (the "Lüdinghausen, Sachsen-
-- Anhalt" trap).
--
-- The whole thing is one JSON setting — `shop_delivery_areas`. Schema:
--
--   {
--     "served_municipalities": [
--       {
--         "match_names": ["Lüdinghausen", "Luedinghausen"],
--         "zone_routing": [
--           { "match_suburb": "Seppenrade",   "zone_id": "dz-seppenrade" },
--           { "match_suburb": "Lüdinghausen", "zone_id": "dz-luedinghausen" },
--           { "match_suburb": "*",            "zone_id": "dz-bauerschaften" }
--         ]
--       }
--     ],
--     "extra_circles": [
--       { "label": "Nordkirchen-Bauerschaften (Auswahl)",
--         "center_lat": 51.7, "center_lon": 7.5, "radius_km": 2.5,
--         "zone_id": "dz-bauerschaften" }
--     ]
--   }
--
-- Match-rule:
--
--   * `match_names` and `match_suburb` are compared umlaut-folded and
--     case-insensitively (see locality::normalize). So
--     "Lüdinghausen"/"Luedinghausen"/"LUEDINGHAUSEN" all match.
--
--   * `zone_routing` is walked top-to-bottom; first specific suburb that
--     matches wins. `"match_suburb": "*"` is the optional fallback for
--     "any suburb in this municipality we haven't pinned to a specific
--     zone". Without it, an unmatched suburb is rejected (wrong_suburb).
--
--   * `extra_circles` is walked in order; first whose haversine to the
--     resolved point is within `radius_km` wins.
--
-- Customer-input aliases (LH, Luedinghausen, …) are NOT relevant for
-- classify_zone: Nominatim does its own search-side normalisation —
-- the customer can type "LH" or "Luedinghausen" and OSM resolves it.
-- We only need the umlaut-fold on the RESPONSE side. The cart drawer's
-- separate PLZ → city autofill is purely cosmetic (see
-- `shop_postcode_hints` below).
--
-- Empty defaults. The framework migration must NOT seed any shop-
-- specific data — Rusterando is multi-tenant. Per-shop locality
-- configuration (which towns + suburbs + circles + bypass) is
-- bootstrapped via `data/locality.<profile>.sql` seed files applied
-- by the deploy script after migrations run. Davids' real config
-- lives in `data/locality.davids.sql` (gitignored).
--
-- A fresh shop boots with NO served municipalities → every delivery
-- address is rejected as `wrong_municipality` until the admin
-- configures /admin/localities. That's the intended out-of-box
-- behaviour: don't quietly accept deliveries the admin hasn't
-- promised yet.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('shop_delivery_areas',
   '{"served_municipalities":[],"extra_circles":[]}',
   'Liefergebiet (Orte + Inseln)',
   'JSON: belieferte Orte (match_names + zone_routing) und zusätzliche Liefer-Inseln (Umkreis-Kreise). Wird auf /admin/localities bearbeitet.');

-- PLZ → Ort autofill hints. PURELY KOSMETISCH — wird nicht zur
-- Adressprüfung benutzt. Wenn der Kunde im Warenkorb eine bekannte
-- PLZ eintippt, wird der zugehörige Ort als Vorschlag ergänzt.
-- Leer per Default — der Shop-Admin trägt seine PLZ ein.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('shop_postcode_hints',
   '[]',
   'PLZ → Ort Vorschläge (Warenkorb-Autofill)',
   'Reine UI-Hilfe: wenn der Kunde eine dieser PLZ eintippt, wird der Ort vorgeschlagen. Beeinflusst NICHT die Liefergebiets-Prüfung.');

-- Paid-bypass rule: if a customer's address would normally be rejected
-- (wrong_municipality / wrong_suburb), accept it anyway when the order
-- is ONLINE-PAID (card or voucher-to-0) and the total reaches the
-- configured threshold. The 30 km sanity wall still applies (a
-- "Lüdinghausen, Sachsen-Anhalt" hit isn't a real customer).
--
-- Default 0 = feature off. Set both values together (threshold + zone)
-- on /admin/localities to enable.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('shop_bypass_paid_min_cents',
   '0',
   'Bezahlte Bestellungen außerhalb: Schwelle (Cent)',
   'Ab welchem Online-bezahlten Bestellwert (in Cent) wir auch außerhalb des Standard-Liefergebiets liefern. 0 = aus.'),
  ('shop_bypass_zone_id',
   '',
   'Bezahlte Bestellungen außerhalb: Zone',
   'Welche Liefer-Zone für solche Bestellungen gilt (Gebühr/ETA). Leer = aus.');

-- ---------------------------------------------------------------------------
-- address_attempts: durable log of every delivery address that failed to
-- classify into a zone. The admin page /admin/address-attempts reads from
-- this table to diagnose "customer says they couldn't order".
--
-- Storage budget is bounded by:
--   * boot-time prune of rows older than 60 days
--   * hard cap of 5 000 rows; oldest evicted on insert (FIFO)
-- Both enforced from the server crate; this table just holds rows.
--
-- PII note: input_city / input_postcode / input_street are the customer's
-- raw typing — keep retention short. customer_phone/email are populated
-- only when a customer is already identified (logged-in or paying); do not
-- bind aggressively.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS address_attempts (
    id                 TEXT PRIMARY KEY,
    attempted_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    input_street       TEXT NOT NULL,
    input_house_number TEXT NOT NULL,
    input_postcode     TEXT NOT NULL,
    input_city         TEXT NOT NULL,
    -- Nominatim's hit, when present. NULL when the geocoder returned zero
    -- results or errored before responding.
    resolved_lat       REAL,
    resolved_lon       REAL,
    resolved_city      TEXT,
    resolved_suburb    TEXT,
    -- Haversine km from shop to resolved point, when coords present. Useful
    -- to spot near-radius rejections without re-running the math.
    distance_km        REAL,
    -- One of:
    --   no_results          — Nominatim returned 0 hits
    --   http_error          — Geocoder unreachable / 5xx
    --   out_of_radius       — past 30 km sanity wall
    --   wrong_municipality  — resolved city not in served_municipalities AND no extra_circle hit
    --   wrong_suburb        — municipality matched but suburb not in zone_routing AND no '*' fallback
    --   no_zone             — zone_routing pointed at a delivery_zones.id that doesn't exist / is inactive
    rejection_reason   TEXT NOT NULL,
    -- Human-readable detail, e.g.
    --   "32.1 km from shop (limit 30 km)"
    --   "city='Senden' not in [Lüdinghausen, Luedinghausen]; no extra-circle match"
    --   "Nominatim returned 0 hits"
    rejection_detail   TEXT,
    -- Pretty-printed first Nominatim hit. Useful for the admin to see what
    -- OSM thinks the address looks like. NULL for no_results / http_error.
    raw_nominatim_json TEXT,
    -- Best-effort identity. Populated when the customer is already known
    -- (logged-in cart session). Often NULL for guest carts.
    customer_phone     TEXT,
    customer_email     TEXT
);

CREATE INDEX IF NOT EXISTS idx_address_attempts_at
    ON address_attempts(attempted_at DESC);
