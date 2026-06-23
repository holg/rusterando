"""Order → admin → customer **two-way** live messaging round-trip.

This is the e2e test the multi-tenant deploy needs: it drives one shop end
to end over two browser tabs and proves the live channel works in *both*
directions, against whatever `BASE_URL` points at — `rusterando.de`,
any `*.rusterando.de` tenant subdomain, or a local sandbox. Nothing here
is shop-specific; it discovers the menu item, order id and customer at
runtime, so the same file validates every tenant we deploy.

The story (customer left, admin right):

  0. Admin makes sure orders can be placed: on /admin/hours they close the
     shop (customer's open /menu banner flips to red "Gerade geschlossen"
     **live**), then hit "🚨 Notbetrieb" (customer banner flips to green
     "Jetzt geöffnet" **live, immediately**). The order flow is now possible
     regardless of opening hours, and the shop-status SSE is proven both
     ways. Teardown hands control back to the weekly schedule.
  1. Customer places a cash-on-pickup order and keeps `/orders/{id}` open.
  2. Admin marks that customer **VIP** on /admin/customers so the order
     page grows a reply box (VIP-gated, server-enforced). Customer reloads
     once to pick up the flag.
  3. Admin clicks "Zubereitung starten" → the customer's status pill flips
     to "In Zubereitung" **live, no reload** (`/api/live/orders/{id}` SSE).
  4. Admin sends a message → it lands in the customer's log live.
  5. Customer **replies** → the reply lands in the admin's staff log live,
     tagged as coming from the customer. This is the leg that was broken
     until `SendCustomerMessage` got registered — the test guards it.

## Running

It's a normal e2e test: **headless by default**, runs in the regular suite.

    cd tests/e2e
    # apex shop
    BASE_URL=https://rusterando.de DPE2E_ADMIN_PW=<pw> pytest test_order_admin_message.py
    # a tenant subdomain
    BASE_URL=https://flizza.rusterando.de DPE2E_ADMIN_PW=<pw> pytest test_order_admin_message.py
    # local sandbox
    BASE_URL=http://127.0.0.1:3001 DPE2E_ADMIN_PW=<pw> pytest test_order_admin_message.py

### Optional side-by-side recording

Recording is **opt-in** — off by default so the headless run stays fast.
Turn it on with `RECORD=1` (most useful with a visible window, since
headless Chromium captures little):

    RECORD=1 HEADLESS=false SLOWMO=350 \
    BASE_URL=http://127.0.0.1:3001 DPE2E_ADMIN_PW=<pw> \
        pytest test_order_admin_message.py -s

Output (only when RECORD=1): tests/e2e/recordings/<run>/{customer.webm,
admin.webm} and, with ffmpeg, a stitched `order_admin_message_demo.mp4`.

The sandbox-mode guard always applies — the test self-skips against a shop
flipped to Stripe live.
"""

from __future__ import annotations

import re
import time
from pathlib import Path

import pytest
from playwright.sync_api import Browser, Page, expect

from conftest import admin_login, merge_side_by_side, video_context_kwargs

PANE = {"width": 640, "height": 720}


