//! Reusable machinery for hand-split wasm artifacts — the pattern used by the
//! i18n translation pack (`rusterando-i18n-pack`), the translation management
//! wasm (`rusterando-i18n-mgmt`), and any future split-wasm tool.
//!
//! Two independent facets, behind two features so a consumer pulls only what it
//! needs (and the heavy deps never bleed into crates that don't):
//!
//!   * `build`   — codegen helpers for a consumer's `build.rs`: turn
//!                 `{ locale: { key: value } }` JSON into sorted
//!                 `&'static [(…)]` Rust slices baked with `include!`. No JSON
//!                 parser ships in the wasm; lookups are binary search.
//!                 (build-dependency side; pulls serde_json at build time only.)
//!
//!   * `interop` — browser side: call a function on a loaded split wasm that the
//!                 loader registered at `window.__<name>` (see
//!                 wasm-split-loader.js). Hydrate-only; pulls wasm-bindgen.
//!
//! The shell builder (`scripts/build-wasm-split.sh`) and the loader
//! (`crates/rusterando-frontend/src/static/wasm-split-loader.js`) are the
//! language-agnostic halves of the same pattern.

#[cfg(feature = "build")]
pub mod build;

#[cfg(feature = "interop")]
pub mod interop;
