"""Live auth-status heartbeat on the staff [Abmelden] control.

The shell's logout control carries a chip that heartbeats `current_role()`
(every 30s + on tab focus, with an 8s timeout) so staff see the TRUE
server-side session state instead of trusting a cookie the server may have
already expired. This is the fix for "the client trusts the cookie, the
server says it's gone, and the page hangs forever".

Two things proved here, both tenant-agnostic (any `BASE_URL`):

  1. Signed in → the chip is green and reads the role ("Admin").
  2. Cookie cleared underneath the open tab (simulating a server-side
     expiry / stale cookie) → on the next heartbeat the chip flips to the
     red "Sitzung abgelaufen" and becomes a link back to the login page —
     no hang, no reload needed.

We force the heartbeat to fire promptly by dispatching a `visibilitychange`
event (the shell re-checks on focus) instead of waiting out the 30s poll.

Headless by default like the rest of the suite. Runs in the normal `pytest`.

    cd tests/e2e
    BASE_URL=https://rusterando.de DPE2E_ADMIN_PW=<pw> pytest test_auth_status.py
"""

from __future__ import annotations

import pytest
from playwright.sync_api import Page, expect

# These tests only log in and read current_role() — no data created, no
# Stripe touched — so they're safe (and useful) against a live shop too.
pytestmark = pytest.mark.no_sandbox_guard


def test_logout_chip_shows_role_when_signed_in(admin_page: Page):
    """A fresh admin session shows the green chip reading 'Admin'."""
    chip = admin_page.locator(".auth-control .auth-chip")
    # The heartbeat starts at "…" (checking) then settles to authed within
    # its 8s timeout. Wait for the authed state specifically.
    expect(admin_page.locator(".auth-chip.authed")).to_be_visible(timeout=15_000)
    expect(chip).to_contain_text("Admin")


def test_logout_chip_flips_to_expired_when_cookie_cleared(
    admin_page: Page, base_url: str
):
    """Clear the session cookies under the open tab, then nudge the heartbeat
    (a visibilitychange, which the shell re-checks on) — the chip must flip to
    the red 'Sitzung abgelaufen' link instead of trusting the stale cookie."""
    # Start signed in (green chip).
    expect(admin_page.locator(".auth-chip.authed")).to_be_visible(timeout=15_000)

    # Simulate a server-side expiry: drop the session cookies the browser
    # still holds. The page DOM is untouched — exactly the stale-cookie case.
    ctx = admin_page.context
    remaining = [
        c for c in ctx.cookies()
        if c["name"] not in ("dp_session", "admin_session")
    ]
    ctx.clear_cookies()
    if remaining:
        ctx.add_cookies(remaining)

    # Nudge the heartbeat: the shell re-checks current_role() on
    # visibilitychange (the "came back to the tab" path). This avoids waiting
    # out the 30s poll and proves the focus-driven recheck works.
    admin_page.evaluate(
        "document.dispatchEvent(new Event('visibilitychange'))"
    )

    # The chip flips to the red expired state and becomes a login link.
    expired = admin_page.locator("a.auth-chip.expired")
    expect(expired).to_be_visible(timeout=15_000)
    expect(expired).to_contain_text("abgelaufen")
    # It points back at the admin login so re-auth is one tap.
    expect(expired).to_have_attribute("href", "/admin/login")
