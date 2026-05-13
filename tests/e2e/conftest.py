"""Shared fixtures for the davidspizzeria E2E test suite.

Run:
    source ~/Documents/develeop/rust/geodb-rs/crates/geodb-py/.env_py312/bin/activate
    cd tests/e2e
    cp .env.example .env       # fill in DPE2E_ADMIN_PW
    pytest -v

Tests target a configurable base URL (default davidspizzeria.de) and
hit an already-running server. They never modify production data — every
created entity uses a unique E2E-prefixed identifier so we can tell test
data apart by eye and clean it up out-of-band.

Key fixtures:
  base_url   — the URL under test (env BASE_URL or davidspizzeria.de)
  admin_pw   — admin password (env DPE2E_ADMIN_PW, required)
  e2e_tag    — short timestamp tag for the run, used in voucher codes etc.
  sandbox_only — autouse guard that skips the test if Stripe mode is 'live'

The sandbox guard is a safety net: if the shop has been flipped to live
mode and someone runs the suite by accident, every test self-skips with
a clear message instead of creating noise in the real Buchhaltung.
"""

from __future__ import annotations

import os
import time
from pathlib import Path

import pytest
from playwright.sync_api import Page, expect


# ---------------------------------------------------------------------------
# Env loading
# ---------------------------------------------------------------------------


def _load_dotenv() -> None:
    """Tiny .env loader so we don't need python-dotenv as a dep.

    Reads tests/e2e/.env if present; KEY=VALUE per line, # for comments.
    Does NOT overwrite existing environment variables — explicit
    `BASE_URL=… pytest` on the command line still wins.
    """
    env_file = Path(__file__).parent / ".env"
    if not env_file.exists():
        return
    for raw in env_file.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        os.environ.setdefault(key, value)


_load_dotenv()


# ---------------------------------------------------------------------------
# Basic fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def base_url() -> str:
    """Where to send requests. Override with BASE_URL env var.

    Default points at production davidspizzeria.de so first-time
    contributors don't need a local dev server. Tests stay safe via
    the sandbox-mode guard below.
    """
    url = os.environ.get("BASE_URL", "https://davidspizzeria.de").rstrip("/")
    return url


@pytest.fixture(scope="session")
def admin_pw() -> str:
    """Admin password from env. Failing fast here surfaces the missing
    secret before a test confusingly hangs on a login screen."""
    pw = os.environ.get("DPE2E_ADMIN_PW")
    if not pw:
        pytest.fail(
            "DPE2E_ADMIN_PW not set. Copy tests/e2e/.env.example to .env "
            "and fill in the admin password, or export it in your shell."
        )
    return pw


@pytest.fixture(scope="session")
def e2e_tag() -> str:
    """Unique short tag for this test run. Embedded in voucher codes
    + test phones so manual cleanup is grep-able."""
    return f"E2E{int(time.time())}"


# ---------------------------------------------------------------------------
# Playwright pytest-plugin overrides
# ---------------------------------------------------------------------------


def _env_flag(name: str, default: bool) -> bool:
    """Parse a boolean env var. Accepts: 1/0, true/false, yes/no (any case)."""
    raw = os.environ.get(name)
    if raw is None:
        return default
    return raw.strip().lower() in ("1", "true", "yes", "y", "on")


@pytest.fixture(scope="session")
def browser_context_args(browser_context_args, base_url):
    """Default every context to base_url so page.goto('/admin') works."""
    return {**browser_context_args, "base_url": base_url}


@pytest.fixture(scope="session")
def browser_type_launch_args(browser_type_launch_args):
    """Honour HEADLESS + SLOWMO from .env.

    HEADLESS unset → don't touch defaults (lets `--headed` CLI win).
    HEADLESS=true  → headless mode (no window).
    HEADLESS=false → visible window.
    SLOWMO=300     → 300ms artificial delay between actions, for watching.
    """
    extras = {}
    if "HEADLESS" in os.environ:
        extras["headless"] = _env_flag("HEADLESS", default=True)
    slowmo = os.environ.get("SLOWMO")
    if slowmo:
        try:
            extras["slow_mo"] = int(slowmo)
        except ValueError:
            pass
    return {**browser_type_launch_args, **extras}


def pytest_configure(config):
    """Inject BROWSER env into pytest's CLI options before pytest-playwright
    consumes them. Runs early enough that the plugin sees our value."""
    browser = os.environ.get("BROWSER")
    if not browser:
        return
    browser = browser.strip().lower()
    # Engine vs channel: 'chromium', 'firefox', 'webkit' are engines.
    # 'chrome' + 'msedge' are channels on top of the chromium engine.
    engines = {"chromium", "firefox", "webkit"}
    channels = {"chrome", "msedge", "chrome-beta", "msedge-beta"}
    # `--browser` from pytest-playwright is a list (multiple-allowed).
    # We treat "no value passed" as either missing or empty list.
    cli_browsers = config.getoption("--browser") or []
    if cli_browsers:
        return  # CLI wins, leave env alone
    if browser in engines:
        config.option.browser = [browser]
    elif browser in channels:
        # Channels run on the chromium engine.
        config.option.browser = ["chromium"]
        if not config.getoption("--browser-channel"):
            config.option.browser_channel = browser
    else:
        # Unknown value — surface it instead of silently ignoring.
        raise pytest.UsageError(
            f"BROWSER={browser!r} not recognised. "
            f"Use one of {sorted(engines | channels)}."
        )


# ---------------------------------------------------------------------------
# Sandbox-mode safety guard
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def sandbox_only(page: Page, base_url: str) -> None:
    """Refuse to run if the target shop is currently in Stripe live mode.

    Call the same server fn the admin chip uses: POST /api/get_stripe_mode
    (a leptos server fn — empty body, returns JSON). 'sandbox' → continue,
    anything else (including network errors) → skip the test.
    """
    api = f"{base_url}/api/get_stripe_mode"
    try:
        resp = page.request.post(api, data="")
    except Exception as e:
        pytest.skip(f"could not reach {api}: {e}")
    if not resp.ok:
        pytest.skip(f"{api} returned {resp.status}; cannot verify sandbox mode")
    try:
        mode = resp.json()
    except Exception:
        pytest.skip(f"{api} did not return JSON: {resp.text()[:200]}")
    # Server fn return value is a bare string in JSON (leptos serialises
    # `Result<String, _>` as `"sandbox"` directly).
    if mode != "sandbox":
        pytest.skip(
            f"Stripe mode is '{mode}', not 'sandbox' — refusing to run E2E "
            f"tests against live data. Flip /admin/settings to Sandbox first."
        )


# ---------------------------------------------------------------------------
# Admin login helper
# ---------------------------------------------------------------------------


@pytest.fixture
def admin_page(page: Page, admin_pw: str) -> Page:
    """Log into /admin/login and return the now-authenticated page.

    Re-uses the storage state for the rest of the test so navigations
    within /admin/* don't need to re-auth. We deliberately do NOT make
    this session-scoped — admin_session cookies can leak between tests
    that exercise the logout flow.
    """
    page.goto("/admin/login")
    page.fill("input[name='password']", admin_pw)
    page.click("button[type='submit']")
    # After successful login the action redirects to /admin (the server
    # fn returns Ok(true) and the page picks that up). Wait for the
    # admin shell instead of guessing a fixed delay.
    expect(page.locator(".admin-shell-bar")).to_be_visible(timeout=10_000)
    return page
