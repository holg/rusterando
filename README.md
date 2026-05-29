# Rusterando

> **A self-hostable, open-source restaurant + delivery platform written in Rust.**
> Online menu, cart, checkout (cash + Stripe), kitchen board, driver tour
> optimisation, push notifications, live order tracking over SSE, an
> opening-hours / pause / snooze scheduler, vouchers, a customer CRM, a
> Typst-rendered printable PDF menu with switchable cover + theme
> libraries, runtime-toggleable 8-locale i18n, multi-role staff tooling,
> and an admin UI to edit it all without redeploying.

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust](https://img.shields.io/badge/Rust-edition_2021-orange.svg)](https://www.rust-lang.org/)
[![Leptos](https://img.shields.io/badge/Leptos-0.8_SSR-green.svg)](https://leptos.dev/)

---

## What is it for?

A neighbourhood pizzeria, a takeaway shop, or a small chain that:

- wants its own website and online ordering instead of paying 14% per order to
  a marketplace,
- needs a kitchen view, a delivery dispatcher, and a way to push notifications
  to staff phones,
- runs on one box (a €5/month VPS is plenty for hundreds of orders a day),
- prefers AGPL self-hosting to a SaaS vendor.

Rusterando is the platform Davids Pizzeria has been running on since early 2026.
The codebase was generalised and re-licenced once it became clear other shops
could use the same shape.

## Highlights

- **Pure Rust full stack.** Leptos 0.8 SSR + hydration on the client side,
  Axum on the server, sqlx + SQLite for storage, Stripe for payments,
  apns-h2 for iOS push, Typst for the printed menu PDF.
- **Single binary, single SQLite file.** Deploy is `rsync + systemd restart`.
  No Postgres, no Redis, no JS toolchain, no container runtime. ~72 MB
  release binary that boots in milliseconds.
- **One source tree, many shops.** Each deployment is a single
  `.env.<profile>` file. `./scripts/deploy_to_server.sh -e davids full`
  ships Davids Pizzeria; `./scripts/deploy_to_server.sh -e demo full`
  ships rusterando.de from the *same checkout*, on the *same VPS*,
  to a different port + domain + DB. Add a third shop in five minutes:
  copy `.env.demo.example` → `.env.<shop>`, point an nginx vhost at
  the new port, deploy.
- **Editable in production.** Hero copy, photos, offer cards, the gallery,
  the menu, the extras catalogue, the delivery zones, the free-delivery
  threshold, the colour theme, the opening hours, the PDF cover + Typst
  theme, vouchers, the shop's contact/imprint details — all live in
  `app_settings`, `home_*`, `menu_*`, and the PDF/voucher tables. The
  admin edits everything from the `/admin/*` pages. No redeploy.
- **Live order tracking over SSE.** A per-order Server-Sent-Events channel
  (`/api/live/orders/{id}`) pushes status changes and admin→customer
  messages to the customer's open tab with no polling; a global
  `/api/live/shop` channel flips the "we're open / closed" banner on the
  home and cart pages the instant staff toggle it. Messages carry a
  delivery ack so the admin sees "✓ Zugestellt" when the customer's
  browser actually rendered them.
- **Open / closed on your terms.** A weekly opening-hours editor, dated
  holiday overrides ("closed 24.12", "open on the Easter Monday"), a
  manual "close now" switch, a timed **snooze** ("stop taking orders for
  90 min, auto-reopen"), and an ad-hoc **force-open** (accept past
  closing today). The checkout enforces all of it server-side and only
  offers valid pickup slots.
- **Switchable printed menu.** `/menu.pdf` renders live from the DB via
  Typst. A **cover-image library** and a **Typst theme library** let the
  admin upload several covers / templates and flip the active one from
  `/admin/pdf` — with a test-render endpoint that rejects a broken
  template before it can poison the live PDF.
- **Real iOS app.** A native Swift WebView shell (`ios-app/`) wraps the
  site and adds APNs push, persistent staff cookies, a role keychain,
  and a settings sheet. Distributed via TestFlight today; Android +
  FCM is on the roadmap.
- **Multi-role staff sign-in.** Customer (no auth), Admin, Kitchen, Driver.
  One device can hold all three staff passwords in iOS Keychain and
  switch in one tap from the gear FAB.
- **Order lifecycle covered.** Cart → checkout (cash or Stripe Payment
  Element, with vouchers + a returning-customer recall) → kitchen board →
  driver tour (with OpenRouteService route optimisation) → SMTP
  confirmation email → audit trail in the bookkeeping page
  (`/admin/history`) with CSV export and a Lieferando-savings simulation.
- **Eight locales, flipped at runtime.** German is the default and needs
  no URL prefix; English, French, Italian, Spanish, Portuguese, Russian
  and Simplified Chinese are built in. The `i18n_enabled` toggle in
  `/admin/settings` turns the language switcher + `/<lang>/*` routes +
  hreflang sitemap on or off without a rebuild — a single-language shop
  serves German only and 404s every locale path.
- **Found by Google.** `/sitemap.xml` (with hreflang alternates when i18n
  is on) and `/robots.txt` (disallowing `/admin`, `/kitchen`, `/driver`,
  `/checkout`, `/orders/`) are served straight from the binary.

## Screenshots

(*Not yet — the davidspizzeria.de live site is the reference.*)

## Features

### Customer-facing

| Route | Page | What it does |
|---|---|---|
| `/` | Home | Hero, offer cards, photo gallery, delivery-zone list, public opening-hours table, live "open / closed" banner. Hero + offers + gallery are admin-edited. |
| `/menu` | Menu | Categorised menu with item cards (price per size, description, allergen + additive codes, spicy badge). Adds to cart with size, extras checkboxes, and required option-groups. Optional sticky category overlay. Download-PDF button. |
| `/checkout` | Checkout | Cart review, phone/name recall of returning customers, pickup-or-delivery with zone fee + min-order, voucher entry, cash or Stripe Payment Element. Server-validated opening hours offer only real pickup slots; a closed / paused / snoozed shop blocks ordering with a banner (pre-orders still allowed when configured). |
| `/orders/{id}` | Confirmation + tracking | Order status, QR code, and a **live message feed** over SSE — the customer sees status changes and admin messages without reloading, and their browser sends a delivery ack back. |
| `/datenschutz`, `/impressum` | Legal | GDPR privacy text and the TMG §5 imprint (address / owner / tax id / bank from the admin contact settings). |

### Kitchen & driver

| Route | Page | What it does |
|---|---|---|
| `/kitchen` | Kitchen board | Cook-focused three-column board (eingegangen → in Zubereitung → abholbereit), one-click status advance, no revenue or refund controls. Login at `/kitchen/login`. |
| `/driver` | Driver board | Delivery-only board: bundle ready orders into a tour, ORS-optimised stop order with ETAs + map deep-links, mark stops delivered, finish or trim a tour. Login at `/driver/login`. |

### Live updates (SSE)

A `tokio::sync::broadcast` hub fans events to two Server-Sent-Events streams
(`crates/rusterando-frontend/src/live.rs`, served from
`crates/rusterando-server/src/main.rs`):

- **`/api/live/orders/{id}`** — per-order: `Status` changes, admin→customer
  `Message`s (logged + persisted in `order_messages`), and `MessageAck`
  (customer browser confirms render → admin's open order view flips to
  "✓ Zugestellt").
- **`/api/live/shop`** — global `ShopStatus { closed, reason }`; the home +
  cart banners and the admin online/offline chip flip live when staff
  pause, snooze, force-open, or hit a Ruhetag.

Subscriptions live in post-hydration effects so they never alter the
SSR/hydrate DOM — the `tests/e2e/` hydration suite guards against the
mismatch panic that would otherwise cause.

## Admin tooling

Every page below sits under `/admin/*`, is gated by an 8-hour
`admin_session` cookie (`/admin/login`, password = `ADMIN_PASSWORD`), and
is reachable from the top nav in `AdminShell`. None of it requires a
redeploy — it all reads and writes the live DB.

| Route | What you manage |
|---|---|
| `/admin` | Dashboard: today's order count + revenue, tiles to every section. |
| `/admin/orders` · `/admin/orders/{id}` | Order list with status transitions + revenue; per-order detail with the live SSE message box (send a message to the customer, watch the delivery ack). |
| `/admin/menu` | Categories + items: prices per size, allergens, additives, availability, per-item extras allowance + flat-extra price, and attached option-groups (Dressing, Beilage, …). |
| `/admin/extras` | Pizza-topping catalogue: label, per-piece price, availability. |
| `/admin/pricing` | Price-sanity checker — flags extras that undercut named pizzas and suggests corrected prices to protect margins. |
| `/admin/home` | Public landing page: hero image + copy, two offer cards, gallery photos (uploads via `/api/admin/upload_image`). |
| `/admin/hours` | Weekly opening hours, dated holiday overrides, the **close-now** switch, **snooze** (timed auto-reopen) and **force-open** (accept past closing today). Flips the live banner via SSE. |
| `/admin/zones` | Delivery zones: postcode → name, fee, min-order, ETA. Drives checkout city auto-complete, route optimisation, and the home-page "Wir liefern" list. |
| `/admin/vouchers` | Vouchers: percent / fixed / free-delivery, min-subtotal, first-order-only, per-phone + global caps, validity window, optional bind-to-one-customer. |
| `/admin/customers` · `/admin/customers/{id}` | Lightweight CRM: order count + lifetime spend per phone, addresses on file, notes, and a blacklist flag that blocks cash orders. |
| `/admin/pdf` | PDF editor — text fields (tagline, hours, extras), ad-slot images, the **cover-image library** (upload / activate / delete; PNG auto-transcoded to JPEG for Typst), and the **Typst theme library** (create / edit / activate, with a safe test-render). |
| `/admin/broadcast` | One-shot push to staff roles or all devices; each send is audited in `push_broadcasts`. |
| `/admin/settings` | Generic `app_settings` editor: free-delivery threshold, colour theme, Stripe sandbox/live mode, `i18n_enabled`, category overlay, contact / imprint details, the order-pause flag, and an optional Google Search Console verification token (`shop_google_site_verification` — empty = no meta tag; when set it's rendered into the homepage `<head>`). |
| `/admin/history` | Bookkeeping: date-range order list, daily + per-item totals, **CSV export** (`/admin/history.csv`), and the **Lieferando-savings simulation** (14% commission + per-order fees vs. Stripe-only on your own site, net of VAT). |

## Architecture

```
                         ┌─────────────────────┐
        Browser ─────────│  Axum + Leptos SSR  │─────── Stripe webhook
                         │  (rusterando-server)│        SMTP (lettre)
                         │                     │        APNs (apns-h2)
                         │  ┌───────────────┐  │        ORS routing API
                         │  │ rusterando-   │  │
                         │  │ frontend lib  │  │
                         │  │ (Leptos       │  │
                         │  │  components,  │  │
                         │  │  server fns)  │  │
                         │  └───────────────┘  │
                         └──────────┬──────────┘
                                    │ sqlx
                         ┌──────────▼──────────┐
                         │  SQLite             │
                         │  data/<name>.db     │
                         └─────────────────────┘
```

Crates in the workspace:

| Crate | Role |
|---|---|
| `rusterando-shared` | Pure-Rust types shared between server and frontend (cart, menu, orders). No I/O. |
| `rusterando-frontend` | Leptos components + `#[server]` functions. Compiles to **WASM** (hydrate) and **native** (SSR). |
| `rusterando-server` | Axum HTTP server. Wires Leptos SSR, sqlx, Stripe, APNs, SMTP, the static asset handler, the upload endpoint. |
| `rusterando-app` | Tiny placeholder for legacy desktop targets — currently not used. |

The site is **fully server-rendered first, then hydrated**. Search engines and
slow phones see a complete page in the first byte. The cart and any
client-only interactivity (Stripe Payment Element, image upload, sticky cart
drawer) come alive after hydration.

### Why these choices

- **Leptos 0.8 over Yew/Sycamore/Dioxus.** Leptos has the most mature SSR
  story in Rust today: real streaming SSR, server functions that look like
  ordinary `async fn`s, and a hydration model that survives full-fat
  server-rendered HTML without needing a separate API surface.
- **SQLite over Postgres.** Single-file backups (`zip data/`), zero
  operational overhead, fast enough for a single-shop workload by
  several orders of magnitude. We lose nothing here.
- **No Docker.** A static binary + a systemd unit + a `.env` is simpler to
  reason about, faster to deploy, and easier to debug than any container
  runtime. Migrations run automatically at server start via
  `sqlx::migrate!()`.
- **Typst over LaTeX/wkhtmltopdf.** The printed menu is a single
  `templates/menu.typ` file, the binary embeds the Typst engine, fonts are
  baked into the binary at build time (see `crates/rusterando-server/build.rs`),
  and the PDF renders byte-identically across machines.

## Repository layout

```
.
├── Cargo.toml                   workspace + cargo-leptos metadata
├── .env.example                 every supported env var, with explanations
├── .env                         your local secrets (gitignored)
├── crates/
│   ├── rusterando-shared/       cart / order / menu types
│   ├── rusterando-frontend/     Leptos pages + components + server fns
│   ├── rusterando-server/       Axum + Leptos host, Stripe / APNs / SMTP
│   └── rusterando-app/          (unused)
├── migrations/                  sqlx migrations applied at boot
├── style/main.scss              SCSS source for site CSS
├── public/                      static assets shipped under /
│   ├── img/                     pizza-1.jpg, ladenfront.jpg, …
│   └── .well-known/             apple-app-site-association (Universal Links)
├── templates/menu.typ           printable menu (Typst)
├── ios-app/                     native Swift WebView shell
├── scripts/
│   ├── deploy_to_server.sh      .env-driven deploy (rsync + systemd)
│   ├── cross_build_on_mac.sh    Mac → Linux release-prod cross-build
│   └── test-ci-locally.sh       fmt + clippy + check (SSR + WASM)
└── docs/                        deeper deployment + architecture notes
```

## Quickstart (local dev)

You need: Rust stable, `cargo-leptos`, and `sass` on your `$PATH`.

```bash
# 1. clone
git clone https://github.com/holg/rusterando
cd rusterando

# 2. install build helpers
cargo install cargo-leptos sass

# 3. minimal .env so the dev server can boot
cat > .env <<'EOF'
APP_NAME=rusterando-server
DATABASE_URL=sqlite:./data/rusterando.db
ADMIN_PASSWORD=devadmin
KITCHEN_PASSWORD=devkitchen
DRIVER_PASSWORD=devdriver
PUBLIC_URL=http://127.0.0.1:3001
EOF

# 4. run — migrations + asset build all happen automatically
cargo leptos serve
# → open http://127.0.0.1:3001
```

First visit creates `./data/rusterando.db` and seeds default rows
(empty branding, the warm theme, no offers, no menu, no delivery zones).
Sign in at `/admin/login` with `ADMIN_PASSWORD` and start filling in:

1. **`/admin/settings`** — shop name, address, phone, email (the
   contact / imprint block), free-delivery threshold, colour theme.
2. **`/admin/menu`** — categories, items, sizes, option-groups.
3. **`/admin/extras`** — pizza toppings.
4. **`/admin/home`** — hero photo, two offer cards, gallery photos.
5. **`/admin/hours`** — weekly opening hours.
6. **`/admin/zones`** — delivery zones (postcode → fee, min-order, ETA).
7. **`/admin/pdf`** — printed-menu cover, theme, and text fields (optional).

See `docs/delivery_zones_and_routing.md` for how zones feed route
optimisation.

## Configuration: every `.env` key

Rusterando is configured **entirely through `.env`**. No build-time flags,
no per-shop overlay scripts. The same source tree powers every deployment;
only `.env` changes.

For multiple shops on one VPS, name the files `.env.<profile>` (e.g.
`.env.davids`, `.env.demo`) and pass `-e <profile>` to the deploy
script. See [Multi-tenant on one VPS](#multi-tenant-on-one-vps) below.

### Naming + paths

| Key | Default | Example (Davids) | Meaning |
|---|---|---|---|
| `APP_NAME` | `rusterando-server` | `davidspizzeria-server` | Binary filename on disk and systemd unit name. The deploy script renames `target/.../rusterando-server` → `$APP_NAME` during rsync, so the on-server filename stays stable for the life of the deployment. |
| `BIN_NAME` | (unset) | `davidspizzeria-server` | Optional alias for `APP_NAME`. |
| `LEPTOS_OUTPUT_NAME` | `rusterando` | `davidspizzeria` | JS/WASM bundle name in `target/site/pkg/<name>.<hash>.js`. Setting this stable across renames means existing browser caches don't miss after a deploy. |
| `LEPTOS_SITE_ADDR` | `127.0.0.1:3001` | `127.0.0.1:3001` | Localhost bind address. Each deployment on the same VPS picks a different port (3001, 3002, …); nginx vhosts route by hostname to the right one. |
| `DEPLOY_REMOTE_BASE` | (unset) | `/var/www/example.com` | Server-side install root. Holds the binary, `html/`, `data/`, `backups/`, `.env`. |
| `SSH_HOST` | (unset) | `myhost.example` | Where `scripts/deploy_to_server.sh` SSHs to. |
| `DATABASE_URL` | `sqlite:./data/rusterando.db` | `sqlite:./data/<name>.db` | sqlx connection string. SQLite only today. |
| `PUBLIC_URL` | `http://127.0.0.1:3001` | `https://www.example.com` | Base URL used in QR codes, emails, Stripe return URLs. |

### Auth + secrets

| Key | Required | Meaning |
|---|---|---|
| `ADMIN_PASSWORD` | yes | Plaintext password for `/admin/login`. Cookie `admin_session=ok` on success. |
| `KITCHEN_PASSWORD` | yes | Same shape, for `/kitchen/login`. |
| `DRIVER_PASSWORD` | yes | Same shape, for `/driver/login`. |

### Stripe (optional — cash-only mode works without)

| Key | Meaning |
|---|---|
| `STRIPE_PUBLISH_KEY` | Public key shipped to the browser. |
| `STRIPE_SECRET_KEY` | Server-side key. |
| `STRIPE_WEBHOOK_SECRET` | For `/api/webhook/stripe` signature verification. |

### SMTP (optional — falls back to log-only)

| Key | Meaning |
|---|---|
| `SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_PASS` | Lettre over native-tls. |
| `EMAIL_FROM` | Friendly From: address used for order confirmations. |

### Apple Push Notifications (optional)

| Key | Meaning |
|---|---|
| `APNS_TEAM_ID` | Apple Developer team id. |
| `APNS_KEY_ID` | APNs auth key id. |
| `APNS_BUNDLE_ID` | Your iOS app bundle id. |
| `APNS_KEY_PATH` | Filesystem path to the `.p8` auth key. |
| `APNS_PRODUCTION` | `true` for App Store builds, `false` for sandbox. |

### Routing (optional — order-batching uses it)

| Key | Meaning |
|---|---|
| `ORS_API_KEY` | OpenRouteService API key. Free tier is fine. |
| `ORS_BASE_URL` | Override if you self-host ORS. |
| `PIZZERIA_LAT`, `PIZZERIA_LON` | Where the driver's tour starts and ends. |

If `APNS_KEY_PATH` is unset, push is silently disabled and orders still
work end-to-end. Same pattern for SMTP and Stripe — the platform fails
soft so you can run it bare.

## Building + deploying

### CI locally

```bash
./scripts/test-ci-locally.sh
```

Runs `cargo fmt`, clippy (SSR), check (SSR), check (WASM hydrate target),
and check (server). Mirrors what GitHub Actions runs, so green-locally
== green-on-PR.

### Cross-build (Mac → Linux)

```bash
./scripts/cross_build_on_mac.sh x86_64-unknown-linux-gnu
```

Produces `target/x86_64-unknown-linux-gnu/release-prod/rusterando-server`
plus the hashed JS/WASM/CSS bundle in `target/site/pkg/`. Build profile
is **release-prod**: fat LTO, `codegen-units = 1`, `strip = symbols`.
Roughly 72 MB binary, 6 minutes on M-series Mac.

Cross-toolchain prerequisites are documented at the top of
`scripts/cross_build_on_mac.sh`. Short version: install
`x86_64-unknown-linux-gnu` via the messense Homebrew tap, drop OpenSSL +
SQLite headers in `~/opt/{openssl,sqlite}_for_cross/`.

### First-time server setup

```bash
./scripts/deploy_to_server.sh setup
```

Creates `/var/www/<your domain>/{html,data,backups}/`, uploads your
`.env`, writes a systemd unit named after `APP_NAME`, enables and
starts the service.

### Subsequent deploys

```bash
./scripts/deploy_to_server.sh full
```

Builds, backs up the live install (zipped to `backups/`), rsyncs the new
binary + site assets, restarts, then applies any per-deployment seed SQL
under `data/*.deploy.sql` (idempotent — see [Multi-tenant on one
VPS](#multi-tenant-on-one-vps)). Backup retention is the last 10 zips.
Roll back interactively with `./scripts/deploy_to_server.sh restore`.

The deploy script reads `.env` at the very top — same one the server runs
under — so build-time and run-time configuration never drift.

> **Multiple shops on one VPS?** Use `-e <profile>` to load
> `.env.<profile>` instead of `.env`:
> `./scripts/deploy_to_server.sh -e davids full`. See the
> [Multi-tenant section](#multi-tenant-on-one-vps).

## Multi-tenant on one VPS

A single Rusterando checkout can deploy any number of independent shops
to the same VPS. Each shop has its own `.env.<profile>` file at the repo
root; `./scripts/deploy_to_server.sh -e <profile> <command>` loads it.
Everything per-shop — domain, port, binary name, systemd unit, SQLite
DB, photo uploads, backups, branding seed SQL — flows from that one
file.

This is how davidspizzeria.de (real shop) and rusterando.de (public
demo install) share a single VPS today. Same source tree, same Rust
binary on disk, two systemd units, two SQLite databases, two nginx
vhosts.

### What's per-deployment

| Concern | Davids (`.env.davids`) | Demo (`.env.demo`) |
|---|---|---|
| Domain | `davidspizzeria.de` | `rusterando.de` |
| Localhost port | `127.0.0.1:3001` | `127.0.0.1:3002` |
| Binary name on disk | `davidspizzeria-server` | `rusterando-server` |
| systemd unit | `davidspizzeria-server.service` | `rusterando-server.service` |
| Install root | `/var/www/davidspizzeria.de/` | `/var/www/rusterando.de/` |
| SQLite DB | `data/davidspizzeria.db` | `data/rusterando.db` |
| Photo uploads | `html/img/uploads/` (per-install) | `html/img/uploads/` (per-install) |
| Branding seed | `data/branding.davids.sql` | `data/branding.demo.sql` |
| Backups dir | `…/backups/davidspizzeria_*.zip` | `…/backups/rusterando_*.zip` |
| iOS app | yes (TestFlight, APNs) | no |

### What's shared

- **The same source-tree checkout.** One `git pull` updates both shops.
- **The same compiled binary.** `cargo leptos build` runs once; deploy
  rsyncs the same `target/.../rusterando-server` to two server paths
  under two filenames.
- **The same migrations.** `sqlx::migrate!()` runs at server boot
  against each DB independently. Old, applied migrations are immutable
  (sqlx checksums) so the migration timeline is identical across shops.

### Adding a third shop

```bash
cp .env.demo.example .env.<shop>
$EDITOR .env.<shop>     # set APP_NAME, LEPTOS_OUTPUT_NAME,
                        # LEPTOS_SITE_ADDR=127.0.0.1:3003,
                        # DEPLOY_REMOTE_BASE, SSH_HOST,
                        # ADMIN_PASSWORD, etc.

# Optional per-shop branding seed:
$EDITOR data/branding.<shop>.sql   # contact details, zone names

# Server-side: create the install dir + nginx vhost (one-time)
ssh root@<host> "sudo mkdir -p /var/www/<shop>.com/{html,data,backups}"
# add /etc/nginx/sites-available/<shop>.com pointing at port 3003
sudo ln -s /etc/nginx/sites-available/<shop>.com /etc/nginx/sites-enabled/
sudo certbot --nginx -d <shop>.com
sudo nginx -t && sudo systemctl reload nginx

# First-time deploy:
./scripts/deploy_to_server.sh -e <shop> setup
./scripts/deploy_to_server.sh -e <shop> full
```

Subsequent deploys are `./scripts/deploy_to_server.sh -e <shop> full`.

### Why this shape

- **No multi-tenancy in the binary.** Rusterando is single-tenant per
  process — one DB, one set of admin passwords, one set of APNs creds.
  Multiple shops = multiple processes. Simpler than path-based or
  subdomain-based multi-tenancy, easier to reason about, and a shop's
  data is genuinely isolated at the OS level (different DB files,
  different systemd units that can't see each other's memory).
- **No deploy overlays.** Earlier drafts of the deploy script had
  per-shop "overlay" branches that rewrote `Cargo.toml` mid-build.
  That's gone. The single source of truth is the `.env.<profile>`
  file: it picks the binary name, the JS bundle name, the port, the
  remote paths. Cargo always builds `rusterando-server`; the deploy
  script renames it on the wire.
- **Branding seed files (`data/*.<profile>.sql`)** carry shop-specific
  contact details (phone, address, owner) that don't belong in the
  public migrations. They run after `sqlx migrate` finishes, are
  idempotent (each `UPDATE` has a `WHERE value = '<placeholder>'`
  guard), and never enter the public repo.

### nginx vhost shape

Two real vhost files for the davidspizzeria + rusterando setup:

```nginx
# /etc/nginx/sites-available/davidspizzeria.de  (port 3001)
server {
    server_name davidspizzeria.de www.davidspizzeria.de;
    client_max_body_size 10m;

    location /pkg/ {
        alias /var/www/davidspizzeria.de/html/pkg/;
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
    # Bundled assets AND admin uploads both live under html/img/ — uploads
    # in html/img/uploads/ are written there by the app and served here
    # statically, so the data/ dir (DB included) never sits under a web
    # root. The deploy rsync excludes img/uploads/ from --delete so
    # releases don't wipe them.
    location /img/ { alias /var/www/davidspizzeria.de/html/img/; expires 30d; }
    location /.well-known/ {
        alias /var/www/davidspizzeria.de/html/.well-known/;
        types { application/json apple-app-site-association; }
        default_type application/json;
    }
    location / {
        proxy_pass http://127.0.0.1:3001;     # ← Davids' port
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        proxy_read_timeout 1h;
    }

    listen 443 ssl;
    # … ssl_certificate paths managed by certbot
}
```

The rusterando.de vhost is identical but with `proxy_pass
http://127.0.0.1:3002;` and `/var/www/rusterando.de/` paths.

## How Davids Pizzeria runs on Rusterando

This is the worked example — one of the two deployments running off
the public Rusterando source tree (the other is rusterando.de itself,
the demo install). All the contact details below are anonymised;
substitute your own.

Davids Pizzeria (<https://davidspizzeria.de>) was the codebase's first
user; the platform was generalised to AGPL once it stopped being
shop-specific. The deploy lives under the `davids` profile —
`./scripts/deploy_to_server.sh -e davids full` ships it.

### What's in `.env.davids` (shape, not values)

```
APP_NAME=davidspizzeria-server
BIN_NAME=davidspizzeria-server
LEPTOS_OUTPUT_NAME=davidspizzeria
LEPTOS_SITE_ADDR=127.0.0.1:3001
DEPLOY_REMOTE_BASE=/var/www/davidspizzeria.de
SSH_HOST=<their VPS hostname>
DATABASE_URL=sqlite:./data/davidspizzeria.db
PUBLIC_URL=https://davidspizzeria.de
ADMIN_PASSWORD=<long random string>
KITCHEN_PASSWORD=<long random string>
DRIVER_PASSWORD=<long random string>
STRIPE_PUBLISH_KEY=pk_live_<…>
STRIPE_SECRET_KEY=sk_live_<…>
STRIPE_WEBHOOK_SECRET=whsec_<…>
SMTP_HOST=<their email host>
SMTP_USER=<their bestellung@ inbox>
SMTP_PASS=<…>
EMAIL_FROM=<bestellung@…>
APNS_TEAM_ID=<10-char Apple team id>
APNS_KEY_ID=<10-char APNs key id>
APNS_BUNDLE_ID=<reverse-DNS bundle id>
APNS_KEY_PATH=/var/www/davidspizzeria.de/.hidden/AuthKey_<…>.p8
APNS_PRODUCTION=true
ORS_API_KEY=<…>
PIZZERIA_LAT=<lat>
PIZZERIA_LON=<lon>
```

The `.env` is owned by `www-data`, mode 600, lives at
`/var/www/davidspizzeria.de/.env` and is loaded by systemd
(`EnvironmentFile=-...`) as well as by the deploy script.

### Branding (data, not code)

After first install, Davids ran through `/admin/settings` and
`/admin/home` to fill in their specifics: shop name, German-language
addresses, phone, email, hero photo, two offer cards (Pizzablech + Pizza
36 cm), three gallery photos (storefront, counter, baking sheet), the
warm colour theme.

These all live in `app_settings`, `home_hero`, `home_offers`, and
`home_gallery`. They survive every deploy because the deploy script
preserves `data/`. They render server-side from a one-shot DB read
cached in `BrandingHandle` so SSR doesn't hit SQLite per request.

### Delivery zones

Lüdinghausen + four neighbouring villages, each with a `min_order_cents`
threshold and a `fee_cents` surcharge. The zones drive the city
auto-complete in checkout, the route optimisation when grouping orders
for one driver tour, and the "Wir liefern" section on the home page.
Free delivery kicks in above `app_settings.free_delivery_threshold_cents`
(currently set to 3500 = €35).

### iOS app

A native Swift WebView shell loads `https://davidspizzeria.de` on launch
and registers an APNs token tied to the staff role currently signed in.
Three Keychain entries hold the staff passwords; tapping the gear FAB
opens a native Settings sheet to switch role with one tap. Universal
Links route deep links from push notifications (`deep_link` payload key)
into the WebView so a "neue Bestellung" ping opens straight to
`/admin/orders/<id>`.

The bundle id (`<reverse-DNS>`), team id, and AASA file are tied to
Davids' Apple Developer account — anyone forking would replace them with
their own. The `ios-app/` source is otherwise generic.

### Push trigger flow

```
Customer places order
  → Stripe webhook (or place_order for cash) hits the server
  → AppState.apns is Some(ApnsHandle) (because APNS_KEY_PATH is set)
  → notify_roles(["kitchen", "admin", "driver"], …) fans out via HTTP/2
  → Each registered staff device buzzes within ~200 ms
```

Driver only gets pinged for delivery orders, not pickups. `/admin/broadcast`
exposes a manual one-shot push to staff or "all" devices, with the
audience choice persisted in `push_broadcasts` for audit.

### Backup discipline

`./scripts/deploy_to_server.sh -e davids backup` zips the binary,
html bundle, `.env`, and the SQLite DB into
`/var/www/davidspizzeria.de/backups/` named
`davidspizzeria_YYYYMMDD_HHMMSS.zip`. Pre-deploy this happens
automatically; manual backups are 30 seconds.

`./scripts/deploy_to_server.sh -e davids restore` shows a numbered
list of backups and rolls back interactively — service stop, unzip,
restore, restart. The pre-restore state is itself backed up first, so
you can always undo the undo. Each deployment's backups are
namespaced by `APP_NAME` prefix, so `davids` and `demo` backups
never collide.

### What we'd do differently

- Postgres-or-SQLite as a config switch from the start. SQLite has been
  more than enough for one shop, but a chain probably wants it.
- Stricter separation of "tenant content" (which today lives in
  `app_settings` and `home_*`) from "platform data" (orders, menu).
  A future "instance dump" feature would `pg_dump`-equivalent only the
  tenant rows.
- Generated CSS (Tailwind or Lightning CSS) instead of hand-rolled SCSS.
  Wasn't worth it for two themes; will be for ten.

## Roadmap

- [x] **/admin/zones** — delivery zones are now edited in the UI (was
  seeded by hand).
- [x] **Multi-locale** — 8 locales built in (`--features i18n`), runtime
  `i18n_enabled` toggle. Remaining: translate the last long-form legal
  strings and the email templates.
- [x] **PDF cover + theme libraries** — switchable covers and Typst
  templates from `/admin/pdf`.
- [x] **Live order tracking** — per-order SSE with delivery acks.
- [ ] **FCM / Android app**: parallel to APNs/iOS. Server-side
  `MultiPushSink` will dispatch by `platform` column.
- [ ] **WebSocket channel** to replace the 30 s shop-status poll fallback
  and back the driver-location board.
- [ ] **Cargo workspace split**: peel the kitchen and driver views out
  into optional features so a takeaway-only shop has a smaller binary.
- [ ] **Postgres support** behind a feature flag.
- [ ] **OpenAPI / proper API docs** for shops that want to integrate
  their own POS.

## Contributing

Issues and PRs welcome at <https://github.com/holg/rusterando>. See
`CONTRIBUTING.md` for the build + test loop and the code style.
The license is **AGPL-3.0-or-later**: any modified version that you
let users interact with over the network must publish its source.

If you fork Rusterando to run a real shop, drop a PR or an issue with
your shop's URL — it'd be nice to keep an example list in this README.

## License

```
Copyright (C) 2026  Rusterando contributors

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.
```

Full text: <https://www.gnu.org/licenses/agpl-3.0.html>
