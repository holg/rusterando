"""SEO surface tests: structured data + the dynamic SEO pages.

Two complementary proofs about the Restaurant JSON-LD:

  * **Raw-HTML (it IS there, server-side):** the initial SSR HTML carries
    one `<script type="application/ld+json">` with a valid `Restaurant`
    graph (menu sections, items, prices, address/geo). It's injected in the
    shell `<head>`, so it appears on EVERY page (/, /menu, /impressum, …).
    These checks use `page.request.get(...)` — raw HTTP, no hydration — so
    they assert exactly what a crawler reads.

  * **E2E (it is NOT there on the hydrated client):** after the WASM app
    hydrates and the user navigates client-side (/ → /menu), the browser
    fires NO request to the (removed) `restaurant_jsonld` server fn and does
    NOT inject a second JSON-LD script. The structured data lives only in the
    SSR HTML; re-emitting it on the client would be wasted work. This proves
    the SSR-only design that the shell-injection refactor put in place.

Plus the dynamic SEO pages: category pages (/menu/<slug>) canonical→/menu,
delivery landing pages (/lieferservice/<slug>) self-canonical, unknown slugs
404, and the dynamic sitemap. All gated by the autouse sandbox guard.
"""

from __future__ import annotations

import json
import re

import pytest
from playwright.sync_api import Page, expect

# The server fn endpoint we deliberately REMOVED when the JSON-LD became
# shell-injected + cached. A client must never call it again.
DROPPED_JSONLD_ENDPOINT = "/api/restaurant_jsonld"

