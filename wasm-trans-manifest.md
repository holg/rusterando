# wasm-trans-manifest

The translation system for the rusterando frontend: a **DB-first** model with an
optional, hand-split **translation `.wasm`** overlay, a **management wasm** for
per-shop gap reporting, reusable **split-wasm machinery**, and a planned
**ratatui TUI** to manage translations over SSH (iPhone-optimised).

This is both the architecture reference (what exists) and the plan for the TUI
(what's next).

---

## 1. Model — DB-first, wasm fills the gaps

The database is **always served first**. Translations are NOT bulk-loaded into
the DB; the `_<lang>` columns are kept **intentionally empty** and act as the
**per-shop override slot**. When a cell is empty, the loaded translation pack
supplies the value. German is the base.

**Resolution order for any translatable string:**

1. **DB `_<lang>` cell** — if non-null, use it. This is a deliberate per-shop
   override; the DB always wins.
2. **Translation pack `.wasm`** — if loaded and it has the string, use it. The
   normal translated path. (Browser-only; loaded on demand.)
3. **German base** — final fallback (pack not loaded / string missing).

Consequences:
- The DB stays sparse (German + deliberate overrides only).
- Multi-tenant safe: the pack is shared *vocabulary*; each shop's DB holds its
  own truth + overrides.
- The pack loads/unloads **live** in the browser and caches by content hash —
  a returning visitor whose hash matches fetches nothing.
- SSR renders DB-first (German / DB override) → first paint is correct and
  hydration-safe; the pack swaps empty-cell strings in **after** hydration.

Two string domains:
- **Chrome** (UI labels) — keyed by dotted namespace key (`menu.add_to_cart`).
  Source: `crates/rusterando-frontend/locales/<locale>.json`.
- **Menu** (item names / descriptions / option labels) — keyed by the **German
  source phrase** (`"mit Tomaten und Mozzarella"`). Source:
  `rusterando-scrape/menu_strings.<locale>.json` (deduped phrases).

---

## 2. Categorization — topics (product-agnostic)

Translations are categorized by **topic**, declared in `i18n-pack.toml`. The
format is product-agnostic: each product (`rusterando` here; `lighting` for the
eulumdat/gldf apps) is its own topic space. A pack = **LOCALES × TOPICS**.

Topics map to chrome **namespaces** (the `locales/*.json` top-level keys) plus
the menu vocabulary:

| Topic     | Contents                                                        |
|-----------|-----------------------------------------------------------------|
| `shared`  | `common`, `header`, `errors` — cross-cutting, auto-included      |
| `shop`    | `home`, `menu`, `cart`, `voucher`, `checkout`, `order_confirm`, `lieferservice` |
| `kitchen` | `status`, `payment_status`, `shop_status`, `notify` — back-office/ops |
| `menu`    | the menu-item vocabulary (German→7-locale phrases)              |

> NB: the Pi printer's own receipt strings live in the `kitchen-protocol` crate,
> NOT in this i18n system yet. The `kitchen` topic is the operational *UI*
> strings that exist in `locales/`.

**Profiles** (`[profiles.*]`) name LOCALES+TOPICS sets: `fat` = all 7 non-German
locales × all topics; `small` = a lean subset. Override with `--locales` /
`--topics`.

---

## 3. Artifacts & crates

| Crate | Role | Runtime deps |
|---|---|---|
| `crates/rusterando-i18n-pack` | the translation pack wasm (chrome + menu data) | wasm-bindgen only |
| `crates/rusterando-i18n-mgmt` | management wasm: per-shop gap reporting | wasm-bindgen only |
| `crates/rusterando-wasm-split` | **reusable** split-wasm machinery (shared) | feature-gated |

**Data is baked into sorted `&'static [..]` slices at build time** (via
`rusterando-wasm-split::build` in each crate's `build.rs`), looked up by binary
search — no JSON parser, no HashMap, no alloc in the wasm. The generated JSON
(`generated/*.json`) is the committed source the slices are baked from.

Served sizes (brotli): **pack ≈ 40 KB** (7 locales × 4 topics), **mgmt ≈ 14 KB**
(coverage keys only). A narrowed `--topics kitchen --locales en` pack is ≈ 7 KB.

### `rusterando-wasm-split` — the shared machinery

Reuse this for ANY future split-wasm tool; don't re-roll it.

- **`build` feature** (`build.rs` side): `Codegen` turns `{locale:{key:value}}`
  JSON into sorted slices — `full_table` (locale,key,value), `key_table`
  (locale,key membership set), `locales_union`, `literal_str`. serde_json runs
  at build time only; never ships.
- **`interop` feature** (hydrate side): call a window-registered split wasm —
  `call(windowKey, fn, args)`, `is_ready`, `load(loaderFn)`,
  `load_and_call(...)`. All failures degrade to `None` (never panics the page).
- **`scripts/build-wasm-split.sh`** — the shell half: env-driven
  (`WS_CRATE`/`WS_WASM_NAME`/`WS_DIST`/`WS_LOADER`/`WS_EXTRA_MANIFEST`) →
  cargo wasm32 → wasm-bindgen → wasm-opt -Oz → md5-hash filenames →
  `manifest.json` → hashed loader → brotli → `target/site/pkg/<WS_DIST>/`.
- **`wasm-split-loader.js`** — the JS half: `makeWasmSplitLoader({windowKey,
  loaderKey, manifest, importBase, exports, loadedEvent})` fetches the manifest,
  dynamic-imports the hashed glue, inits, and registers the surface on
  `window[windowKey]`. Thin per-wasm loaders (`i18n-loader.js`,
  `i18n-mgmt-loader.js`) just call it.

### Hash-based caching

Every artifact filename carries a content hash (`<name>-<hash>_bg.wasm`); the
loader reads `manifest.json` (small, cacheable) for the hashed name, so it never
hardcodes a hash. Hashed files are served immutable. Change the data → new hash
→ new filename → re-fetch; otherwise the client re-uses its cache. **This is the
data-saving core: clients fetch only what they don't already have.**

---

## 4. Build & data flow

```
locales/*.json  (chrome)                    menu_strings.<locale>.json  (menu)
        \                                            /
         └──────► gen_i18n_pack.py (reads i18n-pack.toml: profile→locales×topics)
                          │   emits generated/{chrome,menu}.json + hash.txt + topics.txt
                          ▼
   build-i18n-pack.sh ──► build-wasm-split.sh ──► /pkg/i18n/{<hashed>.js,_bg.wasm,manifest.json}
                                                       + /pkg/wasm-split-loader.js + i18n-loader.js

   build-i18n-mgmt.sh ─► build-wasm-split.sh ──► /pkg/i18n-mgmt/{...}  (coverage from the pack's menu.json)
```

Commands:
- `scripts/build-i18n-pack.sh [--profile fat|small | --locales .. --topics ..]`
- `scripts/build-i18n-mgmt.sh`  (run after the pack if menu data changed)

Translation *source* tooling (`rusterando-scrape/`):
- `export_menu_strings.py` — dump a tenant DB's unique German menu phrases to a
  worklist.
- `import_menu_translations.py` — write translations into the DB `_<lang>`
  columns (overrides only — bulk translations belong in the pack, not the DB).

---

## 5. Runtime wiring (browser)

- SSR shell injects `wasm-split-loader.js` + `i18n-loader.js` **only when i18n
  is enabled** for the shop. German-only shops ship nothing extra.
- On hydrate, for a non-German locale, `App()` calls `window.__loadI18nPack()`;
  on the `i18n-pack-loaded` event a reactive generation signal bumps so every
  `t!()` / `t_menu()` view re-renders with the now-available translations.
- `t(key)` → pack `lookup` → German. `t_menu(db_value, german_source)` → DB
  override wins; else pack `menuLookup`; else German.
- `/admin/translations` loads the **mgmt** wasm and renders the per-shop gap
  report (which of the shop's phrases are untranslated, per locale) with a JSON
  download.

---

## 6. ratatui TUI — manage translations over SSH (iPhone-optimised)

### Why
The web `/admin/translations` page gives a gap report, but filling translations
and managing per-shop overrides from a phone is painful in a browser. A
**ratatui terminal app** run over SSH (e.g. from Blink/Termius on iPhone) is
faster for keyboard-driven bulk work, and works anywhere there's a shell.

### What it does (scope: gap report + edit)
1. **Pick a tenant DB** (`data/<slug>.sqlite`) — or the `_shared` template.
2. **Gap view** — per locale, the German phrases this shop uses that have no
   translation (in the pack OR the DB). Same logic as the mgmt wasm, computed
   natively in Rust (shares the `menu_strings.*.json` / pack coverage as the
   reference set).
3. **Edit** — pick a phrase, type the translation for a locale; writes the DB
   `_<lang>` override cell (the DB-first override slot). Optionally also append
   to `menu_strings.<locale>.json` (the pack source) for the next pack build.
4. **Filter** — by locale, by topic (chrome vs menu), by "missing only".
5. **Trigger a pack rebuild** (optional) — shell out to
   `build-i18n-pack.sh` / `build-i18n-mgmt.sh`.

### iPhone-over-SSH constraints (the "optimised" part)
- **Narrow width** (~40–50 cols in portrait): single-column layout, no wide
  tables; truncate + wrap; avoid horizontal scrolling.
- **Few keystrokes**: single-key actions (`j/k` move, `e` edit, `s` save,
  `f` filter, `n`/`p` next/prev locale, `q` quit). Avoid chords needing modifier
  keys that soft keyboards hide.
- **No mouse**: fully keyboard-navigable; no hover.
- **Latency-tolerant**: minimal redraws, debounced input; assume a laggy link.
- **Resilient TTY**: handle small/odd terminal sizes and resizes; restore the
  terminal cleanly on exit/panic (ratatui's restore guard).
- **Touch-keyboard friendly editing**: a simple single-line input with a big,
  obvious cursor; paste-friendly (translators paste from another app).

### Shape — BUILT (`crates/rusterando-i18n-tui`, bin `i18n-tui`)
- Deps: `ratatui` + `crossterm` (TUI), `rusqlite` (bundled SQLite — no system
  lib needed on the VPS), `serde_json`, and **`rusterando-i18n-core`** for the
  gap computation.
- **Gap logic extracted to `rusterando-i18n-core`** (pure, zero deps): the
  `Coverage` trait + `gaps()` / `gap_report_json()`. The mgmt wasm passes a
  `SliceCoverage` (its build-baked slice); the TUI passes a `SetCoverage` read
  from the `menu_strings.<locale>.json` files. **Same code path → they can't
  drift.**
- Run: `i18n-tui [--data ./data] [--strings ./rusterando-scrape]`.
- Screens: **Tenants** (pick `data/<slug>.sqlite`) → **Gaps** (one row per
  locale × phrase, missing-only by default, `f` toggles all) → **Edit** (type
  the translation) → save. Edits write the DB `_<lang>` **override** cell
  (DB-first slot) for every row matching the German base; empty input clears the
  override. Status bar shows tenant, mode, counts, key hints.
- iPhone/SSH: single-column narrow layout, single-key nav (`j/k` move, `e` edit,
  `f` filter, `b` back, `q` quit; `Enter`/`Esc` in edit), blocking input (no
  polling — battery-friendly over SSH), and a restore guard so a panic can't
  wedge the phone terminal.

### Wasm SOURCE = a translations SQLite (canonical, editable)

A `.wasm` is compiled and can't be edited in place, so the editable source of
truth for the MENU translations is **`rusterando-scrape/menu-translations.sqlite`**
(table `translations(locale, german, value, topic, source)`, PK
`(locale, german, topic)`). The builder reads it:
`gen_i18n_pack.py --source-sqlite <db>` (defaults to that path) → `menu.json` →
wasm. The per-locale `menu_strings.<lang>.json` are now a **fallback** (used only
if the sqlite is absent). Verified the sqlite path produces a **byte-identical
`menu.json` + same data hash** as the JSON path.

Seed it once: `rusterando-scrape/migrate_to_source_sqlite.py` imports the 7
`menu_strings.<lang>.json` (1127 rows, topic `menu`).

**TUI wasm-source mode** (`Wasm-Quelle bearbeiten` from the tenant picker):
browse the source grouped/filtered by build **profile** (fat/small — `p` cycles)
and **locale** (`l` cycles), edit a translation (`e` → writes the sqlite),
**merge** another translations sqlite (`--merge <db>` at startup), and **rebuild**
the wasm (`r` → shells out to `build-i18n-pack.sh --profile <X>`). The whole
edit→rebuild loop is verified (edit bumps the data hash; restore returns it).

### Decisions taken
- Shared gap logic → its own crate `rusterando-i18n-core` (not bolted onto
  wasm-split), since it's pure logic both the wasm and the native TUI use.
- v1 edits write the **DB override** only (the DB-first slot). Appending to the
  `menu_strings.*.json` pack source + triggering a pack rebuild are deliberate
  follow-ups (keeps the TUI a focused DB editor; pack rebuilds stay a CI/script
  step).
- v1 covers menu item **name + description**; category/option labels are a
  follow-up (same pattern, more columns).

---

## 7. File map

```
i18n-pack.toml                              topic manifest (product/topics/profiles)
crates/rusterando-i18n-pack/                the translation pack wasm
  generated/{chrome,menu}.json, hash.txt    committed pack source (baked by build.rs)
crates/rusterando-i18n-mgmt/                the gap-report management wasm
crates/rusterando-i18n-core/                shared pure gap logic (wasm + TUI)
crates/rusterando-i18n-tui/                 ratatui SSH/iPhone translation manager
crates/rusterando-wasm-split/               shared: build codegen + browser interop
crates/rusterando-frontend/src/i18n.rs      t()/t_menu(), pack bridge, reactive gen
crates/rusterando-frontend/src/static/      wasm-split-loader.js + thin loaders
crates/rusterando-frontend/src/pages/admin/translations.rs   /admin/translations
scripts/build-wasm-split.sh                 generic packager
scripts/build-i18n-pack.sh                  pack wrapper (data-gen + packager)
scripts/build-i18n-mgmt.sh                  mgmt wrapper
rusterando-scrape/gen_i18n_pack.py          pack data generator (reads i18n-pack.toml)
rusterando-scrape/menu_strings.<locale>.json   menu vocabulary (7 locales)
```
