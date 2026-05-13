"""Verify /admin/extras shows the merged tabs UI (Extras + Auswahl-Gruppen)
and the Auswahl-Gruppen tab renders the bulk-attach-by-category form.

This is the post-deploy smoke test for the option-groups feature merge:
- Top-nav has NO 'Auswahl-Gruppen' link (merged into /admin/extras).
- /admin/extras shows two .tab buttons.
- Default tab is "Extras (frei wählbar)" → .extras-create form visible.
- Clicking the second tab reveals the options-admin body, including
  "Neue Gruppe" form and (if any groups exist) the bulk-attach row.
- Page produces no console errors / page errors.
"""

from playwright.sync_api import Page, expect


def test_extras_admin_has_two_tabs(admin_page: Page) -> None:
    page = admin_page

    console_errors: list[str] = []
    page_errors: list[str] = []
    page.on("console", lambda m: console_errors.append(f"{m.type}: {m.text}") if m.type == "error" else None)
    page.on("pageerror", lambda e: page_errors.append(str(e)))

    # Top-nav must not link to the legacy /admin/options anymore.
    nav_texts = page.locator(".admin-nav a").all_text_contents()
    print(f"NAV LINKS: {nav_texts}")
    assert "Auswahl-Gruppen" not in nav_texts, (
        f"top nav still has 'Auswahl-Gruppen' link: {nav_texts}"
    )

    page.goto("/admin/extras")
    page.wait_for_load_state("networkidle", timeout=15_000)

    # Confirm we actually got the new page header.
    header_text = page.locator(".admin-bar h1").first.inner_text()
    print(f"HEADER: {header_text}")

    tabs = page.locator("nav.admin-tabs button.tab")
    n_tabs = tabs.count()
    print(f"TAB COUNT: {n_tabs}")
    for i in range(n_tabs):
        print(f"  - {tabs.nth(i).inner_text()}")

    assert n_tabs == 2, (
        f"expected 2 tabs ('Extras' + 'Auswahl-Gruppen'), saw {n_tabs}. "
        f"Header was: {header_text!r}"
    )

    # Default tab is Extras.
    expect(page.locator(".extras-create")).to_be_visible()

    # Flip to the Auswahl-Gruppen tab.
    page.locator("nav.admin-tabs button.tab", has_text="Auswahl-Gruppen").click()
    expect(page.locator(".options-admin")).to_be_visible(timeout=5_000)

    # The "Neue Gruppe" form must be present.
    expect(page.locator(".options-create")).to_be_visible()

    # Bulk-attach row only shows up if there's at least one group.
    # We log either way; assertion is only on the create-form + tab structure.
    bulk_rows = page.locator(".og-bulk").count()
    print(f"BULK-ATTACH ROWS: {bulk_rows}")

    assert not page_errors, f"page errors: {page_errors}"
    assert not console_errors, f"console errors: {console_errors}"
