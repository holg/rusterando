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

## Safety

Every test self-skips when the target is in Stripe **live** mode. This
protects the real Buchhaltung from accidental test data. Flip
`/admin/settings` → Stripe Modus → Sandbox before running.

Test-created data uses an `E2E<unix-ts>` prefix on voucher codes and
`00000<ts>` for phone numbers, so leftover rows are grep-able. The
voucher test deactivates its own code at the end; phones can be cleaned
manually via `/admin/customers` if they ever appear (they only would if
a checkout completed, which the suite doesn't do).
