"""Full cash-on-pickup order lifecycle — two-browser demo + recording.

The story (customer left, admin right):

  1. Customer browses the menu, adds an item, opens the cart, checks out
     with **cash on pickup**, and lands on the live `/orders/{id}` page —
     and *keeps that tab open*.
  2. A new order pops on the admin board; the admin opens it and clicks
     "Zubereitung starten". The customer's status pill flips to
     "In Zubereitung" **live, no reload**.
  3. While the order is being prepared, the admin rewards the customer:
     creates a 10 %-off voucher on /admin/vouchers, then goes back to the
     order and sends it as a message — "Danke für die Bestellung! Hier ein
     Geschenk: 10 % mit Code …". It appears in the customer's tab live.
  4. Only THEN does the admin mark the order "Abholbereit" — and again the
     customer's pill updates live.

Gated behind `-m demo`; run it visible + recorded against a LOCAL sandbox:

    cd tests/e2e
    BASE_URL=http://127.0.0.1:3001 \
    DPE2E_ADMIN_PW=<local admin pw> \
    HEADLESS=false SLOWMO=350 \
    pytest -m demo test_order_demo.py -s

Output: tests/e2e/recordings/<run>/{customer.webm,admin.webm} and, with
ffmpeg, a stitched side-by-side `order_lifecycle_demo.mp4`.
"""

from __future__ import annotations

import re
import time
from pathlib import Path

import pytest
from playwright.sync_api import Browser, expect

from conftest import admin_login, merge_side_by_side

PANE = {"width": 640, "height": 720}

# The gift code is unique per run so reruns don't collide on the unique
# constraint, and leftover codes are grep-able (same convention as the
# voucher test's E2E-prefix).
GIFT_PREFIX = "GIFT"


