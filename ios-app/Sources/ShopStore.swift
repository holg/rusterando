//
//  ShopStore.swift
//  Davids Pizzeria / Rusterando
//
//  Persisted shop list + per-shop credentials. Two stores:
//
//  - UserDefaults: list of [Shop] (non-secret metadata: id, name, URL,
//    active-role hint). Encoded as JSON for forward-compat.
//  - Keychain (kSecClassGenericPassword): one slot per `(shopId, role)`
//    holding the password. Service name "rusterando.shop-passwords";
//    account "<shopId>:<role>".
//
//  On first launch the list is seeded with one entry — the historical
//  Davids shop — so existing installs upgrade cleanly. After that
//  every mutation goes through this class so the in-memory cache
//  and persistent stores stay in sync.
//

import Foundation
import Security

enum ShopStore {

    private static let shopsKey = "rusterando.shops.v1"
    private static let activeShopKey = "rusterando.active_shop_id"
    private static let keychainService = "rusterando.shop-passwords"

    // MARK: - Bootstrap

    /// Seed strategy on first launch. Reads two optional Info.plist
    /// keys so the same source tree can ship either:
    ///
    ///  - The Davids build — Info.plist sets `SeedShopName` +
    ///    `SeedShopURL` so the app boots ready-to-use with Davids
    ///    pre-configured (single-shop customer never opens Settings).
    ///
    ///  - The brand-neutral Rusterando build — Info.plist omits
    ///    those keys; the app boots with an empty shop list and the
    ///    UI nudges the user toward "+ Shop hinzufügen". Intended for
    ///    multi-shop drivers / multi-shop clients.
    ///
    /// When both Info.plist keys are missing AND no shop list is
    /// stored yet, we fall back to the historical Davids seed so the
    /// existing TestFlight install upgrades without becoming empty.
    private static func bootstrappedShops() -> [Shop] {
        let info = Bundle.main.infoDictionary
        let name = info?["SeedShopName"] as? String
        let url = info?["SeedShopURL"] as? String
        if let url, let name {
            return [Shop(
                id: seedShopId(for: url),
                name: name,
                baseUrl: Shop.normaliseUrl(url),
                activeRole: .customer,
            )]
        }
        if let url {
            return [Shop(
                id: seedShopId(for: url),
                name: url,
                baseUrl: Shop.normaliseUrl(url),
                activeRole: .customer,
            )]
        }
        // No Info.plist seed configured — brand-neutral build. Boots
        // with an empty list; the Shops screen invites the user to
        // "+ Shop hinzufügen". A Davids-flavored target sets the
        // Info.plist seed keys to pre-configure one shop on first
        // launch.
        return []
    }

    /// Deterministic id for the seeded shop, derived from its URL. MUST be
    /// stable across launches + app versions: the seed is re-created on a
    /// genuine first launch, and Keychain passwords are keyed by
    /// `<shopId>:<role>` — a random UUID would orphan them every time. So the
    /// same shop always resolves to the same id, and saved logins survive.
    private static func seedShopId(for url: String) -> String {
        "seed-" + Shop.normaliseUrl(url)
    }

    // MARK: - Shops list (UserDefaults)

    /// The stored shop list. CRITICAL DATA-SAFETY CONTRACT:
    ///
    ///   - Key ABSENT (true first launch) → seed from the flavor + persist.
    ///   - Key PRESENT but undecodable → return **empty WITHOUT overwriting**.
    ///     A schema change or transient corruption must NEVER cause us to
    ///     re-seed on top of the user's real data — that's how an update wiped
    ///     saved accounts. We keep the raw bytes so a future migration can
    ///     still recover them, and we do NOT touch the active-shop pointer.
    ///   - Key PRESENT and decodable → return it.
    ///
    /// The previous implementation collapsed the middle case into the first,
    /// silently destroying stored shops (and orphaning Keychain passwords) on
    /// any decode failure.
    static func all() -> [Shop] {
        guard let data = UserDefaults.standard.data(forKey: shopsKey) else {
            // Genuinely nothing stored yet → first launch: seed + persist.
            let seeded = bootstrappedShops()
            saveAll(seeded)
            UserDefaults.standard.set(seeded.first?.id, forKey: activeShopKey)
            return seeded
        }
        guard let shops = try? JSONDecoder().decode([Shop].self, from: data) else {
            // Data exists but won't decode. DO NOT re-seed/overwrite — that
            // would wipe real accounts. Preserve the bytes untouched; surface
            // an empty list so the UI shows "add a shop" rather than crashing.
            NSLog("[shopstore] stored shops present but failed to decode — "
                + "NOT overwriting (preserving \(data.count) bytes for recovery)")
            return []
        }
        return shops
    }

