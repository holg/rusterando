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
