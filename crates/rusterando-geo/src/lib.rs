//! Local, in-process address lookup for Rusterando.
//!
//! Replaces the public Nominatim round-trip in the checkout with an index
//! built from the official NRW cadastre (Gebäudereferenzen, Datenlizenz
//! Deutschland – Zero). Street + house-number autocomplete, exact lat/lon per
//! building, and delivery-zone evaluation — all without an outbound request
//! and without customer addresses leaving the server.
//!
//! Layers (nothing above the index depends on the data backend):
//!
//! - [`normalize`] — the one normaliser used for stored names and user input
//! - [`utm`] — EPSG:25832 (ETRS89 / UTM 32N) → WGS84, pure Rust
//! - [`index`] — [`GeoIndex`]: streets + house numbers, suggest/resolve
//! - [`import`] — [`AddressSource`] trait + the [`CadastreNrw`] backend
//! - [`store`] — SQLite persistence (`geo_streets` / `geo_addresses`)
//! - [`zones`] — delivery-zone precedence: street override → geometry →
//!   default; GeoJSON / radius parsing + validation
//! - [`config`] — `GEO_*` tenant env keys
//! - [`sample`] — built-in fixture so everything works offline
//!
//! The `geo-import` binary (src/bin) drives an import for one tenant.

pub mod config;
pub mod import;
pub mod index;
pub mod normalize;
pub mod sample;
pub mod store;
pub mod utm;
pub mod zones;

pub use config::GeoConfig;
pub use import::{AddressSource, CadastreNrw, ImportSummary};
pub use index::{GeoIndex, HouseNo, Resolved, Street};
pub use zones::{zone_for, Geometry, GeometryKind, StreetOverride, ZoneDef, ZoneMatch};