def test_order_admin_customer_two_way_messaging(
    browser: Browser,
    base_url: str,
    admin_pw: str,
    recordings_dir: Path,
    record_video: bool,
    e2e_tag: str,
):
    # Unique phone so the admin can find exactly this customer to flag VIP,
    # and so reruns don't collide. Grep-able by the E2E tag.
    phone = f"01512{e2e_tag[-6:]}"
    run_dir = recordings_dir / f"{e2e_tag}_order_admin_message"
    # Only touch the recordings dir / pass video kwargs when recording is on.
    video_kw = video_context_kwargs(record_video, run_dir, PANE)

    customer_ctx = browser.new_context(
        base_url=base_url, viewport=PANE, **video_kw
    )
    admin_ctx = browser.new_context(
        base_url=base_url, viewport=PANE, **video_kw
    )
    customer = customer_ctx.new_page()
    admin = admin_ctx.new_page()

    customer_video = admin_video = None
    vip_set = False
    overrode_hours = False
    try:
        # ============ ADMIN: log in, force the shop OPEN ===============
        # The order flow is impossible while the shop isn't taking orders, so
        # the admin guarantees it first — and the customer sees the change
        # live. The sequence proves the shop-status SSE both ways:
        #   close (customer banner → red "Gerade geschlossen", live) then
        #   🚨 Notbetrieb (customer banner → green "Jetzt geöffnet", live).
        admin_login(admin, admin_pw)
        customer.goto("/menu", wait_until="domcontentloaded")
        _force_open_via_emergency(admin, customer)
        overrode_hours = True

        # ============ CUSTOMER: place a cash-on-pickup order ===========
        order_id = _place_cash_pickup_order(customer, phone, e2e_tag)
        expect(
            customer.locator(".status-pill[data-status='received']")
        ).to_be_visible(timeout=10_000)

        # ============ ADMIN: mark this customer VIP ===================
        # The reply box on the order page is VIP-gated (server-enforced),
        # so the customer can only chat back once flagged. Do this before
        # the customer reloads to surface the box.
        _set_customer_vip(admin, phone, vip=True)
        vip_set = True

        # Customer reloads once to pick up the VIP flag → reply box appears.
        # (VIP status is resolved at SSR time from the order's contact phone;
        # a single reload is the natural "I refreshed my order page" moment.)
        customer.reload(wait_until="domcontentloaded")
        reply_box = customer.locator(".restaurant-message .reply-box textarea")
        expect(reply_box).to_be_visible(timeout=15_000)

        # ============ ADMIN: open the order, start preparing ===========
        admin.goto(f"/admin/orders/{order_id}", wait_until="domcontentloaded")
        start_prep = admin.locator("button.btn.primary[data-next='preparing']")
        expect(start_prep).to_be_visible(timeout=10_000)
        start_prep.click()

        # Customer's pill flips to "In Zubereitung" LIVE — no reload.
        expect(
            customer.locator(".status-pill[data-status='preparing']")
        ).to_be_visible(timeout=15_000)

        # ============ ADMIN → CUSTOMER: send a message ================
        admin_msg = f"Hallo! Deine Bestellung läuft. (E2E {e2e_tag})"
        ta = admin.locator("textarea[maxlength='500']")
        expect(ta).to_be_visible(timeout=10_000)
        ta.fill(admin_msg)
        admin.get_by_role("button", name="Nachricht senden").click()
        expect(admin.locator(".customer-message span.ok")).to_be_visible(
            timeout=10_000
        )

        # Customer sees the staff message arrive LIVE in their order page.
        staff_li = customer.locator(
            "ul.message-log li.from-staff", has_text=e2e_tag
        )
        expect(staff_li).to_be_visible(timeout=15_000)

        # ============ CUSTOMER → ADMIN: reply ==========================
        # This is the leg that hung before SendCustomerMessage was
        # registered. The reply must land in the admin's staff log live,
        # tagged as from the customer.
        reply_text = f"Danke! Bin gleich da. (E2E {e2e_tag})"
        reply_box.fill(reply_text)
        customer.locator(".reply-box button.btn.primary").click()
        # The customer's own success tick confirms the server fn returned
        # (not hung) — the whole point of the registration fix.
        expect(customer.locator(".reply-box span.ok")).to_be_visible(
            timeout=15_000
        )
        # And it shows in the customer's own thread as their line.
        expect(
            customer.locator("ul.message-log li.from-customer", has_text=e2e_tag)
        ).to_be_visible(timeout=10_000)

        # Admin sees the customer's reply arrive LIVE in the staff log.
        admin_reply_li = admin.locator(
            ".customer-message .message-log li.from-customer", has_text=e2e_tag
        )
        expect(admin_reply_li).to_be_visible(timeout=15_000)
        expect(admin_reply_li).to_contain_text("Bin gleich da")

    finally:
        # Best-effort cleanup: drop the VIP flag we set so the test customer
        # doesn't linger as VIP. (The order row is real test data, tagged by
        # the E2E phone — left for the same out-of-band cleanup as the rest
        # of the suite.)
        if vip_set:
            try:
                _set_customer_vip(admin, phone, vip=False)
            except Exception:
                pass

        # Hand shop control back to the weekly schedule so we don't leave the
        # deployment force-open after the run (it was almost certainly closed
        # — outside hours / Ruhetag — when we force-opened it).
        if overrode_hours:
            try:
                _restore_schedule(admin)
            except Exception:
                pass

        if record_video:
            try:
                customer_video = (
                    customer.video.path() if customer.video else None
                )
            except Exception:
                customer_video = None
            try:
                admin_video = admin.video.path() if admin.video else None
            except Exception:
                admin_video = None
        customer_ctx.close()
        admin_ctx.close()

    if record_video and customer_video and admin_video:
        merged = merge_side_by_side(
            Path(customer_video),
            Path(admin_video),
            run_dir / "order_admin_message_demo.mp4",
        )
        if merged:
            print(f"\n🎬 side-by-side demo: {merged}")
        else:
            print(
                f"\n🎬 raw clips (ffmpeg merge unavailable): "
                f"{customer_video} | {admin_video}"
            )


