"""Regression test for the admin top-nav hydration bug.

History: AdminShell used to read BrandingHandle inside a
`#[cfg(feature = "ssr")]` block, falling back to a hardcoded string
on the hydrate side. SSR rendered "Davids Pizzeria — Admin", hydrate
expected "Mein Restaurant — Admin", tachys' walker aborted at the
brand <a>, and every link in the top nav stayed unbound until a
full-page reload re-built the tree from scratch.

This test loads /admin cold (no prior navigation), then clicks one of
the late-in-the-list nav links ("Liefergebiete") directly. If the
click triggers a real client-side route change to /admin/zones, the
hydration walker reached that <a> — i.e. the bug is gone. If the
link is still un-hydrated, Playwright will hit a hard timeout waiting
for the URL change (the raw <a> would fire a full reload, but only
after the browser actually receives the event, which we also check).
"""

from __future__ import annotations

from playwright.sync_api import expect


def test_admin_top_nav_clickable_on_cold_load(admin_page):
    """First click after cold /admin load must navigate. No workaround."""
    admin_page.goto("/admin", wait_until="domcontentloaded")

    # Wait until the shell is interactive. Brand text resolves via a
    # server fn — once it's there, hydration of that subtree has run
    # (or has at least had a chance to). The exact text isn't asserted
    # because the shop name is configurable.
    brand = admin_page.locator(".admin-shell-bar .brand")
    expect(brand).to_be_visible(timeout=10_000)
    expect(brand).not_to_have_text("Admin")  # past the fallback

    # Click a non-trivial link (not "Übersicht", which would no-op).
    # Liefergebiete sits late in the nav; if it works the whole walker
    # reached it.
    admin_page.locator(".admin-nav a[href='/admin/zones']").click()
    admin_page.wait_for_url("**/admin/zones", timeout=5_000)

    # And we land on the zones page proper.
    expect(admin_page.locator(".admin-zones h1")).to_have_text("Liefergebiete")


def test_admin_pdf_nav_clickable_on_cold_load(admin_page):
    """Same shape, different link — guards against partial-hydration
    bugs where only the *first* nav link works."""
    admin_page.goto("/admin", wait_until="domcontentloaded")
    expect(admin_page.locator(".admin-shell-bar")).to_be_visible(timeout=10_000)

    admin_page.locator(".admin-nav a[href='/admin/pdf']").click()
    admin_page.wait_for_url("**/admin/pdf", timeout=5_000)
    expect(admin_page.locator(".admin-pdf h1")).to_have_text("PDF-Editor")
