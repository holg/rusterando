"""Test the "Gutschein für diesen Kunden"-deeplink phone autofill.

Background: clicking the link on /admin/customers/:id should land on
/admin/vouchers?phone=<that-customer's-phone>, with the Telefon-
Bindung input pre-filled. Earlier today the input stayed empty because
the URL pickup was done via a one-shot `window.location.search` effect
that didn't fire on CSR navigation; the fix routes the param through
leptos_router's reactive query map.

This test:
  1. Opens /admin/customers (list view), picks the first row (any
     customer will do — the assertion is shape-only, not value).
  2. Reads the phone shown on the detail page.
  3. Clicks the "🎁 Gutschein für diesen Kunden"-button.
  4. Verifies /admin/vouchers loaded AND the customer-phone input
     contains the customer's phone.

Skips itself if there is no customer yet (fresh DB).
"""

from __future__ import annotations

import re

import pytest
from playwright.sync_api import expect


def test_customer_voucher_deeplink_prefills_phone(admin_page):
    admin_page.goto("/admin/customers")
    expect(admin_page.locator(".admin-customers h1")).to_have_text("Kunden")

    first_row = admin_page.locator(".admin-customers-table tbody tr").first
    try:
        first_row.wait_for(state="visible", timeout=5_000)
    except Exception:
        pytest.skip("no customers in DB yet — nothing to test against")

    # Skip if the table only renders the "no customers found" placeholder
    # (one cell, colspan=7 — our SQL `LIKE` filter style).
    cells = first_row.locator("td")
    if cells.count() < 2:
        pytest.skip("empty-state row in customers table — no customer to use")

    # The row carries multiple <a>'s: a tel: link, optionally a mailto:,
    # and the "Details"-button which is the navigation target. Pick that
    # one explicitly — clicking the first <a> would dial the customer.
    first_row.locator("a[href^='/admin/customers/']").click()
    expect(admin_page.locator(".admin-customer-detail")).to_be_visible(
        timeout=5_000
    )

    # Pull the phone string from the tel: href — it's the same value
    # the deep-link encodes. Reading the visible text and stripping the
    # 📞 emoji would lose a leading '+' on international numbers
    # because the simple "strip non-digits" regex eats it too.
    phone_link = admin_page.locator(".customer-head .contacts a[href^='tel:']")
    expect(phone_link).to_be_visible()
    phone = (phone_link.get_attribute("href") or "").removeprefix("tel:").strip()
    assert phone, f"could not extract phone from tel: href"

    # Click the "Gutschein für diesen Kunden"-link. It's anchored as
    # `/admin/vouchers?phone=<encoded>`.
    voucher_link = admin_page.locator(
        ".customer-head .contacts a[href^='/admin/vouchers']"
    )
    expect(voucher_link).to_be_visible()
    voucher_link.click()

    admin_page.wait_for_url(re.compile(r"/admin/vouchers\?phone="), timeout=5_000)

    # The phone-input in the create card must be filled with the same
    # number we saw on the detail page. Allow trailing whitespace; the
    # field shows the raw URL-decoded value.
    phone_input = admin_page.locator(
        ".create-voucher input[type='tel']"
    )
    expect(phone_input).to_be_visible()
    actual = (phone_input.input_value() or "").strip()
    assert actual == phone.strip(), (
        f"deeplink didn't autofill: expected {phone!r}, got {actual!r}"
    )
