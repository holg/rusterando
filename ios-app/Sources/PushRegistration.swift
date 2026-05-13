//
//  PushRegistration.swift
//  Davids Pizzeria
//
//  Sends the APNs device token to /api/register_push_token. The role
//  is read from the WebView's cookies, so the user must have logged in
//  to /admin, /kitchen or /driver before push starts working — that's
//  the design (login binds device-to-role).
//

import Foundation
import UIKit
import WebKit

actor PushRegistration {
    static let shared = PushRegistration()
    private init() {}

    /// Per-shop last-registered token. Keeps us from spamming each
    /// shop's `register_push_token` with the same value on every
    /// launch. Keyed by shop id.
    private var lastRegistered: [String: String] = [:]

    /// Called from AppDelegate when iOS hands us a token. Multi-shop:
    /// every shop whose user is logged in (dp_session cookie present
    /// for that host) gets its own register call. APNs gives the
    /// device one token; each shop's push_devices table stores it
    /// independently.
    func register(token: String) async {
        let shops = ShopStore.all()
        if shops.isEmpty {
            NSLog("[push] no shops configured — nothing to register")
            return
        }
        for shop in shops {
            await registerWith(shop: shop, token: token)
        }
    }

    private func registerWith(shop: Shop, token: String) async {
        guard lastRegistered[shop.id] != token else {
            NSLog("[push] token unchanged for \(shop.displayLabel), skipping")
            return
        }
        guard let role = await currentSessionRole(for: shop) else {
            NSLog("[push] no dp_session for \(shop.displayLabel) — skipping")
            return
        }

        let body: [String: Any] = [
            "token": token,
            "role": role,
            "platform": "ios",
            "label": UIDevice.current.name,
        ]

        guard let url = URL(string: shop.baseUrl + "/api/register_push_token") else {
            NSLog("[push] bad register url for \(shop.displayLabel)")
            return
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        req.httpBody = body
            .map { k, v in "\(k)=\(urlencode("\(v)"))" }
            .joined(separator: "&")
            .data(using: .utf8)

        // Send the shop's cookies along so the server can authenticate
        // the registration against the existing role session.
        let cookies = await sessionCookies(for: shop)
        if !cookies.isEmpty {
            let header = HTTPCookie.requestHeaderFields(with: cookies)
            for (k, v) in header { req.setValue(v, forHTTPHeaderField: k) }
        }

        do {
            let (_, resp) = try await URLSession.shared.data(for: req)
            if let http = resp as? HTTPURLResponse, (200..<300).contains(http.statusCode) {
                lastRegistered[shop.id] = token
                NSLog("[push] registered with \(shop.displayLabel), role=\(role)")
            } else {
                NSLog("[push] \(shop.displayLabel) HTTP \(((resp as? HTTPURLResponse)?.statusCode).map(String.init) ?? "?")")
            }
        } catch {
            NSLog("[push] \(shop.displayLabel) network error: \(error)")
        }
    }

    /// Reads `dp_session` from WKWebView's shared cookie store for a
    /// specific shop host. Values are "admin" / "kitchen" / "driver".
    /// Legacy `admin_session=ok` is honoured too.
    @MainActor
    private func currentSessionRole(for shop: Shop) async -> String? {
        guard let host = URL(string: shop.baseUrl)?.host else { return nil }
        let cookies = await WKWebsiteDataStore.default().httpCookieStore.allCookies()
        for c in cookies where c.domain.hasSuffix(host) {
            if c.name == "dp_session", ["admin", "kitchen", "driver"].contains(c.value) {
                return c.value
            }
            if c.name == "admin_session", c.value == "ok" {
                return "admin"
            }
        }
        return nil
    }

    @MainActor
    private func sessionCookies(for shop: Shop) async -> [HTTPCookie] {
        guard let host = URL(string: shop.baseUrl)?.host else { return [] }
        let all = await WKWebsiteDataStore.default().httpCookieStore.allCookies()
        return all.filter { $0.domain.hasSuffix(host) }
    }

    private nonisolated func urlencode(_ s: String) -> String {
        var allowed = CharacterSet.urlQueryAllowed
        allowed.remove(charactersIn: "&=+;")
        return s.addingPercentEncoding(withAllowedCharacters: allowed) ?? s
    }
}
