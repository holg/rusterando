"""Two-browser live-demo scenario — also a recording.

What it proves (and shows on camera): the admin pauses online ordering
for an hour in one browser, and the customer's *already-open* browser
flips from "🟢 Jetzt geöffnet" to the amber "🟡 Gerade geschlossen"
(a 1 h snooze is a temporary pause) within ~1 s over the
`/api/live/shop` SSE channel — no reload — and can then no longer place
an order.

Run it (visible + recorded), against a LOCAL sandbox server:

    cd tests/e2e
    BASE_URL=http://127.0.0.1:3001 \
    DPE2E_ADMIN_PW=<your local admin pw> \
    HEADLESS=false SLOWMO=400 \
    pytest -m demo test_live_demo.py

Output: tests/e2e/recordings/<run>/{customer.webm,admin.webm} and, if
ffmpeg is on PATH, a stitched side-by-side `live_pause_demo.mp4`.

This is gated behind `-m demo` (deselected from the normal run) because
it needs a reachable server and is most useful with visible browsers.
The sandbox-mode guard still applies — it self-skips against a live shop.
"""

from __future__ import annotations

import time
from pathlib import Path

import pytest
from playwright.sync_api import Browser, expect

from conftest import admin_login, merge_side_by_side

# Customer banner (home + menu, fed by the /api/live/shop SSE).
BANNER = ".shop-status-bar"
BANNER_OPEN = ".shop-status-bar.open"
# A timed snooze is a *temporary* closure → amber "opens-soon", not red
# "closed" (red is reserved for a hard close / Ruhetag). The demo snoozes
# for 1 h, so the banner that appears is the amber one.
BANNER_NOT_OPEN = ".shop-status-bar.opens-soon, .shop-status-bar.closed"
BANNER_PAUSED = ".shop-status-bar.opens-soon"
# Checkout SSR banner shown when ordering is disabled.
CHECKOUT_CLOSED = ".order-closed-banner"

# Side-by-side video layout: each pane is 720px tall (matched in the
# ffmpeg merge), 640 wide so two fit a 1280-wide frame.
PANE = {"width": 640, "height": 720}


@pytest.mark.demo
def test_admin_pause_flips_customer_live(
    browser: Browser,
    base_url: str,
    admin_pw: str,
    recordings_dir: Path,
    e2e_tag: str,
):
    run_dir = recordings_dir / f"{e2e_tag}_live_pause"
    run_dir.mkdir(parents=True, exist_ok=True)

    # Two independent contexts = two browsers on screen = two videos.
    # Customer on the left, admin on the right.
    customer_ctx = browser.new_context(
        base_url=base_url,
        viewport=PANE,
        record_video_dir=str(run_dir),
        record_video_size=PANE,
    )
    admin_ctx = browser.new_context(
        base_url=base_url,
        viewport=PANE,
        record_video_dir=str(run_dir),
        record_video_size=PANE,
    )
    customer = customer_ctx.new_page()
    admin = admin_ctx.new_page()

    customer_video = None
    admin_video = None
    restored = False
    try:
        # --- 1. Customer is browsing the menu, shop shows OPEN ---------
        customer.goto("/menu", wait_until="domcontentloaded")
        # The banner hydrates + then the SSE Effect attaches. The shop is
        # open at this point, so we expect the green bar. If a previous
        # run left it paused, restore first so the demo starts clean.
        try:
            expect(customer.locator(BANNER_OPEN)).to_be_visible(timeout=10_000)
        except AssertionError:
            _restore_open(admin, admin_pw)
            customer.reload(wait_until="domcontentloaded")
            expect(customer.locator(BANNER_OPEN)).to_be_visible(timeout=10_000)

        # --- 2. Admin logs in and snoozes ordering for 1 hour ----------
        admin_login(admin, admin_pw)
        admin.goto("/admin/hours", wait_until="domcontentloaded")
        expect(admin.get_by_role("button", name="1 Std")).to_be_visible(
            timeout=10_000
        )
        # Beat so the recording clearly shows the "before" state on both
        # panes before the click.
        time.sleep(1.0)
        admin.get_by_role("button", name="1 Std").click()

        # Admin's own page should now report the timed pause.
        expect(admin.get_by_text("automatisch wieder offen ab")).to_be_visible(
            timeout=10_000
        )

        # --- 3. Customer banner flips to NOT-OPEN *live*, no reload ----
        # This is the whole point: the customer never touched their tab.
        # The /api/live/shop SSE pushes the new ShopStatus and the banner
        # swaps green→amber in place (a 1 h snooze is a *temporary* pause,
        # so it's the amber "Gerade geschlossen" bar, not red).
        expect(customer.locator(BANNER_PAUSED)).to_be_visible(timeout=15_000)
        expect(customer.locator(BANNER_PAUSED)).to_contain_text("geschlossen")
        # Hold the closed state on camera.
        time.sleep(1.5)

        # --- 4. Customer can no longer order ---------------------------
        # The closed banner replaced the "Online-Bestellung möglich" line
        # live; the open banner is gone. Belt-and-braces, confirm a fresh
        # /checkout load is server-gated too — place_order rejects while
        # closed, and the SSR checkout reflects it. (Empty cart shows the
        # cart-empty page, so we assert the menu banner — the customer's
        # actual on-screen state — rather than a cart-dependent element.)
        expect(customer.locator(BANNER_OPEN)).to_have_count(0)
        time.sleep(1.5)

        # --- 5. Restore: admin reopens; customer banner flips back -----
        _restore_open(admin, admin_pw)
        restored = True
        # Show the live reopen too (back on the menu where the SSE lives).
        customer.goto("/menu", wait_until="domcontentloaded")
        expect(customer.locator(BANNER_OPEN)).to_be_visible(timeout=10_000)
        time.sleep(1.0)

    finally:
        # Restore even if an assertion failed mid-demo, so the local DB
        # isn't left paused for the next run / manual use.
        if not restored:
            try:
                _restore_open(admin, admin_pw)
            except Exception:
                pass

        # close_video() handles flush; grab the paths before closing ctx.
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

    # --- Merge into one side-by-side mp4 (best effort) -----------------
    if customer_video and admin_video:
        merged = merge_side_by_side(
            Path(customer_video),
            Path(admin_video),
            run_dir / "live_pause_demo.mp4",
        )
        if merged:
            print(f"\n🎬 side-by-side demo: {merged}")
        else:
            print(
                f"\n🎬 raw clips (ffmpeg merge unavailable): "
                f"{customer_video} | {admin_video}"
            )


def _restore_open(admin, admin_pw: str) -> None:
    """Click 'Jetzt öffnen' on /admin/hours so the shop is taking orders.

    `set_orders_open(true)` clears any pause/snooze/force state, so this
    is the single canonical 'reset to open' action. Logs in first if the
    admin context isn't authenticated yet (e.g. teardown after an early
    failure)."""
    if "/admin" not in admin.url:
        admin_login(admin, admin_pw)
    admin.goto("/admin/hours", wait_until="domcontentloaded")
    # The big toggle reads "Jetzt öffnen" whenever the shop is not
    # currently taking orders (paused, snoozed, or schedule-closed).
    open_btn = admin.get_by_role("button", name="Jetzt öffnen")
    if open_btn.count() > 0 and open_btn.first.is_visible():
        open_btn.first.click()
        # Wait for the toggle to settle to the open label.
        expect(admin.get_by_role("button", name="Jetzt schließen")).to_be_visible(
            timeout=10_000
        )
