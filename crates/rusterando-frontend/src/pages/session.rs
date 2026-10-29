//! Role-based session helpers for the staff endpoints.
//!
//! Three roles, each with its own password env var and login form:
//!   - Admin   → ADMIN_PASSWORD,   /admin/login   (full management UI)
//!   - Kitchen → KITCHEN_PASSWORD, /kitchen/login (cook view only)
//!   - Driver  → DRIVER_PASSWORD,  /driver/login  (delivery view only)
//!   - SysAdmin → SYS_ADMIN_PASSWORD, /sysadmin/login (platform operator:
//!     tenant import/provisioning on the apex host only — see pages/sysadmin)
//!
//! Sessions are cookie-bound, two cookies set together at login:
//!   - `dp_session=<role>` ("admin" / "kitchen" / "driver" / "sysadmin"): the
//!     role in plain text, which the native apps read (push registration);
//!   - `dp_auth=<hex>`: HMAC-SHA256 with the server's key over the tenant, the
//!     role and that role's CURRENT password.
//!
//! A role counts only with a matching `dp_auth`, so the role can't be
//! forged by setting a cookie, a session from one shop is worthless at
//! another, and changing a role's password logs that role out everywhere.
//! The key lives in `data/session.key` (`SESSION_KEY_FILE`), created on first
//! start; deleting it logs everyone out. The old unsigned `admin_session=ok`
//! is no longer honored (logout still clears it).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Kitchen,
    Driver,
    /// Platform operator on the apex host (`rusterando.de/sysadmin`). NOT a
    /// superset of Admin: `require_admin` / `require_any` don't accept it,
    /// so the operator cookie never grants shop-admin access anywhere.
    SysAdmin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Kitchen => "kitchen",
            Role::Driver => "driver",
            Role::SysAdmin => "sysadmin",
        }
    }

    pub fn label_de(self) -> &'static str {
        match self {
            Role::Admin => "Admin",
            Role::Kitchen => "Küche",
            Role::Driver => "Fahrer",
            Role::SysAdmin => "System",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Role::Admin),
            "kitchen" => Some(Role::Kitchen),
            "driver" => Some(Role::Driver),
            "sysadmin" => Some(Role::SysAdmin),
            _ => None,
        }
    }

    /// Login URL for this role.
    pub fn login_path(self) -> &'static str {
        match self {
            Role::Admin => "/admin/login",
            Role::Kitchen => "/kitchen/login",
            Role::Driver => "/driver/login",
            Role::SysAdmin => "/sysadmin/login",
        }
    }

    /// Landing URL after a successful login.
    pub fn home_path(self) -> &'static str {
        match self {
            Role::Admin => "/admin",
            Role::Kitchen => "/kitchen",
            Role::Driver => "/driver",
            Role::SysAdmin => "/sysadmin",
        }
    }
}

pub const SESSION_COOKIE: &str = "dp_session";
/// The session's signature (see the module docs).
pub const AUTH_COOKIE: &str = "dp_auth";
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

/// Live client-side view of "am I still signed in?" for a staff page. The
/// shell's `[Abmelden]` button renders this so the operator always sees the
/// TRUE auth state — not whatever the page was SSR'd with. The root problem
/// it fixes: the browser trusts a session cookie that the server has already
/// expired, then hangs forever on the next server-fn call. Here we ask the
/// server directly (with a hard timeout) and surface the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStatus {
    /// Heartbeat in flight / not yet checked. Render the button optimistically.
    Checking,
    /// The server confirms this session authenticates as `Role`.
    Authed(Role),
    /// The server says we're NOT signed in (cookie missing/expired server-side).
    /// The cookie the browser still holds is stale → force a real re-login.
    Expired,
    /// The check didn't come back in time (8s) — server slow/unreachable or a
    /// proxy is eating the request. We can't prove auth either way; tell the
    /// operator to check the connection rather than spin forever.
    Unreachable,
}

impl AuthStatus {
    /// German one-word status for the chip next to `[Abmelden]`.
    pub fn label_de(self) -> &'static str {
        match self {
            AuthStatus::Checking => "…",
            AuthStatus::Authed(r) => r.label_de(),
            AuthStatus::Expired => "Sitzung abgelaufen",
            AuthStatus::Unreachable => "Server antwortet nicht",
        }
    }

    /// CSS modifier for colour (green / amber / red).
    pub fn css_class(self) -> &'static str {
        match self {
            AuthStatus::Checking => "checking",
            AuthStatus::Authed(_) => "authed",
            AuthStatus::Expired => "expired",
            AuthStatus::Unreachable => "unreachable",
        }
    }
}

