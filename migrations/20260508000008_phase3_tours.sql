-- Phase 3: route-optimised delivery tours.
--
-- A tour bundles N "ready" orders into one optimised driver run. Stops are
-- materialised at start_tour time (sequence comes from ORS, or oldest-first
-- when ORS is unavailable). order.tour_id lets the admin/kitchen views
-- show which orders are currently on a tour and which are still waiting.
--
-- The raw ORS response is stashed on the tour row for debugging — useful
-- when a route looks weird and you want to inspect what the optimiser saw.

CREATE TABLE delivery_tours (
    id                TEXT PRIMARY KEY,
    driver_name       TEXT,
    started_at        TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at       TIMESTAMP,
    total_distance_m  INTEGER,
    total_duration_s  INTEGER,
    optimised_by      TEXT NOT NULL DEFAULT 'fallback',  -- 'ors' | 'fallback'
    ors_response_json TEXT,
    created_at        TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_delivery_tours_open
    ON delivery_tours(finished_at) WHERE finished_at IS NULL;

CREATE TABLE delivery_tour_stops (
    tour_id      TEXT NOT NULL REFERENCES delivery_tours(id) ON DELETE CASCADE,
    sequence     INTEGER NOT NULL,
    order_id     TEXT NOT NULL REFERENCES orders(id),
    eta_seconds  INTEGER,                  -- seconds since tour start; NULL for fallback ordering
    arrived_at   TIMESTAMP,
    delivered_at TIMESTAMP,
    PRIMARY KEY (tour_id, sequence)
);

CREATE INDEX idx_tour_stops_order ON delivery_tour_stops(order_id);

ALTER TABLE orders ADD COLUMN tour_id TEXT REFERENCES delivery_tours(id);
CREATE INDEX idx_orders_tour ON orders(tour_id);
