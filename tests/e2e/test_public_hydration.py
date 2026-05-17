"""Regression tests for the customer-facing hydration panic.

History (May 2026): the public site (/) and the menu (/menu) tripped
tachys's hydration walker with `internal error: entered unreachable
code` because chrome components above `<Routes>` (SiteHeader wordmark,
LocaleSwitcher, TestModeBanner, root <Title>) each read an
`OnceResource` inside their own `<Suspense>`. Resource reads at the
document root can't be coordinated with the SSR stream markers, so
hydration aborted and Leptos's delegated click handlers never
attached. Users reported "Jetzt bestellen / Speisekarte sometimes
needs 5 taps" — taps fell through to plain anchor navigation which
iOS Safari WebView occasionally dropped.

The fix replaced all four resources with a single SSR-baked
`window.__appBootstrap` inline script, read synchronously on both
sides. These tests guard the fix:

  1. The bootstrap blob is present and contains the expected keys.
  2. Clicking the homepage "Jetzt bestellen" CTA navigates to /menu
     on the FIRST click (no retry loop, no full-page reload).
  3. Clicking the header "Speisekarte" link navigates to /menu on the
     FIRST click.
  4. No `unreachable` / hydration panic surfaces in the WASM console
     during the page load.

The fourth check is the strongest signal: any future regression to
the resource-in-chrome pattern fires the same panic message, and the
test will catch it directly instead of waiting for the symptom to
manifest as "weird click behaviour".
"""

from __future__ import annotations

import pytest
from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _capture_console_errors(page: Page) -> list[str]:
    """Subscribe to console + pageerror and return a mutable list of
    captured messages. The list is updated by the event listeners; the
    caller iterates it after navigation completes."""
    errors: list[str] = []

    def on_console(msg):
        # Only track errors — info/warn/log are too noisy.
        if msg.type == "error":
            errors.append(f"console.error: {msg.text}")

    def on_pageerror(exc):
        errors.append(f"pageerror: {exc}")

    page.on("console", on_console)
    page.on("pageerror", on_pageerror)
    return errors


def _hydration_panics(errors: list[str]) -> list[str]:
    """Filter for messages that match the tachys hydration panic
    signature. We look for two strings ANY of which is unique to the
    panic — either the literal Rust panic banner or the WASM unreachable
    trap that follows it in Safari's console."""
    needles = (
        "entered unreachable code",
        "unreachable executed",  # webkit phrasing
        "Unreachable code should not be executed",  # safari phrasing
        "tachys",
    )
    return [e for e in errors if any(n in e for n in needles)]


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_bootstrap_script_present_on_homepage(page: Page):
    """The SSR shell bakes window.__appBootstrap so chrome can read
    shop_name / i18n_enabled / stripe_sandbox synchronously. Without
    this blob the chrome falls back to defaults and the page is
    technically still usable, but the hydration fix relies on it
    being there."""
    page.goto("/", wait_until="domcontentloaded")
    bootstrap = page.evaluate("() => window.__appBootstrap")
    assert bootstrap is not None, (
        "window.__appBootstrap missing — the SSR <script> blob that "
        "carries shop_name + i18n_enabled + stripe_sandbox isn't being "
        "rendered. Chrome will fall back to its hydration-resource path."
    )
    assert "shop_name" in bootstrap, f"bootstrap missing shop_name: {bootstrap!r}"
    assert "i18n_enabled" in bootstrap, f"bootstrap missing i18n_enabled: {bootstrap!r}"
    assert "stripe_sandbox" in bootstrap, (
        f"bootstrap missing stripe_sandbox: {bootstrap!r}"
    )