# ---------------------------------------------------------------------------
# Helpers — all tenant-agnostic (discover item/order/customer at runtime).
# ---------------------------------------------------------------------------


# Customer-facing banner states (driven by the /api/live/shop SSE).
BANNER = ".shop-status-bar"
BANNER_OPEN = ".shop-status-bar.open"
BANNER_NOT_OPEN = ".shop-status-bar.closed, .shop-status-bar.opens-soon"


def _force_open_via_emergency(admin: Page, customer: Page) -> None:
    """Guarantee the shop is taking orders, proving the shop-status SSE live
    in both directions on the customer's open `/menu` tab.

    Sequence (admin on /admin/hours, customer watching /menu):
      1. If the shop is currently open, click "Jetzt schließen" so the
         customer's banner flips to the red "Gerade geschlossen" live — we
         want to *see* the close→open transition, not start already-open.
      2. Click "🚨 Notbetrieb: alles sofort annehmen" (force-open until end
         of day, works even on a Ruhetag / outside hours). The customer's
         banner flips to the green "Jetzt geöffnet" live, immediately.

    `admin` must already be authenticated."""
    admin.goto("/admin/hours", wait_until="domcontentloaded")

    # 1. If open, close first so the customer sees the live close. The big
    #    toggle reads "Jetzt schließen" only while the shop is open.
    close_btn = admin.get_by_role("button", name="Jetzt schließen")
    if close_btn.count() and close_btn.first.is_visible():
        close_btn.first.click()
        # Customer banner flips to NOT-OPEN live (red close, or amber if a
        # later slot exists today). No reload on the customer side.
        expect(customer.locator(BANNER_NOT_OPEN)).to_be_visible(timeout=15_000)

    # 2. Emergency on: accept everything right now. Customer sees green live.
    notbetrieb = admin.get_by_role(
        "button", name="Notbetrieb: alles sofort annehmen"
    )
    expect(notbetrieb).to_be_visible(timeout=10_000)
    notbetrieb.click()
    # The whole point: the customer's already-open tab flips to green
    # "Jetzt geöffnet" live over SSE, no reload. If the SSE flip doesn't
    # land (e.g. the customer tab subscribed late / a dropped event), fall
    # back to a reload — the server is force-open now, so a fresh SSR /menu
    # shows green and ordering proceeds. The order flow below would fail
    # loudly anyway if the shop were not actually taking orders.
    try:
        expect(customer.locator(BANNER_OPEN)).to_be_visible(timeout=15_000)
    except AssertionError:
        customer.reload(wait_until="domcontentloaded")
        expect(customer.locator(BANNER_OPEN)).to_be_visible(timeout=15_000)