/// Run `current_role()` with a hard `timeout_ms` ceiling. The whole point of
/// the exercise: never await a staff server-fn forever. Returns the resolved
/// `AuthStatus` — `Authed`/`Expired` on a real answer, `Unreachable` if the
/// call doesn't come back in time (or errors at the transport layer).
///
/// `expected` is the role this page is for; an admin always satisfies a
/// kitchen/driver page (no downgrade), mirroring `require_any`.
#[cfg(feature = "hydrate")]
pub async fn check_auth_once(expected: Role, timeout_ms: i32) -> AuthStatus {
    // Race the server-fn against a JS setTimeout. wasm is single-threaded, so
    // "abort" here means "stop awaiting and move on" — the in-flight fetch is
    // abandoned (the browser cancels it when nothing holds the promise).
    let call = async { current_role().await };
    match with_timeout(call, timeout_ms).await {
        // Server answered in time.
        Some(Ok(Some(role_str))) => match Role::parse(&role_str) {
            Some(role) => {
                // Admin passes every staff page; otherwise the role must match.
                if role == Role::Admin || role == expected {
                    AuthStatus::Authed(role)
                } else {
                    // Signed in, but as the wrong role for this page.
                    AuthStatus::Expired
                }
            }
            None => AuthStatus::Expired,
        },
        // Server answered: not signed in.
        Some(Ok(None)) => AuthStatus::Expired,
        // Transport/server error — treat like unreachable (don't claim expired,
        // we genuinely don't know).
        Some(Err(_)) => AuthStatus::Unreachable,
        // Timed out.
        None => AuthStatus::Unreachable,
    }
}

