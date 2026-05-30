//! Role-based session helpers for the staff endpoints.
//!
//! Three roles, each with its own password env var and login form:
//!   - Admin   → ADMIN_PASSWORD,   /admin/login   (full management UI)
//!   - Kitchen → KITCHEN_PASSWORD, /kitchen/login (cook view only)
//!   - Driver  → DRIVER_PASSWORD,  /driver/login  (delivery view only)
//!
//! Sessions are cookie-bound. The new cookie is `dp_session` whose value is
//! the role string ("admin" / "kitchen" / "driver"). The existing
//! `admin_session=ok` cookie is honored too so live admin sessions survive
//! the rollout.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Kitchen,
    Driver,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Kitchen => "kitchen",
            Role::Driver => "driver",
        }
    }

    pub fn label_de(self) -> &'static str {
        match self {
            Role::Admin => "Admin",
            Role::Kitchen => "Küche",
            Role::Driver => "Fahrer",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Role::Admin),
            "kitchen" => Some(Role::Kitchen),
            "driver" => Some(Role::Driver),
            _ => None,
        }
    }

    /// Login URL for this role.
    pub fn login_path(self) -> &'static str {
        match self {
            Role::Admin => "/admin/login",
            Role::Kitchen => "/kitchen/login",
            Role::Driver => "/driver/login",
        }
    }

    /// Landing URL after a successful login.
    pub fn home_path(self) -> &'static str {
        match self {
            Role::Admin => "/admin",
            Role::Kitchen => "/kitchen",
            Role::Driver => "/driver",
        }
    }
}

pub const SESSION_COOKIE: &str = "dp_session";
pub const LEGACY_ADMIN_COOKIE: &str = "admin_session";

use leptos::prelude::*;

/// Shared logout server fn used by every role's "Abmelden" button. Clears
/// both the new session cookie and the legacy admin cookie, then redirects
/// the browser to the public home.
#[server(
    name = SessionLogout,
    prefix = "/api",
    endpoint = "session_logout"
)]
pub async fn session_logout() -> Result<(), ServerFnError> {
    ssr::logout().await?;
    leptos_axum::redirect("/");
    Ok(())
}

/// Returns the role of the current session, or `None` if not signed in.
/// Used by the cross-role shell nav so an admin can hop between
/// /admin, /kitchen and /driver without re-logging in. Kitchen/driver
/// sessions only see their own view (no escalation).
#[server(
    name = CurrentRole,
    prefix = "/api",
    endpoint = "current_role"
)]
pub async fn current_role() -> Result<Option<String>, ServerFnError> {
    Ok(ssr::current_role().await.map(|r| r.as_str().to_string()))
}

/// Server-fn used by protected pages to gate their initial render.
///
/// Returns `Ok(true)` if the current session has any of `allowed_roles`
/// (admin always passes — they see everything). Returns `Ok(false)` and
/// emits a `Location: <login_path>` redirect otherwise, so the browser
/// (during SSR) lands on the login page instead of a "nicht angemeldet"
/// error blob. Client-side after hydration, the redirect comes back as
/// a 303-style response that Leptos's Resource layer follows.
#[server(
    name = RequireRoleOrRedirect,
    prefix = "/api",
    endpoint = "require_role_or_redirect"
)]
pub async fn require_role_or_redirect(
    /// Comma-separated role names the page accepts ("driver" / "kitchen" /
    /// "admin"). Admin always passes regardless of this list.
    allowed_roles: String,
    /// Where to send the user when not authorised.
    login_path: String,
) -> Result<bool, ServerFnError> {
    let role = ssr::current_role().await;
    let allowed: Vec<Role> = allowed_roles
        .split(',')
        .filter_map(|s| Role::parse(s.trim()))
        .collect();

    let ok = match role {
        Some(Role::Admin) => true,
        Some(r) => allowed.contains(&r),
        None => false,
    };
    if ok {
        Ok(true)
    } else {
        leptos_axum::redirect(&login_path);
        Ok(false)
    }
}