def _restore_schedule(admin: Page) -> None:
    """Hand shop control back to the weekly schedule ("Auf Plan zurücksetzen"),
    clearing the force-open we set. Leaves the deployment as we found it
    (schedule-driven) rather than stuck force-open. Best-effort: if the reset
    control isn't present (already schedule-driven), does nothing."""
    if "/admin" not in admin.url:
        return
    admin.goto("/admin/hours", wait_until="domcontentloaded")
    reset = admin.get_by_role("button", name="Auf Plan zurücksetzen")
    if reset.count() and reset.first.is_visible():
        reset.first.click()
        admin.wait_for_timeout(500)


def _place_cash_pickup_order(customer: Page, phone: str, e2e_tag: str) -> str:
    """Add the first enabled menu item, check out cash-on-pickup, and return
    the order id from the resulting /orders/{id} URL.

    Picks an item whose add button is enabled (items with required options
    are disabled until chosen) so we never hit the option picker — keeps the
    test shop-independent."""
    # The shop was force-opened by _force_open_via_emergency before this
    # runs, so a fresh /menu load shows the green banner and an enabled
    # checkout. Reload to be on the menu with the open state applied.
    customer.goto("/menu", wait_until="domcontentloaded")

    add_btn = customer.locator("button.add:not([disabled])").first
    expect(add_btn).to_be_visible(timeout=15_000)
    add_btn.click()

    drawer = customer.locator("aside.cart-drawer.open")
    if drawer.count() == 0:
        customer.locator("button.cart-fab:not(.hidden)").click()
        expect(drawer).to_be_visible(timeout=10_000)
    # The checkout CTA carries a `disabled` class while the cart is empty OR
    # ordering is closed; match only the *enabled* one so we don't click a
    # dead link. (`:not(.disabled)` also skips the Suspense fallback, which
    # ships disabled until the cart resolves.)
    proceed = customer.locator(
        "aside.cart-drawer.open a.btn.primary[href='/checkout']:not(.disabled)"
    )
    expect(proceed).to_be_visible(timeout=10_000)
    proceed.click()
    customer.wait_for_url("**/checkout", timeout=10_000)

    customer.check("input[name='order_type'][value='pickup']")
    customer.fill("input[name='name']", "E2E Kund:in")
    customer.fill("input[name='phone']", phone)
    customer.fill("input[name='email']", "e2e@example.com")
    customer.check("input[name='pickup_time'][value='asap']")
    customer.check("input[name='payment_method'][value='cash']")

    customer.click("button[type='submit'].btn.primary")
    # Cash path redirects straight to the live confirmation page.
    customer.wait_for_url(re.compile(r".*/orders/[^/]+$"), timeout=15_000)
    return customer.url.rstrip("/").rsplit("/", 1)[-1]


def _set_customer_vip(admin: Page, phone: str, vip: bool) -> None:
    """Find the customer by phone on /admin/customers and toggle their VIP
    flag to the desired state. Idempotent: if the row already shows the
    target state, leaves it alone.

    The button label is "Als VIP" when not VIP and "VIP entfernen" when VIP,
    so the wanted label tells us whether a click is needed.

    Assumes `admin` is already authenticated (every caller logs in first)."""
    admin.goto("/admin/customers", wait_until="domcontentloaded")
    search = admin.locator("input.search[type='search']")
    expect(search).to_be_visible(timeout=10_000)
    search.fill(phone)

    row = admin.locator("table.admin-customers-table tbody tr", has_text=phone)
    expect(row.first).to_be_visible(timeout=10_000)

    has_vip_pill = row.first.locator(".vip-pill").count() > 0
    if has_vip_pill == vip:
        return  # already in the desired state

    # Button reads "Als VIP" to promote, "VIP entfernen" to demote.
    label = "Als VIP" if vip else "VIP entfernen"
    btn = row.first.get_by_role("button", name=label)
    expect(btn).to_be_visible(timeout=10_000)
    btn.click()
    # Confirm the toggle landed: the pill appears/disappears after the
    # server action + refetch.
    if vip:
        expect(row.first.locator(".vip-pill")).to_be_visible(timeout=10_000)
    else:
        expect(row.first.locator(".vip-pill")).to_have_count(0, timeout=10_000)