    static func activeShop() -> Shop? {
        let shops = all()
        if let active = UserDefaults.standard.string(forKey: activeShopKey),
           let hit = shops.first(where: { $0.id == active }) {
            return hit
        }
        return shops.first
    }

    static func shop(id: String) -> Shop? {
        all().first { $0.id == id }
    }

    static func setActiveShop(id: String) {
        guard all().contains(where: { $0.id == id }) else { return }
        UserDefaults.standard.set(id, forKey: activeShopKey)
    }

    @discardableResult
    static func addShop(name: String, baseUrl: String) -> Shop {
        var shops = all()
        let shop = Shop.new(name: name, baseUrl: baseUrl)
        shops.append(shop)
        saveAll(shops)
        // First-ever shop becomes active.
        if UserDefaults.standard.string(forKey: activeShopKey) == nil {
            UserDefaults.standard.set(shop.id, forKey: activeShopKey)
        }
        return shop
    }

    static func updateShop(_ updated: Shop) {
        var shops = all()
        if let i = shops.firstIndex(where: { $0.id == updated.id }) {
            shops[i] = updated
            saveAll(shops)
        }
    }

    static func removeShop(id: String) {
        var shops = all()
        shops.removeAll { $0.id == id }
        saveAll(shops)

        // Drop all stored credentials for this shop.
        for role in Role.staffRoles {
            deletePassword(shopId: id, role: role)
        }
        UserDefaults.standard.removeObject(forKey: kitchenDisplayKey(id))

        // Pick a new active shop if we just removed it.
        if UserDefaults.standard.string(forKey: activeShopKey) == id {
            if let next = shops.first {
                UserDefaults.standard.set(next.id, forKey: activeShopKey)
            } else {
                UserDefaults.standard.removeObject(forKey: activeShopKey)
            }
        }
    }

    static func setActiveRole(shopId: String, role: Role) {
        guard var s = shop(id: shopId) else { return }
        s.activeRole = role
        updateShop(s)
    }

    // MARK: - Kitchen display (UserDefaults, per shop)

    /// Whether the Küche role starts on the kitchen display
    /// (`/kitchen/display`, the board screen for a kitchen iPad) instead of
    /// the order list (`/kitchen`). Kept outside [Shop] on purpose: a new
    /// field there changes the persisted shop JSON, which every installed
    /// version must keep decoding (see `all()`).
    static func kitchenOpensDisplay(shopId: String) -> Bool {
        UserDefaults.standard.bool(forKey: kitchenDisplayKey(shopId))
    }

    static func setKitchenOpensDisplay(shopId: String, _ on: Bool) {
        UserDefaults.standard.set(on, forKey: kitchenDisplayKey(shopId))
    }

    /// Where the shop opens for its active role.
    static func homePath(for shop: Shop) -> String {
        if shop.activeRole == .kitchen && kitchenOpensDisplay(shopId: shop.id) {
            return "/kitchen/display"
        }
        return shop.activeRole.homePath
    }

    private static func kitchenDisplayKey(_ shopId: String) -> String {
        "rusterando.kitchen_display.\(shopId)"
    }

    /// Drop every shop, every credential, every cached state.
    static func wipeAll() {
        for shop in all() {
            for role in Role.staffRoles {
                deletePassword(shopId: shop.id, role: role)
            }
            UserDefaults.standard.removeObject(forKey: kitchenDisplayKey(shop.id))
        }
        UserDefaults.standard.removeObject(forKey: shopsKey)
        UserDefaults.standard.removeObject(forKey: activeShopKey)
    }

    private static func saveAll(_ shops: [Shop]) {
        if let data = try? JSONEncoder().encode(shops) {
            UserDefaults.standard.set(data, forKey: shopsKey)
        }
    }

    // MARK: - Credentials (Keychain)

    static func savePassword(shopId: String, role: Role, password: String) {
        guard role.requiresPassword else { return }
        deletePassword(shopId: shopId, role: role)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account(shopId: shopId, role: role),
            kSecValueData as String: Data(password.utf8),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlock,
        ]
        let status = SecItemAdd(query as CFDictionary, nil)
        if status != errSecSuccess {
            NSLog("[shopstore] save \(shopId)/\(role.rawValue) failed: \(status)")
        }
    }

    static func loadPassword(shopId: String, role: Role) -> String? {
        guard role.requiresPassword else { return nil }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account(shopId: shopId, role: role),
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data,
              let pw = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        return pw
    }

    static func deletePassword(shopId: String, role: Role) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: account(shopId: shopId, role: role),
        ]
        SecItemDelete(query as CFDictionary)
    }

    /// Staff roles with a saved password for a given shop.
    static func savedRoles(shopId: String) -> [Role] {
        Role.staffRoles.filter { loadPassword(shopId: shopId, role: $0) != nil }
    }

    private static func account(shopId: String, role: Role) -> String {
        "\(shopId):\(role.rawValue)"
    }
}