/// Await `fut`, but give up after `timeout_ms`. `Some(v)` = `fut` finished
/// first; `None` = the timer won. Built on `setTimeout` + a JS Promise so it
/// needs no extra timer crate. Hydrate-only (there's no wall clock in SSR).
#[cfg(feature = "hydrate")]
pub async fn with_timeout<T>(
    fut: impl std::future::Future<Output = T>,
    timeout_ms: i32,
) -> Option<T> {
    use futures::future::{select, Either};
    use std::pin::pin;

    let timer = async {
        let p = js_sys::Promise::new(&mut |resolve, _reject| {
            let win = web_sys::window().expect("window");
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, timeout_ms);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(p).await;
    };

    match select(pin!(fut), pin!(timer)).await {
        Either::Left((v, _)) => Some(v),
        Either::Right(((), _)) => None,
    }
}

/// Reactive auth heartbeat for a staff shell. Returns a signal that starts at
/// `Checking`, then settles to the live `AuthStatus` and KEEPS it fresh:
///   - an immediate check on mount,
///   - a re-check every 30s,
///   - a re-check whenever the tab regains visibility (the classic "left the
///     iPad open overnight, came back to a dead session" case).
/// Hydrate-only; SSR returns a constant `Checking` (the button renders
/// optimistically server-side, then the browser tells the truth).
#[cfg(feature = "hydrate")]
pub fn use_auth_heartbeat(expected: Role) -> ReadSignal<AuthStatus> {
    use wasm_bindgen::prelude::Closure;
    use wasm_bindgen::JsCast;

    let (status, set_status) = signal(AuthStatus::Checking);

    let run_check = move || {
        leptos::task::spawn_local(async move {
            let s = check_auth_once(expected, 8_000).await;
            set_status.set(s);
        });
    };

    // 1. Check now.
    run_check();

    // 2. Every 30s.
    leptos::leptos_dom::helpers::set_interval(run_check, std::time::Duration::from_secs(30));

    // 3. On tab focus / visibility regain. Keep the closure alive for the page
    //    lifetime by leaking it (the listener lives as long as the document).
    if let Some(win) = web_sys::window() {
        if let Some(doc) = win.document() {
            let cb = Closure::<dyn FnMut()>::new(run_check);
            let _ = doc
                .add_event_listener_with_callback("visibilitychange", cb.as_ref().unchecked_ref());
            cb.forget();
        }
    }

    status
}

/// SSR / non-hydrate: constant `Checking`. The shell renders the button
/// optimistically; the browser heartbeat replaces it after hydration.
#[cfg(not(feature = "hydrate"))]
pub fn use_auth_heartbeat(_expected: Role) -> ReadSignal<AuthStatus> {
    signal(AuthStatus::Checking).0
}

/// The shared staff `[Abmelden]` control with a LIVE auth-status chip. Every
/// role's shell renders this instead of a bare button so the operator can see
/// at a glance whether the session is still valid:
///   - green  "Admin/Küche/Fahrer"     — confirmed signed in,
///   - amber  "Server antwortet nicht" — heartbeat timed out (8s),
///   - red    "Sitzung abgelaufen"     — server says not signed in.
/// On `Expired` the chip becomes a link to the login page; clicking
/// `[Abmelden]` always does a clean server logout + full reload.
#[component]
pub fn LogoutButton(
    /// The role whose shell this is (for the heartbeat's expectation).
    role: Role,
) -> impl IntoView {
    let logout = ServerAction::<SessionLogout>::new();
    let status = use_auth_heartbeat(role);

    let do_logout = move |_| {
        logout.dispatch(SessionLogout {});
    };

    // After a successful logout the server cleared the cookie, but the
    // dispatch is an in-page fetch so the redirect it returns doesn't move
    // the browser. Force a full navigation to the login page so the auth
    // middleware re-checks the (now-absent) cookie on a real request.
    let login_path = role.login_path();
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if matches!(logout.value().get(), Some(Ok(()))) {
            if let Some(w) = web_sys::window() {
                let _ = w.location().set_href(login_path);
            }
        }
    });

    view! {
        <div class="auth-control">
            {move || {
                let s = status.get();
                let cls = format!("auth-chip {}", s.css_class());
                // When the server says the session is dead, offer the one-tap
                // path back in instead of a dead-end label.
                match s {
                    AuthStatus::Expired => view! {
                        <a class=cls href=login_path title="Sitzung serverseitig abgelaufen — neu anmelden">
                            <span class="dot"></span>
                            <span class="label">{s.label_de()}</span>
                        </a>
                    }.into_any(),
                    _ => view! {
                        <span class=cls title="Live-Sitzungsstatus">
                            <span class="dot"></span>
                            <span class="label">{s.label_de()}</span>
                        </span>
                    }.into_any(),
                }
            }}
            <button class="logout" on:click=do_logout>"Abmelden"</button>
        </div>
    }
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
        if current != Role::Admin {
            others.push(pill(Role::Admin, "Admin", "🏪"));
        }
        if current != Role::Kitchen {
            others.push(pill(Role::Kitchen, "Küche", "👨‍🍳"));
        }
        if current != Role::Driver {
            others.push(pill(Role::Driver, "Fahrer", "🛵"));
        }
        view! {
            <nav class="role-switcher" aria-label="Bereich wechseln">
                {pill(current, current.label_de(), match current {
                    Role::Admin => "🏪",
                    Role::Kitchen => "👨‍🍳",
                    Role::Driver => "🛵",
                    Role::SysAdmin => "🛠",
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

/// Per-tenant auth secrets (admin/kitchen/driver passwords), parsed from the
/// tenant's `.env`. Provided into request context by the tenant router so
/// every login checks the CURRENT tenant's password — not the process-global
/// one. `None` field = not set in that tenant's `.env`. In single-tenant
/// (Model A) this carries the global `.env` values, so behaviour is unchanged.
#[cfg(feature = "ssr")]
#[derive(Clone, Default)]
pub struct TenantAuth {
    /// Whose sessions these are (the tenant slug): signed into `dp_auth`, so
    /// a session from one shop is worthless at another.
    pub scope: String,
    pub admin_password: Option<String>,
    pub kitchen_password: Option<String>,
    pub driver_password: Option<String>,
}

#[cfg(feature = "ssr")]
pub mod ssr {
    use super::*;
    use leptos::prelude::ServerFnError;
    use leptos_axum::extract;
    use tower_cookies::{cookie::time::Duration, cookie::SameSite, Cookie, Cookies};

    /// Look up the expected password for `role`. Prefers the per-request
    /// `TenantAuth` (the current tenant's `.env`); falls back to the process
    /// env when it's absent (Model A / a tenant that didn't set the key).
    /// All three keep their dev defaults, as the login forms do (admin:
    /// main.rs's boot fallback); only SysAdmin has none.
    fn expected_password(auth: &TenantAuth, role: Role) -> Option<String> {
        match role {
            Role::Admin => auth
                .admin_password
                .clone()
                .or_else(|| std::env::var("ADMIN_PASSWORD").ok())
                .or_else(|| Some("admin".to_string())),
            Role::Kitchen => auth
                .kitchen_password
                .clone()
                .or_else(|| std::env::var("KITCHEN_PASSWORD").ok())
                .or_else(|| Some("kueche".to_string())),
            Role::Driver => auth
                .driver_password
                .clone()
                .or_else(|| std::env::var("DRIVER_PASSWORD").ok())
                .or_else(|| Some("fahrer".to_string())),
            // Process env only — never per tenant, never a default.
            Role::SysAdmin => std::env::var("SYS_ADMIN_PASSWORD")
                .ok()
                .filter(|p| !p.trim().is_empty()),
        }
    }

    /// The request's tenant auth (Leptos context), or the env fallback.
    fn context_auth() -> TenantAuth {
        leptos::prelude::use_context::<TenantAuth>().unwrap_or_default()
    }

    static SESSION_KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

    /// Load the session key from `path`, or create it there (32 random
    /// bytes, hex, owner-only). Call once at startup, before serving. If the
    /// file can't be read or written, a random key for this run is used:
    /// logins work until the next restart.
    pub fn init_session_key(path: &std::path::Path) {
        use rand::RngCore;

        let parse = |text: &str| -> Option<[u8; 32]> {
            let text = text.trim();
            if text.len() != 64 {
                return None;
            }
            let mut key = [0u8; 32];
            for (i, byte) in key.iter_mut().enumerate() {
                *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
            }
            Some(key)
        };
        let key = match std::fs::read_to_string(path) {
            Ok(text) => match parse(&text) {
                Some(key) => {
                    log::info!("session key loaded from {}", path.display());
                    key
                }
                None => {
                    log::error!(
                        "session key {} is not 64 hex digits — using a temporary key",
                        path.display()
                    );
                    temporary_key()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut key);
                match write_key(path, &hex(&key)) {
                    Ok(()) => log::info!("session key created at {}", path.display()),
                    Err(e) => log::error!(
                        "session key not saved at {}: {e} — sessions end at restart",
                        path.display()
                    ),
                }
                key
            }
            Err(e) => {
                log::error!(
                    "session key {} unreadable: {e} — using a temporary key",
                    path.display()
                );
                temporary_key()
            }
        };
        let _ = SESSION_KEY.set(key);
    }

    fn temporary_key() -> [u8; 32] {
        use rand::RngCore;
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        key
    }

    fn write_key(path: &std::path::Path, text: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(path)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The `dp_auth` value for `role` at this tenant: HMAC-SHA256 over the
    /// tenant, the role and the role's current password. `None` without a
    /// key or without a password for the role (no session possible).
    pub fn session_signature(auth: &TenantAuth, role: Role) -> Option<String> {
        use hmac::{Hmac, Mac};
        let key = SESSION_KEY.get()?;
        let password = expected_password(auth, role)?;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).ok()?;
        for part in [
            "dp_session v1",
            auth.scope.as_str(),
            role.as_str(),
            password.as_str(),
        ] {
            mac.update(part.as_bytes());
            mac.update(&[0]);
        }
        Some(hex(&mac.finalize().into_bytes()))
    }

    /// The role a `dp_session` / `dp_auth` pair authenticates as at this
    /// tenant, if the signature matches.
    pub fn verify_session(
        role: Option<&str>,
        signature: Option<&str>,
        auth: &TenantAuth,
    ) -> Option<Role> {
        let role = Role::parse(role?)?;
        let expected = session_signature(auth, role)?;
        let given = signature?.as_bytes();
        // Constant time: no early exit on the first differing byte.
        let same = expected.len() == given.len()
            && expected
                .as_bytes()
                .iter()
                .zip(given)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        same.then_some(role)
    }

    fn session_cookie(name: &'static str, value: String, max_age: Duration) -> Cookie<'static> {
        let mut c = Cookie::new(name, value);
        c.set_path("/");
        c.set_http_only(true);
        c.set_same_site(SameSite::Lax);
        c.set_max_age(max_age);
        c
    }

    /// Write the session cookies for `role` (365 days): `dp_session=<role>`
    /// plus its signature `dp_auth`. Shared by every role's login path so
    /// admin/kitchen/driver all get the same long-lived session — no more
    /// the old 8h `admin_session` surprise-logout.
    ///
    /// Staff devices (iPad on counter, kitchen tablet, driver phone) should
    /// stay logged in indefinitely between explicit logouts. Apple's WebView
    /// enforces a 7-day cap on *tracking* cookies, but our session cookie is
    /// first-party for the shop domain and exempt from that.
    pub async fn set_session(role: Role) -> Result<(), ServerFnError> {
        let signature = session_signature(&context_auth(), role).ok_or_else(|| {
            ServerFnError::new("Anmeldung derzeit nicht möglich (Sitzungsschlüssel)")
        })?;
        let cookies: Cookies = extract().await?;
        cookies.add(session_cookie(
            SESSION_COOKIE,
            role.as_str().to_string(),
            Duration::days(365),
        ));
        cookies.add(session_cookie(AUTH_COOKIE, signature, Duration::days(365)));
        Ok(())
    }

    /// Verify the password matches and write the role into the session cookie.
    /// Returns `Ok(true)` on success, `Ok(false)` for a wrong password.
    pub async fn try_login(role: Role, password: &str) -> Result<bool, ServerFnError> {
        let expected = expected_password(&context_auth(), role)
            .ok_or_else(|| ServerFnError::new("Passwort für diese Rolle nicht konfiguriert"))?;
        if password != expected {
            return Ok(false);
        }
        set_session(role).await?;
        Ok(true)
    }

    /// Clear the session cookies (role, signature and the legacy admin one).
    /// The delete attributes MUST match the ones set at login (Path=/,
    /// HttpOnly, SameSite=Lax) — a browser only overwrites/deletes a cookie
    /// whose key attributes line up; a mismatch leaves the original in place
    /// and the "logout" silently does nothing.
    pub async fn logout() -> Result<(), ServerFnError> {
        let cookies: Cookies = extract().await?;
        for name in [SESSION_COOKIE, AUTH_COOKIE, LEGACY_ADMIN_COOKIE] {
            cookies.add(session_cookie(name, String::new(), Duration::ZERO));
        }
        Ok(())
    }

    /// What role (if any) does the current request authenticate as?
    pub async fn current_role() -> Option<Role> {
        let cookies: Cookies = extract().await.ok()?;
        role_from_cookies(&cookies, &context_auth())
    }

    /// [`current_role`] for plain axum handlers, which have the request's
    /// `Cookies` and tenant but no leptos context.
    pub fn role_from_cookies(cookies: &Cookies, auth: &TenantAuth) -> Option<Role> {
        verify_session(
            cookies.get(SESSION_COOKIE).as_ref().map(|c| c.value()),
            cookies.get(AUTH_COOKIE).as_ref().map(|c| c.value()),
            auth,
        )
    }

    /// [`role_from_cookies`] from a raw `Cookie` header (middleware that runs
    /// outside the cookie manager).
    pub fn role_from_cookie_header(header: &str, auth: &TenantAuth) -> Option<Role> {
        let value = |name: &str| {
            header.split(';').find_map(|pair| {
                let (k, v) = pair.trim().split_once('=')?;
                (k == name).then_some(v)
            })
        };
        verify_session(value(SESSION_COOKIE), value(AUTH_COOKIE), auth)
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

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::ssr::{
        init_session_key, role_from_cookie_header, session_signature, verify_session,
    };
    use super::{Role, TenantAuth};

    fn auth(scope: &str, kitchen: &str) -> TenantAuth {
        TenantAuth {
            scope: scope.to_string(),
            admin_password: Some("admin-pw".to_string()),
            kitchen_password: Some(kitchen.to_string()),
            driver_password: Some("driver-pw".to_string()),
        }
    }

    fn with_key() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tmp/session-key-test");
        std::fs::create_dir_all(dir).unwrap();
        init_session_key(&std::path::Path::new(dir).join("session.key"));
    }

    #[test]
    fn only_a_signed_role_counts() {
        with_key();
        let davids = auth("davids", "k1");
        let sig = session_signature(&davids, Role::Kitchen).unwrap();
        assert_eq!(
            verify_session(Some("kitchen"), Some(&sig), &davids),
            Some(Role::Kitchen)
        );

        assert_eq!(
            verify_session(Some("admin"), None, &davids),
            None,
            "bare role cookie"
        );
        assert_eq!(
            verify_session(Some("admin"), Some(&sig), &davids),
            None,
            "kitchen signature as admin"
        );
        assert_eq!(
            verify_session(Some("kitchen"), Some("00"), &davids),
            None,
            "garbage signature"
        );
        assert_eq!(
            verify_session(Some("kitchen"), Some(&sig), &auth("other-shop", "k1")),
            None,
            "other shop"
        );
        assert_eq!(
            verify_session(Some("kitchen"), Some(&sig), &auth("davids", "k2")),
            None,
            "password changed"
        );
    }

    #[test]
    fn cookie_header_parsing() {
        with_key();
        let davids = auth("davids", "k1");
        let sig = session_signature(&davids, Role::Admin).unwrap();
        let header = format!("cart_id=abc; dp_session=admin; dp_auth={sig}");
        assert_eq!(role_from_cookie_header(&header, &davids), Some(Role::Admin));
        assert_eq!(role_from_cookie_header("dp_session=admin", &davids), None);
        assert_eq!(
            role_from_cookie_header("admin_session=ok", &davids),
            None,
            "legacy cookie no longer counts"
        );
        assert_eq!(role_from_cookie_header("", &davids), None);
    }
}
