# davidspizzeria E2E tests

Playwright + pytest. Tests run against an already-deployed instance
(default `davidspizzeria.de`).

## Setup

Once per machine:

```bash
source ~/Documents/develeop/rust/geodb-rs/crates/geodb-py/.env_py312/bin/activate
playwright install chromium    # downloads the browser binary
```

Once per checkout:

```bash
cp tests/e2e/.env.example tests/e2e/.env
# edit .env, set DPE2E_ADMIN_PW
```

## Run

```bash
cd tests/e2e
pytest                  # against davidspizzeria.de (default)
BASE_URL=http://127.0.0.1:3001 pytest   # against local dev server
pytest test_hydration.py::test_admin_top_nav_clickable_on_cold_load  # one test
pytest --headed --slowmo 300            # watch it run
```

### Verify new code locally before deploy

Some tests assert behaviour only the *new* code has (e.g. the live
`[Abmelden]` auth-status chip). Those must run against a local build of the
current branch, not the still-old remote. One command builds + serves this
checkout and runs the auth + messaging e2e against it, then tears down:

```bash
scripts/test-auth-locally.sh                 # build, serve :3001, run, stop
scripts/test-auth-locally.sh --keep-running  # leave the server up
```

It loads `.env` for `ADMIN_PASSWORD`. The order/messaging test needs the
local shop in **Stripe sandbox** mode (the auth tests carry
`no_sandbox_guard` and run regardless). Flip a live local DB once with:

```bash
sqlite3 data/davidspizzeria.db \
  "UPDATE app_settings SET value='sandbox' WHERE key='stripe_mode';"
```

(Same one-row change the admin Sandbox toggle makes.) The companion
`scripts/test-hydration-locally.sh` does the same serve dance for the
hydration suite.

### Retargeting the suite

`BASE_URL` is the one knob that retargets the whole suite — point it at the
apex shop, any tenant subdomain, or a local sandbox. The tests discover the
menu item, order id and customer at runtime, so the same files validate
every shop we deploy:

```bash
BASE_URL=https://rusterando.de         pytest   # apex
BASE_URL=https://flizza.rusterando.de  pytest   # a *.rusterando.de tenant
```

`test_order_admin_message.py` is the headline live round-trip: customer
orders → admin marks them VIP, starts preparing (status flips live) and
messages them → **customer replies and it lands in the admin's log live**.
It's a normal headless test — runs in the default `pytest` and works
against any `BASE_URL`:

```bash
BASE_URL=https://rusterando.de DPE2E_ADMIN_PW=<pw> pytest test_order_admin_message.py
```

To watch it / capture a side-by-side video, opt in with `RECORD=1`
(see below).

### Headless + optional recording

Headless is the **hard default** — every test runs windowless unless you
ask otherwise. `HEADLESS=false` (or `--headed`) opens a window; `RECORD=1`
records two-browser tests to `tests/e2e/recordings/<run>/` (off by
default, most useful with a visible window):

```bash
RECORD=1 HEADLESS=false SLOWMO=350 \
BASE_URL=http://127.0.0.1:3001 DPE2E_ADMIN_PW=<pw> \
    pytest test_order_admin_message.py -s
```

With `ffmpeg` on `PATH` the two clips stitch into one
`order_admin_message_demo.mp4`; without it you get the raw `.webm` pair.

## Demo mode (two-browser recording)

`test_live_demo.py` is a scripted **two-browser** scenario that doubles as
a screen recording: a customer browser sits on `/menu`, the admin browser
pauses online ordering for 1 h on `/admin/hours`, and the customer's banner
flips 🟢 → 🔴 **live over SSE** with no reload — then the admin reopens.

It's gated behind the `demo` marker (deselected from the normal `pytest`
run). Run it visible + recorded, against a **local sandbox** server:

```bash
cd tests/e2e
BASE_URL=http://127.0.0.1:3001 \
DPE2E_ADMIN_PW=<local admin pw> \
HEADLESS=false SLOWMO=350 \
pytest -m demo test_live_demo.py -s
```

Output lands in `tests/e2e/recordings/<run>/` (gitignored):

- `*.webm` — one raw clip per browser (customer + admin),
- `live_pause_demo.mp4` — the two stitched **side by side** (needs
  `ffmpeg` on `PATH`; without it you just get the two `.webm` files).

The recording is meaningful only with `HEADLESS=false` (headless Chromium
captures little). `SLOWMO` adds a per-action pause so the flip is visible.

The demo seeds nothing destructive and **restores the shop to open** at
the end (even if an assertion fails mid-run). It needs the shop to have at
least one open pickup slot for the "before" state — if you run it well
outside opening hours, add a wide-open special day for today via
`/admin/hours` → Sondertage (or it self-skips the open baseline by
force-opening).

### Demos available

| File | Story | Output mp4 |
|---|---|---|
| `test_live_demo.py` | Admin pauses ordering 1 h → customer banner flips 🟢→🔴 live. | `live_pause_demo.mp4` |
| `test_order_demo.py` | Full **cash-on-pickup** order: customer orders → admin starts preparing (customer status updates live) → admin creates a 10 % gift voucher and sends it as an order message *while preparing* → customer sees the gift live → admin marks ready. | `order_lifecycle_demo.mp4` |

Run either with `pytest -m demo <file> -s` (or `pytest -m demo` for both).

> `test_order_admin_message.py` covers similar ground but is a **normal
> e2e test** (not demo-gated): it runs headless in the default `pytest`
> and only records when you pass `RECORD=1`. See above.
The order demo needs an **open pickup slot** (same opening-hours note as
above) and leaves a real test order in the DB (cash, phone `01512<ts>`);
it deactivates its own gift voucher (`GIFT<ts>`) on teardown.

## Safety

Every test self-skips when the target is in Stripe **live** mode. This
protects the real Buchhaltung from accidental test data. Flip
`/admin/settings` → Stripe Modus → Sandbox before running.

Test-created data uses an `E2E<unix-ts>` prefix on voucher codes and
`00000<ts>` for phone numbers, so leftover rows are grep-able. The
voucher test deactivates its own code at the end; phones can be cleaned
manually via `/admin/customers` if they ever appear (they only would if
a checkout completed, which the suite doesn't do).
