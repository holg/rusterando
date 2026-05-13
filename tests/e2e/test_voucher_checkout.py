"""End-to-end: phone-bound voucher covers the whole order, no Stripe.

The test needs four specific things in the target deployment:
  * a menu item (DPE2E_ITEM_ID + DPE2E_ITEM_EXTRAS) that costs ≥ the
    min order for delivery
  * a recognised customer (DPE2E_CUSTOMER_PHONE) with a saved
    delivery address — used to exercise the phone-recall auto-fill
  * a voucher (DPE2E_VOUCHER_CODE) bound to that phone, fixed-amount,
    large enough to cap subtotal + delivery_fee to 0 €

  Without those env vars the test skips itself, because the assertions
  don't make sense against a fresh / empty DB.

  UI-only assertions so the test runs against any deployment. Against a
  live shop, it WILL create a real order in the Sandbox-DB — that's the
  price of a real E2E roundtrip. Test order is grep-able by its
  voucher code.
"""

from __future__ import annotations

import os
import re

import pytest
from playwright.sync_api import expect


# Set these in tests/e2e/.env (or shell) for the deployment under test.
# Cart subtotal must clear the delivery min order; the voucher must be a
# fixed-amount type bound to the phone, large enough to cap
# (subtotal + delivery_fee) to 0.
ITEM_ID = os.environ.get("DPE2E_ITEM_ID", "")
EXTRAS_IDS = [
    s.strip() for s in os.environ.get("DPE2E_ITEM_EXTRAS", "").split(",") if s.strip()
]
QTY = int(os.environ.get("DPE2E_ITEM_QTY", "1"))
EXPECTED_SUBTOTAL_CENTS = int(os.environ.get("DPE2E_EXPECTED_SUBTOTAL_CENTS", "0"))
PHONE = os.environ.get("DPE2E_CUSTOMER_PHONE", "")
EXPECTED_STREET_PREFIX = os.environ.get("DPE2E_EXPECTED_STREET_PREFIX", "")
EXPECTED_POSTCODE = os.environ.get("DPE2E_EXPECTED_POSTCODE", "")
VOUCHER_CODE = os.environ.get("DPE2E_VOUCHER_CODE", "")


