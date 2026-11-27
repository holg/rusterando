-- Receipt logos rendered on the server (crates/kitchen-protocol/RECEIPT_LOGOS.md).
-- One row per logo and width: the composed SVG it came from and its 1-bit
-- raster (row-major, MSB-first, set bit = black dot). Print jobs in
-- kitchen_outbox reference a logo by `hash`
-- (kitchen_protocol::receipt::raster_hash(svg, target_width)); every send
-- turns the reference into the raster (or, for older devices, the SVG).
-- Additive only: nothing existing is changed by this file.
CREATE TABLE IF NOT EXISTS receipt_logos (
    hash         BLOB    PRIMARY KEY,
    svg          BLOB    NOT NULL,
    target_width INTEGER NOT NULL,
    width        INTEGER NOT NULL,
    height       INTEGER NOT NULL,
    bits         BLOB    NOT NULL,
    created_at   INTEGER NOT NULL
);
