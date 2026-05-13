//
//  Shop.swift
//  Davids Pizzeria / Rusterando
//
//  One configured shop on this device. Mirrors the cross-platform
//  `shops_v1.json` schema and the Kotlin `Shop` data class one-for-one
//  so the persisted JSON shape is portable.
//

import Foundation

/// All four "modes" the app can be in. Customer is the absence of a
/// session cookie (no password needed); the three staff roles each
/// require a password stored in [ShopStore].
enum Role: String, CaseIterable, Codable {
    case customer
    case admin
    case kitchen
    case driver

    var label: String {
        switch self {
        case .customer: return "Kunde"
        case .admin: return "Admin"
        case .kitchen: return "Küche"
        case .driver: return "Fahrer"
        }
    }

    var homePath: String {
        switch self {
        case .customer: return "/"
        case .admin: return "/admin"
        case .kitchen: return "/kitchen"
        case .driver: return "/driver"
        }
    }

    var loginPath: String? {
        switch self {
        case .customer: return nil
        case .admin: return "/api/admin_login"
        case .kitchen: return "/api/kitchen_login"
        case .driver: return "/api/driver_login"
        }
    }

    var requiresPassword: Bool { loginPath != nil }

    static var staffRoles: [Role] { allCases.filter { $0.requiresPassword } }
}

/// One configured shop. Plain data: persisted list lives in UserDefaults
/// (non-secret metadata), passwords live in Keychain keyed by
/// `(shopId, role)`. Keeping passwords out of this struct lets us write
/// the shop list freely without touching the secure store on every
/// rename / URL edit.
struct Shop: Codable, Equatable {
    var id: String
    var name: String
    /// Without trailing slash. e.g. `https://davidspizzeria.de`.
    var baseUrl: String
    var activeRole: Role

    var displayLabel: String {
        name.trimmingCharacters(in: .whitespaces).isEmpty ? baseUrl : name
    }

    static func new(name: String, baseUrl: String) -> Shop {
        Shop(
            id: UUID().uuidString,
            name: name,
            baseUrl: Self.normaliseUrl(baseUrl),
            activeRole: .customer
        )
    }

    static func normaliseUrl(_ raw: String) -> String {
        var s = raw.trimmingCharacters(in: .whitespaces)
        while s.hasSuffix("/") { s.removeLast() }
        return s
    }
}