def _seed_cart(page, base_url: str) -> None:
    """POST directly to /api/add_to_cart to skip the menu-page UI.

    Leptos serialises `Vec<String>` form params as `extras_ids[0]=…&
    extras_ids[1]=…`. We send a single request with a custom
    URL-encoded body so the bracketed-indices survive (Playwright's
    `form=` would urlencode the brackets again).
    """
    parts = [
        f"menu_item_id={ITEM_ID}",
        "size=single",
        f"quantity={QTY}",
    ]
    for i, eid in enumerate(EXTRAS_IDS):
        parts.append(f"extras_ids[{i}]={eid}")
    body = "&".join(parts)
    resp = page.request.post(
        f"{base_url}/api/add_to_cart",
        data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    assert resp.ok, f"add_to_cart failed: {resp.status} {resp.text()[:300]}"
    payload = resp.json()
    # Sanity vs the env-configured expectation.
    assert payload["subtotal_cents"] == EXPECTED_SUBTOTAL_CENTS, (
        f"seeded cart has wrong subtotal: got {payload['subtotal_cents']} cents, "
        f"expected DPE2E_EXPECTED_SUBTOTAL_CENTS={EXPECTED_SUBTOTAL_CENTS}. "
        "Did the menu_items / pizza_extras prices change, or did you change "
        "DPE2E_ITEM_* without updating the expectation?"
    )


def test_phone_bound_voucher_covers_whole_order(page, base_url):
    # Skip cleanly if the deployment-specific env vars aren't filled in.
    missing = [
        name for name, value in [
            ("DPE2E_ITEM_ID", ITEM_ID),
            ("DPE2E_VOUCHER_CODE", VOUCHER_CODE),
            ("DPE2E_CUSTOMER_PHONE", PHONE),
            ("DPE2E_EXPECTED_SUBTOTAL_CENTS", EXPECTED_SUBTOTAL_CENTS),
        ] if not value
    ]
    if missing:
        pytest.skip(f"voucher-checkout test needs env vars: {', '.join(missing)}")

    # ---- 1) Seed cart via API ----------------------------------------------
    _seed_cart(page, base_url)

    # ---- 2) Land on /checkout with the voucher pre-attached -----------------
    page.goto(f"/checkout?code={VOUCHER_CODE}", wait_until="networkidle")
    expect(page.locator(".checkout h1")).to_have_text("Kasse")

    # Voucher input should be pre-filled from the URL.
    voucher_input = page.locator(".voucher-fieldset input[name='voucher_code']")
    expect(voucher_input).to_have_value(VOUCHER_CODE, timeout=5_000)

    # ---- 3) Select Lieferung (delivery) so the address recall path runs ----
    page.locator("input[name='order_type'][value='delivery']").click()
    # Phone field is in the contact fieldset.
    phone_input = page.locator("input[name='phone']")
    phone_input.fill(PHONE)
    # Debounced lookup fires at ≥6 digits and runs a server fn; give it
    # ~2 seconds to land and auto-fill the address.
    page.wait_for_timeout(2_000)

    # Address fields should be auto-filled from customer_addresses.
    # Specific values come from env so the test stays neutral.
    if EXPECTED_STREET_PREFIX:
        expect(page.locator("input[name='street']")).to_have_value(
            re.compile(re.escape(EXPECTED_STREET_PREFIX)), timeout=5_000
        )
    if EXPECTED_POSTCODE:
        expect(page.locator("input[name='postcode']")).to_have_value(EXPECTED_POSTCODE)
    # Address fields should at least be populated (non-empty) once recall hits.
    expect(page.locator("input[name='street']")).not_to_have_value("")
    expect(page.locator("input[name='house_number']")).not_to_have_value("")
    expect(page.locator("input[name='postcode']")).not_to_have_value("")

    # Name + email should have come along too (just non-empty — exact
    # values are PII we don't pin here).
    expect(page.locator("input[name='name']")).not_to_have_value("")
    expect(page.locator("input[name='email']")).not_to_have_value("")

    # ---- 4) Verify voucher feedback + total = 0 € ---------------------------
    # Live-validator should accept the code now that we have a phone +
    # subtotal ≥ min_order.
    expect(page.locator(".voucher-fieldset .ok")).to_be_visible(timeout=5_000)

    # The summary-side grand-total should read 0,00 € after voucher:
    # 16 € subtotal + 1,50 € delivery − 17,50 € voucher (capped) = 0,00 €.
    big_total = page.locator(".summary-side .big")
    expect(big_total).to_have_text(re.compile(r"0[.,]00\s*€"), timeout=5_000)

    # ---- 5) Pick card payment, verify submit button skips Stripe ------------
    # Card payment is the harder case — without the new total==0 short-
    # circuit, the customer would face a Stripe Payment Element with a
    # zero-amount intent, which Stripe rejects. The fix in place_order
    # marks the order 'voucher_paid' and skips intent creation. Submit
    # button text confirms it: "Bestellung aufgeben" instead of
    # "Weiter zur Zahlung".
    page.locator("input[name='payment_method'][value='card']").click()
    submit_btn = page.locator("button[type='submit']")
    expect(submit_btn).to_have_text("Bestellung aufgeben", timeout=5_000)

    # ---- 6) Submit + verify on the confirmation page -----------------------
    # UI-only assertions so this runs against any deployment. The chain
    # of observable proof that NO Stripe charge happened:
    #   - submit button said "Bestellung aufgeben" (checked above)
    #   - we land on /orders/<id>, NOT on /checkout with a Stripe iframe
    #   - the page renders no #payment-element div (the Stripe mount target)
    #   - "Gesamt: 0,00 €" is displayed
    submit_btn.click()
    page.wait_for_url(re.compile(r"/orders/[0-9a-f-]{36}"), timeout=10_000)
    m = re.search(r"/orders/([0-9a-f-]{36})", page.url)
    assert m, f"didn't land on order detail page: {page.url}"
    order_id = m.group(1)
    print(f"\nCreated order: {order_id}")

    # Confirmation page renders behind a Suspense; wait for the
    # grand-total to materialise.
    grand_total = page.locator(".confirm-card .grand-total")
    expect(grand_total).to_be_visible(timeout=10_000)
    expect(grand_total).to_contain_text(re.compile(r"0[.,]00\s*€"))

    # Stripe-only DOM nodes must NOT be present — that's the strongest
    # in-browser signal that we didn't hit the card-payment path.
    assert page.locator("#payment-element").count() == 0, (
        "found #payment-element on the confirmation page — "
        "Stripe was invoked even though the voucher covers the total"
    )