@pytest.mark.demo
def test_cash_pickup_order_lifecycle(
    browser: Browser,
    base_url: str,
    admin_pw: str,
    recordings_dir: Path,
    e2e_tag: str,
):
    gift_code = f"{GIFT_PREFIX}{e2e_tag[-6:]}"  # e.g. GIFT913422
    run_dir = recordings_dir / f"{e2e_tag}_order_lifecycle"
    run_dir.mkdir(parents=True, exist_ok=True)

    customer_ctx = browser.new_context(
        base_url=base_url, viewport=PANE,
        record_video_dir=str(run_dir), record_video_size=PANE,
    )
    admin_ctx = browser.new_context(
        base_url=base_url, viewport=PANE,
        record_video_dir=str(run_dir), record_video_size=PANE,
    )
    customer = customer_ctx.new_page()
    admin = admin_ctx.new_page()

    customer_video = admin_video = None
    try:
        # ============ CUSTOMER: place a cash-on-pickup order ===========
        customer.goto("/menu", wait_until="domcontentloaded")

        # Add the first *enabled* item (an item with required options has a
        # disabled add button until they're chosen; picking an enabled one
        # avoids the picker). Each size is its own `button.add`.
        add_btn = customer.locator("button.add:not([disabled])").first
        expect(add_btn).to_be_visible(timeout=15_000)
        time.sleep(0.8)  # let the "before" state read on camera
        add_btn.click()

        # Adding a line auto-opens the cart drawer. If for some reason it
        # didn't, click the FAB to open it. Then hit "Zur Kasse".
        drawer = customer.locator("aside.cart-drawer.open")
        if drawer.count() == 0:
            customer.locator("button.cart-fab:not(.hidden)").click()
            expect(drawer).to_be_visible(timeout=10_000)
        proceed = customer.locator(
            "aside.cart-drawer.open a.btn.primary[href='/checkout']"
        )
        expect(proceed).to_be_visible(timeout=10_000)
        proceed.click()
        customer.wait_for_url("**/checkout", timeout=10_000)

        # Fill the pickup + cash form. Radios default to pickup/asap/cash,
        # but we set them explicitly so the demo is unambiguous on camera.
        customer.check("input[name='order_type'][value='pickup']")
        customer.fill("input[name='name']", "Demo Kund:in")
        customer.fill("input[name='phone']", f"01512{e2e_tag[-6:]}")
        customer.fill("input[name='email']", "demo@example.com")
        customer.check("input[name='pickup_time'][value='asap']")
        customer.check("input[name='payment_method'][value='cash']")
        time.sleep(0.8)

        customer.click("button[type='submit'].btn.primary")
        # Cash path redirects straight to the live confirmation page.
        customer.wait_for_url(re.compile(r".*/orders/[^/]+$"), timeout=15_000)
        order_id = customer.url.rstrip("/").rsplit("/", 1)[-1]
        # Fresh order is "Eingegangen".
        expect(
            customer.locator(".status-pill[data-status='received']")
        ).to_be_visible(timeout=10_000)

        # ============ ADMIN: open the order, start preparing ===========
        admin_login(admin, admin_pw)
        admin.goto(f"/admin/orders/{order_id}", wait_until="domcontentloaded")
        start_prep = admin.locator("button.btn.primary[data-next='preparing']")
        expect(start_prep).to_be_visible(timeout=10_000)
        time.sleep(0.8)
        start_prep.click()

        # Customer's pill flips to "In Zubereitung" LIVE — no reload.
        expect(
            customer.locator(".status-pill[data-status='preparing']")
        ).to_be_visible(timeout=15_000)
        time.sleep(1.0)

        # ===== ADMIN: while preparing, create a 10% gift voucher =======
        admin.goto("/admin/vouchers", wait_until="domcontentloaded")
        form = admin.locator("details.create-voucher")
        expect(form).to_be_visible(timeout=10_000)
        # The <details> is open by default; click summary if it's collapsed.
        if not form.evaluate("el => el.open"):
            form.locator("summary").click()
        form.locator("input[placeholder='WILLKOMMEN10']").fill(gift_code)
        form.locator("select").select_option("percent")
        form.locator("input[type='number'][min='1'][max='100']").fill("10")
        time.sleep(0.6)
        form.get_by_role("button", name="Anlegen").click()
        # Success marker, then the code shows in the table below.
        expect(form.locator("span.ok")).to_be_visible(timeout=10_000)
        expect(
            admin.locator("table.admin-vouchers-table").get_by_text(gift_code)
        ).to_be_visible(timeout=10_000)
        time.sleep(0.8)

        # ===== ADMIN: send the gift as an order message (still preparing) ==
        admin.goto(f"/admin/orders/{order_id}", wait_until="domcontentloaded")
        gift_msg = (
            f"Danke für die Bestellung! 🎁 Als Geschenk: 10 % auf die "
            f"nächste Bestellung mit Code {gift_code}"
        )
        ta = admin.locator("textarea[maxlength='500']")
        expect(ta).to_be_visible(timeout=10_000)
        ta.fill(gift_msg)
        admin.get_by_role("button", name="Nachricht senden").click()
        expect(admin.locator(".customer-message span.ok")).to_be_visible(
            timeout=10_000
        )

        # Customer sees the gift message arrive LIVE in the order page.
        msg_li = customer.locator("ul.message-log li", has_text=gift_code)
        expect(msg_li).to_be_visible(timeout=15_000)
        expect(msg_li).to_contain_text("10 %")
        # And the admin's own gift line flips to "✓ Zugestellt" via the
        # ack (the log also holds status-change lines, so scope to the row
        # carrying the gift code).
        gift_row = admin.locator(
            ".customer-message .message-log li", has_text=gift_code
        )
        expect(gift_row.locator(".ack.delivered")).to_be_visible(
            timeout=15_000
        )
        time.sleep(1.5)

        # ============ ADMIN: NOW mark ready for pickup =================
        ready = admin.locator(
            "button.btn.primary[data-next='ready_for_pickup']"
        )
        expect(ready).to_be_visible(timeout=10_000)
        time.sleep(0.6)
        ready.click()

        # Customer's pill flips to "Abholbereit" LIVE.
        expect(
            customer.locator(".status-pill[data-status='ready_for_pickup']")
        ).to_be_visible(timeout=15_000)
        time.sleep(2.0)

    finally:
        # Best-effort cleanup: deactivate the demo voucher so reruns and
        # manual use aren't littered with live gift codes. (The order row
        # is real test data, tagged by the E2E phone — left for the same
        # out-of-band cleanup as the rest of the suite.)
        try:
            _deactivate_voucher(admin, admin_pw, gift_code)
        except Exception:
            pass

        try:
            customer_video = customer.video.path() if customer.video else None
        except Exception:
            customer_video = None
        try:
            admin_video = admin.video.path() if admin.video else None
        except Exception:
            admin_video = None
        customer_ctx.close()
        admin_ctx.close()

    if customer_video and admin_video:
        merged = merge_side_by_side(
            Path(customer_video),
            Path(admin_video),
            run_dir / "order_lifecycle_demo.mp4",
        )
        if merged:
            print(f"\n🎬 side-by-side demo: {merged}")
        else:
            print(
                f"\n🎬 raw clips (ffmpeg merge unavailable): "
                f"{customer_video} | {admin_video}"
            )


def _deactivate_voucher(admin, admin_pw: str, code: str) -> None:
    """Toggle the demo voucher off from /admin/vouchers so it can't be
    redeemed for real. Finds the table row by code and clicks its
    deactivate control."""
    if "/admin" not in admin.url:
        admin_login(admin, admin_pw)
    admin.goto("/admin/vouchers", wait_until="domcontentloaded")
    row = admin.locator("table.admin-vouchers-table tr", has_text=code)
    if row.count() == 0:
        return
    # The row's actions cell has [Bearbeiten] [Deaktivieren] [Löschen];
    # click the deactivate button by name (not just the first button).
    deact = row.first.get_by_role("button", name="Deaktivieren")
    if deact.count() > 0:
        deact.first.click()
        admin.wait_for_timeout(500)