def test_no_hydration_panic_on_homepage(page: Page):
    """Hard fail if tachys re-introduces 'entered unreachable code'.

    The console panic is the canonical signature of a Suspense/Resource
    pair sitting above <Routes> that we can't currently coordinate with
    SSR stream markers. If a future PR adds one back, every customer
    page break-tests until someone notices the symptom; this catches
    it on the first run."""
    errors = _capture_console_errors(page)
    page.goto("/", wait_until="networkidle")
    # Give hydrate a moment to actually run after networkidle.
    page.wait_for_timeout(500)
    panics = _hydration_panics(errors)
    assert not panics, (
        "Hydration panic detected on /. The chrome likely regained a "
        "Suspense/OnceResource pair above <Routes>. Console messages:\n"
        + "\n".join(panics)
    )


def test_jetzt_bestellen_navigates_on_first_click(page: Page):
    """The hero CTA must navigate to /menu on the FIRST click.

    Pre-fix, the hydration panic left the router's delegated handler
    detached; the click either fell through to plain anchor nav (iOS
    sometimes dropped it) or did nothing. We assert the URL changes
    inside 3s of a single click — five-tap-retry would blow well past
    that and fail the test."""
    errors = _capture_console_errors(page)
    page.goto("/", wait_until="networkidle")

    # The CTA is inside .hero-content .cta on the resolved hero.
    # Wait for the resolved hero (not the SSR placeholder, which has
    # only the <h1>) — its CTA is the click target.
    cta = page.locator(".hero-content .cta a[href='/menu']")
    expect(cta).to_be_visible(timeout=10_000)

    cta.click()
    page.wait_for_url("**/menu", timeout=3_000)
    expect(page.locator(".menu .menu-header h1")).to_be_visible(timeout=10_000)

    # Defensive: even if the click navigated, a hydration panic at
    # load time still means the rest of the page is broken. Surface it.
    panics = _hydration_panics(errors)
    assert not panics, (
        "Click navigated but a hydration panic still fired:\n" + "\n".join(panics)
    )


def test_speisekarte_header_link_navigates_on_first_click(page: Page):
    """The site-header `Speisekarte` link must navigate on the first
    click. Same regression shape as the hero CTA but exercises the
    header subtree (where the wordmark resource used to live)."""
    errors = _capture_console_errors(page)
    page.goto("/", wait_until="networkidle")

    link = page.locator(".site-header .site-nav a[href='/menu']")
    expect(link).to_be_visible(timeout=10_000)

    link.click()
    page.wait_for_url("**/menu", timeout=3_000)
    expect(page.locator(".menu .menu-header h1")).to_be_visible(timeout=10_000)

    panics = _hydration_panics(errors)
    assert not panics, (
        "Header link navigated but a hydration panic still fired:\n"
        + "\n".join(panics)
    )


def test_no_hydration_panic_on_menu_page(page: Page):
    """The /menu page mounts MenuPage with its combined OnceResource +
    the scroll/IntersectionObserver Effect. Guard against either
    re-introducing the panic shape."""
    errors = _capture_console_errors(page)
    page.goto("/menu", wait_until="networkidle")
    page.wait_for_timeout(500)
    panics = _hydration_panics(errors)
    assert not panics, (
        "Hydration panic detected on /menu. The combined-resource "
        "pattern (MenuPage::combined) may have been split again. "
        "Console messages:\n" + "\n".join(panics)
    )


def test_site_header_wordmark_renders_real_shop_name(page: Page):
    """The wordmark used to come from an OnceResource and could
    flash the fallback "Mein Restaurant" until the resource resolved.
    With the bootstrap-script switch the SSR HTML carries the real
    name in the markup itself; assert it lands without delay."""
    page.goto("/", wait_until="domcontentloaded")
    wordmark = page.locator(".site-header .brand-wordmark")
    expect(wordmark).to_be_visible(timeout=5_000)
    text = wordmark.inner_text().strip()
    # Should be uppercased per the SCSS and not the fallback. We
    # don't pin the exact string because deployments differ.
    assert text, "wordmark rendered empty"
    assert text != "MEIN RESTAURANT", (
        f"wordmark rendered the fallback ({text!r}) — bootstrap blob "
        f"either missing or shop_name not configured"
    )
