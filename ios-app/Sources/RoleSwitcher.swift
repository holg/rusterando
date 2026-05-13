//
//  RoleSwitcher.swift
//  Davids Pizzeria / Rusterando
//
//  Switches the WebView's active staff role within a given shop, without
//  making the user retype the password. Multi-shop edition: every call
//  is bound to a Shop, whose baseUrl chooses where the POST goes and
//  whose Keychain slot provides the password.
//
//  Flow:
//   1. Read the password from [ShopStore] keyed by (shopId, role).
//   2. POST `password=<value>` to `<shop.baseUrl>/api/<role>_login`.
//   3. Server replies with `Set-Cookie: dp_session=...`; import into
//      WKWebView's shared cookie store so the WebView sees it next.
//   4. Update the shop's activeRole in [ShopStore].
//

import Foundation
import WebKit

enum RoleSwitchError: Error {
    case noPassword
    case wrongPassword
    case network(String)
    case server(Int)
}

actor RoleSwitcher {
    static let shared = RoleSwitcher()
    private init() {}

    func switchTo(_ role: Role, in shop: Shop) async throws {
        if role == .customer {
            try await switchToCustomer(shop: shop)
            return
        }

        guard let password = ShopStore.loadPassword(shopId: shop.id, role: role) else {
            throw RoleSwitchError.noPassword
        }
        guard let loginPath = role.loginPath else {
            throw RoleSwitchError.noPassword
        }

        let body = "password=\(urlencode(password))"
        guard let url = URL(string: shop.baseUrl + loginPath) else {
            throw RoleSwitchError.network("invalid url: \(shop.baseUrl + loginPath)")
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        req.httpBody = body.data(using: .utf8)
        req.httpShouldHandleCookies = true

        let session = URLSession.shared
        let (data, resp): (Data, URLResponse)
        do {
            (data, resp) = try await session.data(for: req)
        } catch {
            throw RoleSwitchError.network(error.localizedDescription)
        }
        guard let http = resp as? HTTPURLResponse else {
            throw RoleSwitchError.network("no http response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw RoleSwitchError.server(http.statusCode)
        }

        // Leptos server-fn convention: 200 + "false" = wrong password.
        let trimmed = String(data: data, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        if trimmed == "false" {
            throw RoleSwitchError.wrongPassword
        }

        if let setCookies = http.value(forHTTPHeaderField: "Set-Cookie") {
            let cookies = HTTPCookie.cookies(
                withResponseHeaderFields: ["Set-Cookie": setCookies],
                for: url
            )
            await Self.importCookies(cookies)
        }

        ShopStore.setActiveRole(shopId: shop.id, role: role)
    }

    /// Customer mode: drop the staff session for this shop. POST to
    /// `<shop>/api/session_logout` (best-effort), then defensively
    /// delete session cookies from the WKWebView store so the next
    /// page load is anonymous.
    private func switchToCustomer(shop: Shop) async throws {
        guard let url = URL(string: shop.baseUrl + "/api/session_logout") else {
            // Fall through to local purge even if the URL is malformed.
            await Self.purgeSessionCookies(for: shop.baseUrl)
            ShopStore.setActiveRole(shopId: shop.id, role: .customer)
            return
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        req.httpBody = Data()
        _ = try? await URLSession.shared.data(for: req)

        await Self.purgeSessionCookies(for: shop.baseUrl)
        ShopStore.setActiveRole(shopId: shop.id, role: .customer)
    }

    /// Delete dp_session + admin_session cookies from the shared
    /// WKWebView store for a specific shop host. Anything else (cart,
    /// language pref) is left intact.
    @MainActor
    private static func purgeSessionCookies(for baseUrl: String) async {
        guard let host = URL(string: baseUrl)?.host else { return }
        let store = WKWebsiteDataStore.default().httpCookieStore
        let all = await store.allCookies()
        for c in all {
            if c.domain.hasSuffix(host),
               (c.name == "dp_session" || c.name == "admin_session")
            {
                await store.deleteCookie(c)
            }
        }
    }

    @MainActor
    private static func importCookies(_ cookies: [HTTPCookie]) async {
        let store = WKWebsiteDataStore.default().httpCookieStore
        for c in cookies {
            await store.setCookie(c)
        }
    }

    private nonisolated func urlencode(_ s: String) -> String {
        var allowed = CharacterSet.urlQueryAllowed
        allowed.remove(charactersIn: "&=+;")
        return s.addingPercentEncoding(withAllowedCharacters: allowed) ?? s
    }
}