LD_RE = re.compile(
    r'<script[^>]*type="application/ld\+json"[^>]*>(.*?)</script>', re.S
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _fetch(page: Page, base_url: str, path: str) -> str:
    """Raw GET of the SSR HTML for `path` (no browser render)."""
    resp = page.request.get(f"{base_url}{path}")
    assert resp.ok, f"GET {path} → {resp.status}"
    return resp.text()


def _extract_jsonld(html: str) -> list[dict]:
    """All parsed application/ld+json blocks in the HTML."""
    out = []
    for m in LD_RE.finditer(html):
        body = m.group(1).strip()
        if body:
            out.append(json.loads(body))
    return out


def _restaurant_node(blocks: list[dict]) -> dict | None:
    for b in blocks:
        if b.get("@type") == "Restaurant":
            return b
    return None


# ---------------------------------------------------------------------------
# RAW-HTML PROOFS — the JSON-LD IS there, server-side, everywhere
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("path", ["/", "/menu", "/impressum"])
def test_jsonld_present_in_raw_html_sitewide(page: Page, base_url: str, path: str):
    """Exactly one valid Restaurant JSON-LD block in the SSR HTML of every
    public page (it's shell-injected, hence site-wide)."""
    html = _fetch(page, base_url, path)
    blocks = _extract_jsonld(html)
    node = _restaurant_node(blocks)
    assert node is not None, (
        f"No <script type=application/ld+json> Restaurant block in the raw "
        f"HTML of {path}. It should be baked into <head> by the shell."
    )
    assert node["@context"] == "https://schema.org"
    assert node.get("name"), "Restaurant.name missing"


def test_jsonld_has_full_menu_with_prices(page: Page, base_url: str):
    """The Restaurant graph carries the FULL menu with prices — item SEO
    without thin per-item pages."""
    node = _restaurant_node(_extract_jsonld(_fetch(page, base_url, "/")))
    assert node is not None
    sections = node["hasMenu"]["hasMenuSection"]
    assert len(sections) >= 1, "hasMenuSection empty"
    items = [it for s in sections for it in s.get("hasMenuItem", [])]
    assert len(items) >= 10, f"expected a real menu, got {len(items)} items"
    # Every item must have at least one EUR offer with a decimal price.
    for it in items:
        offers = it.get("offers", [])
        assert offers, f"item {it.get('name')!r} has no offers"
        for o in offers:
            assert o["priceCurrency"] == "EUR"
            assert re.fullmatch(r"\d+\.\d{2}", o["price"]), (
                f"bad price {o['price']!r} on {it.get('name')!r}"
            )


def test_jsonld_has_localbusiness_fields(page: Page, base_url: str):
    """Address + geo + opening hours — the bits that drive the local rich
    result. (Skip individual fields if the shop hasn't configured them, but
    at least address OR geo must be present for a real deployment.)"""
    node = _restaurant_node(_extract_jsonld(_fetch(page, base_url, "/")))
    assert node is not None
    assert "address" in node or "geo" in node, (
        "neither address nor geo present — LocalBusiness signal too weak"
    )
    if "address" in node:
        assert node["address"]["@type"] == "PostalAddress"
    if "openingHoursSpecification" in node:
        assert isinstance(node["openingHoursSpecification"], list)


# ---------------------------------------------------------------------------
# E2E PROOFS — the JSON-LD is NOT re-emitted on the hydrated client
# ---------------------------------------------------------------------------


def test_exactly_one_jsonld_script_in_live_dom(page: Page):
    """After hydration the live DOM has exactly one ld+json script (the SSR
    one) — the client never injects a duplicate."""
    page.goto("/", wait_until="domcontentloaded")
    page.wait_for_timeout(500)  # let hydrate run
    count = page.evaluate(
        "() => document.querySelectorAll('script[type=\"application/ld+json\"]').length"
    )
    assert count == 1, f"expected exactly 1 ld+json script after hydrate, got {count}"


def test_no_jsonld_fetch_on_client_navigation(page: Page):
    """The killer proof of the SSR-only design: hydrate on /, then navigate
    client-side to /menu, and assert the browser fired NO request to the
    removed restaurant_jsonld server fn (and no duplicate script appears)."""
    jsonld_requests: list[str] = []

    def on_request(req):
        if "restaurant_jsonld" in req.url:
            jsonld_requests.append(req.url)

    page.on("request", on_request)

    page.goto("/", wait_until="domcontentloaded")
    page.wait_for_timeout(500)

    # Client-side nav via the header link (delegated router handler, no full
    # page reload) — this is where a per-page resource WOULD refetch.
    link = page.locator(".site-header .site-nav a[href='/menu']")
    expect(link).to_be_visible(timeout=10_000)
    link.click()
    page.wait_for_url("**/menu", timeout=5_000)
    page.wait_for_timeout(500)

    assert not jsonld_requests, (
        "Client navigation triggered a restaurant_jsonld fetch — the JSON-LD "
        f"should be SSR-only, never fetched on the client: {jsonld_requests}"
    )
    # And still exactly one script in the DOM (the SSR /menu one), not stacked.
    count = page.evaluate(
        "() => document.querySelectorAll('script[type=\"application/ld+json\"]').length"
    )
    assert count == 1, f"after client nav expected 1 ld+json script, got {count}"


def test_dropped_jsonld_endpoint_is_gone(page: Page, base_url: str):
    """The old restaurant_jsonld server fn was removed. Hitting its endpoint
    must NOT return a JSON-LD payload (404 / not-found / error is fine)."""
    resp = page.request.post(f"{base_url}{DROPPED_JSONLD_ENDPOINT}", data="")
    # Acceptable: 404 (route gone) or any non-2xx. The hard fail is a 200
    # that returns the JSON-LD string (meaning the endpoint still exists).
    if resp.ok:
        body = resp.text()
        assert "schema.org" not in body and "Restaurant" not in body, (
            "restaurant_jsonld endpoint still serves structured data — it "
            "should have been removed when JSON-LD went SSR-only."
        )


# ---------------------------------------------------------------------------
# DYNAMIC SEO PAGES — category + delivery landing + sitemap
# ---------------------------------------------------------------------------


def _first_active_category_slug(page: Page, base_url: str) -> str | None:
    """Pull a real category slug out of the sitemap."""
    sm = _fetch(page, base_url, "/sitemap.xml")
    m = re.search(r"<loc>[^<]*/menu/([^<]+)</loc>", sm)
    return m.group(1) if m else None


def _first_area_slug(page: Page, base_url: str) -> str | None:
    sm = _fetch(page, base_url, "/sitemap.xml")
    m = re.search(r"<loc>[^<]*/lieferservice/([^<]+)</loc>", sm)
    return m.group(1) if m else None


def test_sitemap_lists_dynamic_pages(page: Page, base_url: str):
    """The sitemap is DB-driven: it lists the static pages plus a /menu/<slug>
    per category and a /lieferservice/<slug> per active zone, as XML."""
    resp = page.request.get(f"{base_url}/sitemap.xml")
    assert resp.ok
    assert "xml" in resp.headers.get("content-type", "").lower()
    body = resp.text()
    assert "<loc>" in body
    assert "/menu/" in body, "no dynamic category URLs in sitemap"
    assert "/lieferservice/" in body, "no delivery-area URLs in sitemap"


def test_category_page_canonical_to_menu(page: Page, base_url: str):
    """A category page renders (200) and canonicalises to /menu so it doesn't
    cannibalise the main menu page."""
    slug = _first_active_category_slug(page, base_url)
    if not slug:
        pytest.skip("no category slug in sitemap")
    html = _fetch(page, base_url, f"/menu/{slug}")
    m = re.search(r'<link[^>]*rel="canonical"[^>]*href="([^"]+)"', html) or re.search(
        r'<link[^>]*href="([^"]+)"[^>]*rel="canonical"', html
    )
    assert m, "no canonical link on category page"
    assert m.group(1).rstrip("/").endswith("/menu"), (
        f"category canonical should point at /menu, got {m.group(1)}"
    )


def test_delivery_landing_self_canonical(page: Page, base_url: str):
    slug = _first_area_slug(page, base_url)
    if not slug:
        pytest.skip("no delivery-area slug in sitemap")
    html = _fetch(page, base_url, f"/lieferservice/{slug}")
    assert "Pizza-Lieferservice" in html, "landing page missing its H1 copy"
    m = re.search(r'<link[^>]*rel="canonical"[^>]*href="([^"]+)"', html) or re.search(
        r'<link[^>]*href="([^"]+)"[^>]*rel="canonical"', html
    )
    assert m and m.group(1).rstrip("/").endswith(f"/lieferservice/{slug}"), (
        f"delivery landing should be self-canonical, got {m.group(1) if m else None}"
    )


@pytest.mark.parametrize("path", ["/menu/nope-not-a-category", "/lieferservice/nirgendwo"])
def test_unknown_seo_slug_returns_404(page: Page, base_url: str, path: str):
    """Unknown category / area slugs must 404 (not soft-200) so Google
    doesn't index empty pages."""
    resp = page.request.get(f"{base_url}{path}")
    assert resp.status == 404, f"{path} should 404, got {resp.status}"