/// Tiny pill-row navigation that lets an admin tap between Admin / Küche /
/// Fahrer without re-logging in. On a tablet bookmarked to /admin this is
/// the single biggest UX win — three roles, one tap each, same cookie.
///
/// For Kitchen-only or Driver-only sessions we render only the current
/// view's pill so non-admin staff can't escalate themselves into Admin
/// just because the link exists.
#[component]
pub fn RoleSwitcher(
    /// Which view is currently being rendered.
    current: Role,
) -> impl IntoView {
    // SSR + hydrate must produce IDENTICAL DOM. Two rules to satisfy:
    //   (1) Don't read the resource OUTSIDE a Suspense in hydrate mode —
    //       Leptos warns this is a hydration-mismatch landmine.
    //   (2) Suspense fallback's DOM shape must match the resolved view's,
    //       or the Tachys marker walker explodes (the original bug here).
    // Solution: always render all three pills inside Suspense. The resource
    // only gates one CSS class on the "others" span. Fallback renders the
    // same nav with the span pre-hidden; resolved view toggles based on role.
    let role_res = Resource::new(|| (), |_| async move { current_role().await });

    fn render_nav(current: Role, is_admin: bool) -> impl IntoView {
        let pill = move |role: Role, label: &'static str, icon: &'static str| {
            let path = role.home_path();
            let active = role == current;
            view! {
                <a href=path class:role-pill=true class:active=active>
                    <span class="icon">{icon}</span>
                    <span>{label}</span>
                </a>
            }
        };
        let mut others: Vec<_> = Vec::new();
        if current != Role::Admin   { others.push(pill(Role::Admin,   "Admin",  "🏪")); }
        if current != Role::Kitchen { others.push(pill(Role::Kitchen, "Küche",  "👨‍🍳")); }
        if current != Role::Driver  { others.push(pill(Role::Driver,  "Fahrer", "🛵")); }
        view! {
            <nav class="role-switcher" aria-label="Bereich wechseln">
                {pill(current, current.label_de(), match current {
                    Role::Admin => "🏪",
                    Role::Kitchen => "👨‍🍳",
                    Role::Driver => "🛵",
                })}
                <span class="role-switcher-others" class:hidden=!is_admin>
                    {others}
                </span>
            </nav>
        }
    }

    view! {
        <Suspense fallback=move || render_nav(current, false)>
            {move || {
                let is_admin = matches!(
                    role_res.get().and_then(|r| r.ok()).flatten().and_then(|s| Role::parse(&s)),
                    Some(Role::Admin)
                );
                render_nav(current, is_admin)
            }}
        </Suspense>
    }
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use leptos::prelude::ServerFnError;
    use leptos_axum::extract;
    use tower_cookies::{cookie::time::Duration, cookie::SameSite, Cookie, Cookies};

    /// Look up the env-var password for the given role. Falls back to a dev
    /// default for kitchen/driver so a developer can sign in without setting
    /// the env, but ADMIN has no fallback (the existing admin behaviour).
    fn expected_password(role: Role) -> Option<String> {
        match role {
            Role::Admin => std::env::var("ADMIN_PASSWORD").ok(),
            Role::Kitchen => {
                Some(std::env::var("KITCHEN_PASSWORD").unwrap_or_else(|_| "kueche".to_string()))
            }
            Role::Driver => {
                Some(std::env::var("DRIVER_PASSWORD").unwrap_or_else(|_| "fahrer".to_string()))
            }
        }
    }

    /// Verify the password matches and write the role into the session cookie.
    /// Returns `Ok(true)` on success, `Ok(false)` for a wrong password.
    pub async fn try_login(role: Role, password: &str) -> Result<bool, ServerFnError> {
        let expected = expected_password(role)
            .ok_or_else(|| ServerFnError::new("Passwort für diese Rolle nicht konfiguriert"))?;
        if password != expected {
            return Ok(false);
        }
        let cookies: Cookies = extract().await?;
        let mut c = Cookie::new(SESSION_COOKIE, role.as_str().to_string());
        c.set_path("/");
        c.set_http_only(true);
        c.set_same_site(SameSite::Lax);
        // 1 year. Staff devices (iPad on counter, kitchen tablet,
        // driver phone) should stay logged in indefinitely between
        // explicit logouts. Apple's WebView enforces a 7-day cap on
        // *tracking* cookies, but our session cookie is first-party
        // for davidspizzeria.de and exempt from that.
        c.set_max_age(Duration::days(365));
        cookies.add(c);
        Ok(true)
    }

    /// Clear the session cookies (both the new one and the legacy admin one).
    pub async fn logout() -> Result<(), ServerFnError> {
        let cookies: Cookies = extract().await?;
        for name in [SESSION_COOKIE, LEGACY_ADMIN_COOKIE] {
            let mut c = Cookie::new(name, "");
            c.set_path("/");
            c.set_max_age(Duration::ZERO);
            cookies.add(c);
        }
        Ok(())
    }

    /// What role (if any) does the current request authenticate as? Honors
    /// the legacy `admin_session=ok` cookie as well.
    pub async fn current_role() -> Option<Role> {
        let cookies: Cookies = extract().await.ok()?;
        if let Some(c) = cookies.get(SESSION_COOKIE) {
            if let Some(r) = Role::parse(c.value()) {
                return Some(r);
            }
        }
        if let Some(c) = cookies.get(LEGACY_ADMIN_COOKIE) {
            if c.value() == "ok" {
                return Some(Role::Admin);
            }
        }
        None
    }

    /// Gate a server fn to one or more roles. Admin always passes — they can
    /// see/do everything kitchen + driver can.
    pub async fn require_any(allowed: &[Role]) -> Result<Role, ServerFnError> {
        let role = current_role()
            .await
            .ok_or_else(|| ServerFnError::new("nicht angemeldet"))?;
        if role == Role::Admin || allowed.contains(&role) {
            Ok(role)
        } else {
            Err(ServerFnError::new("keine Berechtigung für diese Aktion"))
        }
    }
}
