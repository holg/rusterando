//
//  SettingsViewController.swift
//  Davids Pizzeria / Rusterando
//
//  Two view controllers covering the multi-shop settings UI:
//
//   - ShopsViewController: top-level list. One row per shop, tap to
//     activate, chevron / "Bearbeiten" to drill into ShopDetail.
//     Plus "+ Shop hinzufügen" and "Alle Daten löschen".
//   - ShopDetailViewController: per-shop credentials + URL + delete.
//     Replaces the old single-shop SettingsViewController.
//
//  Both screens share the WebViewController's `onShopChanged` callback:
//  whenever the active shop or its role changes, the WebView reloads
//  to the new (baseUrl + homePath).
//

import UIKit
import WebKit

// MARK: - Shops list

final class ShopsViewController: UIViewController {

    /// Called when the active shop / role changes so the host
    /// (WebViewController) can reload its WebView.
    var onShopChanged: ((Shop) -> Void)?

    private var tableView: UITableView!
    private var shops: [Shop] = []

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Shops"
        view.backgroundColor = .systemGroupedBackground
        navigationItem.rightBarButtonItem = UIBarButtonItem(
            barButtonSystemItem: .done,
            target: self,
            action: #selector(closeTapped)
        )

        tableView = UITableView(frame: .zero, style: .insetGrouped)
        tableView.translatesAutoresizingMaskIntoConstraints = false
        tableView.dataSource = self
        tableView.delegate = self
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "cell")
        view.addSubview(tableView)
        NSLayoutConstraint.activate([
            tableView.topAnchor.constraint(equalTo: view.topAnchor),
            tableView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            tableView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            tableView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])
        refresh()
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        // Coming back from ShopDetail — refresh in case the user
        // renamed / changed URL / added a password.
        refresh()
    }

    @objc private func closeTapped() { dismiss(animated: true) }

    private func refresh() {
        shops = ShopStore.all()
        tableView.reloadData()
    }

    /// Make this shop the active one and stay on screen so the user
    /// can pick a role inline (Section 1). The WebView doesn't reload
    /// until they pick a role — that's the dismiss trigger now.
    private func activate(_ shop: Shop) {
        ShopStore.setActiveShop(id: shop.id)
        refresh()
    }

    /// Inline-row role switch (Section 1). Same RoleSwitcher path that
    /// ShopDetail uses — POST password to <shop>/api/<role>_login,
    /// mirror Set-Cookie into WKWebView, update active role hint.
    /// Refreshes the table on success so the ✓ moves to the new row.
    private func activateRole(_ role: Role, in shop: Shop) {
        Task {
            do {
                try await RoleSwitcher.shared.switchTo(role, in: shop)
                await MainActor.run {
                    self.refresh()
                    if let next = ShopStore.shop(id: shop.id) {
                        self.onShopChanged?(next)
                        self.dismiss(animated: true)
                    }
                }
            } catch RoleSwitchError.wrongPassword {
                await MainActor.run {
                    self.showError("Gespeichertes Passwort wurde abgelehnt. Bitte in (i) → Gespeicherte Passwörter erneut eingeben.")
                }
            } catch RoleSwitchError.noPassword {
                await MainActor.run {
                    self.showError("Kein Passwort für \(role.label) gespeichert. (i) öffnen, um eines hinzuzufügen.")
                }
            } catch {
                await MainActor.run {
                    self.showError("Anmeldung fehlgeschlagen: \(error)")
                }
            }
        }
    }

    private func openDetail(for shop: Shop) {
        let detail = ShopDetailViewController(shopId: shop.id)
        detail.onShopChanged = { [weak self] s in
            self?.onShopChanged?(s)
            self?.dismiss(animated: true)
        }
        navigationController?.pushViewController(detail, animated: true)
    }

    private func promptAddShop() {
        let alert = UIAlertController(
            title: "Shop hinzufügen",
            message: "URL inklusive https:// — Passwörter danach im Shop-Detail.",
            preferredStyle: .alert
        )
        alert.addTextField {
            $0.placeholder = "Name (z.B. Mein Restaurant)"
            $0.autocapitalizationType = .words
        }
        alert.addTextField {
            $0.placeholder = "https://meinshop.de"
            $0.keyboardType = .URL
            $0.autocorrectionType = .no
            $0.autocapitalizationType = .none
        }
        alert.addAction(UIAlertAction(title: "Abbrechen", style: .cancel))
        alert.addAction(UIAlertAction(title: "Speichern", style: .default) { [weak self] _ in
            guard let self else { return }
            let name = alert.textFields?[0].text?.trimmingCharacters(in: .whitespaces) ?? ""
            let url = alert.textFields?[1].text?.trimmingCharacters(in: .whitespaces) ?? ""
            guard url.hasPrefix("http") else {
                self.showError("URL muss mit http:// oder https:// beginnen.")
                return
            }
            _ = ShopStore.addShop(name: name.isEmpty ? url : name, baseUrl: url)
            self.refresh()
        })
        present(alert, animated: true)
    }

    private func confirmWipeAll() {
        let alert = UIAlertController(
            title: "Alles löschen?",
            message: "Alle Shops, gespeicherten Passwörter und Session-Cookies werden entfernt.",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "Abbrechen", style: .cancel))
        alert.addAction(UIAlertAction(title: "Löschen", style: .destructive) { [weak self] _ in
            ShopStore.wipeAll()
            // Drop every cookie too so the next WebView load is anonymous.
            Task {
                let store = WKWebsiteDataStore.default().httpCookieStore
                for c in await store.allCookies() {
                    await store.deleteCookie(c)
                }
                await MainActor.run {
                    self?.refresh()
                    if let active = ShopStore.activeShop() {
                        self?.onShopChanged?(active)
                    }
                }
            }
        })
        present(alert, animated: true)
    }

    private func showError(_ msg: String) {
        let alert = UIAlertController(title: "Hinweis", message: msg, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        present(alert, animated: true)
    }
}

extension ShopsViewController: UITableViewDataSource, UITableViewDelegate {
    /// Section layout:
    ///   0: configured shops (one row each; tap to activate)
    ///   1: active shop's role picker — Kunde + saved staff roles.
    ///      Only present when activeShop != nil AND has at least one
    ///      activatable row (Kunde is always there, so it always shows
    ///      when there's an active shop). Tap a role = log in / log
    ///      out without leaving Settings.
    ///   2: "+ Shop hinzufügen"
    ///   3: destructive — "Alle Daten löschen"
    private var hasActiveShop: Bool { ShopStore.activeShop() != nil }
    private var rolesForActiveShop: [Role] {
        guard let s = ShopStore.activeShop() else { return [] }
        return [Role.customer] + ShopStore.savedRoles(shopId: s.id)
    }

    func numberOfSections(in tv: UITableView) -> Int { 4 }

    func tableView(_ tv: UITableView, titleForHeaderInSection section: Int) -> String? {
        switch section {
        case 0: return "Konfigurierte Shops"
        case 1:
            guard let s = ShopStore.activeShop(), hasActiveShop else { return nil }
            return "Aktive Rolle: \(s.displayLabel)"
        default: return nil
        }
    }

    func tableView(_ tv: UITableView, titleForFooterInSection section: Int) -> String? {
        switch section {
        case 0: return "Tippen, um zu wechseln. (i) öffnet Passwörter, URL und alle Rollen."
        case 1: return "Rollen ohne gespeichertes Passwort: (i) → Gespeicherte Passwörter."
        default: return nil
        }
    }

    func tableView(_ tv: UITableView, numberOfRowsInSection section: Int) -> Int {
        switch section {
        case 0: return max(shops.count, 1)
        case 1: return hasActiveShop ? rolesForActiveShop.count : 0
        case 2: return 1
        case 3: return 1
        default: return 0
        }
    }

    func tableView(_ tv: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = UITableViewCell(style: .subtitle, reuseIdentifier: "cell")
        cell.accessoryType = .none
        cell.textLabel?.textColor = .label
        cell.detailTextLabel?.text = nil
        cell.detailTextLabel?.textColor = .secondaryLabel

        switch indexPath.section {
        case 0:
            if shops.isEmpty {
                cell.textLabel?.text = "Noch kein Shop konfiguriert."
                cell.textLabel?.textColor = .secondaryLabel
                cell.selectionStyle = .none
            } else {
                let shop = shops[indexPath.row]
                let isActive = ShopStore.activeShop()?.id == shop.id
                // Prefix ✓ inline so the right side stays free for the
                // (i) button that opens the detail screen. Without
                // .detailDisclosureButton, tapping the chevron does
                // nothing — only swipe-edit would work.
                cell.textLabel?.text = isActive
                    ? "✓  " + shop.displayLabel
                    : shop.displayLabel
                cell.textLabel?.textColor = isActive ? .systemGreen : .label
                cell.detailTextLabel?.text = shop.baseUrl
                cell.accessoryType = .detailDisclosureButton
                cell.accessoryView = nil
                cell.selectionStyle = .default
            }
        case 1:
            // Active shop's role picker.
            if let s = ShopStore.activeShop() {
                let role = rolesForActiveShop[indexPath.row]
                cell.textLabel?.text = role.label
                if role == s.activeRole {
                    let mark = UILabel()
                    mark.text = "✓"
                    mark.textColor = .systemGreen
                    mark.font = .systemFont(ofSize: 18, weight: .bold)
                    mark.sizeToFit()
                    cell.accessoryView = mark
                }
                cell.selectionStyle = .default
            }
        case 2:
            cell.textLabel?.text = "+ Shop hinzufügen"
            cell.textLabel?.textColor = .systemBlue
            cell.selectionStyle = .default
        case 3:
            cell.textLabel?.text = "Alle Shops und Passwörter löschen"
            cell.textLabel?.textColor = .systemRed
            cell.selectionStyle = .default
        default:
            break
        }
        return cell
    }

    func tableView(_ tv: UITableView, didSelectRowAt indexPath: IndexPath) {
        tv.deselectRow(at: indexPath, animated: true)
        switch indexPath.section {
        case 0:
            if !shops.isEmpty {
                let shop = shops[indexPath.row]
                activate(shop)
            }
        case 1:
            // Tap an inline role row → log in / out for the active shop.
            guard let s = ShopStore.activeShop() else { return }
            let role = rolesForActiveShop[indexPath.row]
            activateRole(role, in: s)
        case 2:
            promptAddShop()
        case 3:
            confirmWipeAll()
        default:
            break
        }
    }

    func tableView(_ tv: UITableView, accessoryButtonTappedForRowWith indexPath: IndexPath) {
        if indexPath.section == 0, !shops.isEmpty {
            openDetail(for: shops[indexPath.row])
        }
    }

    // The disclosure-indicator alone doesn't fire accessoryButtonTapped;
    // for that we'd need .detailDisclosureButton. To keep the row
    // tappable AND the detail reachable, expose detail via a swipe
    // action on each shop row.
    func tableView(_ tv: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        guard indexPath.section == 0, !shops.isEmpty else { return nil }
        let shop = shops[indexPath.row]
        let edit = UIContextualAction(style: .normal, title: "Bearbeiten") { [weak self] _, _, done in
            self?.openDetail(for: shop)
            done(true)
        }
        edit.backgroundColor = .systemBlue
        return UISwipeActionsConfiguration(actions: [edit])
    }
}

// MARK: - Shop detail

final class ShopDetailViewController: UIViewController {

    var onShopChanged: ((Shop) -> Void)?

    private let shopId: String
    private var shop: Shop? { ShopStore.shop(id: shopId) }
    private var tableView: UITableView!

    init(shopId: String) {
        self.shopId = shopId
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) not supported") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemGroupedBackground
        title = shop?.displayLabel ?? "Shop"

        tableView = UITableView(frame: .zero, style: .insetGrouped)
        tableView.translatesAutoresizingMaskIntoConstraints = false
        tableView.dataSource = self
        tableView.delegate = self
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "cell")
        view.addSubview(tableView)
        NSLayoutConstraint.activate([
            tableView.topAnchor.constraint(equalTo: view.topAnchor),
            tableView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            tableView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            tableView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])

        if shop == nil {
            navigationController?.popViewController(animated: true)
        }
    }

    private func refresh() {
        title = shop?.displayLabel ?? "Shop"
        tableView.reloadData()
    }

    // MARK: Actions

    private func activate(role: Role) {
        guard let s = shop else { return }
        Task {
            do {
                try await RoleSwitcher.shared.switchTo(role, in: s)
                ShopStore.setActiveShop(id: s.id)
                await MainActor.run {
                    if let next = ShopStore.shop(id: s.id) {
                        self.onShopChanged?(next)
                    }
                }
            } catch RoleSwitchError.wrongPassword {
                await MainActor.run {
                    self.showError("Gespeichertes Passwort wurde abgelehnt. Bitte erneut eingeben.")
                    self.editPassword(for: role)
                }
            } catch {
                await MainActor.run {
                    self.showError("Anmeldung fehlgeschlagen: \(error)")
                }
            }
        }
    }

    private func editPassword(for role: Role) {
        guard let s = shop else { return }
        let existing = ShopStore.loadPassword(shopId: s.id, role: role)
        let alert = UIAlertController(
            title: role.label,
            message: existing == nil
                ? "Passwort eingeben — wird sicher im Schlüsselbund gespeichert."
                : "Passwort ändern. Leer lassen + Speichern, um es zu löschen.",
            preferredStyle: .alert
        )
        alert.addTextField { tf in
            tf.placeholder = "Passwort"
            tf.isSecureTextEntry = true
            tf.text = existing
        }
        alert.addAction(UIAlertAction(title: "Abbrechen", style: .cancel))
        alert.addAction(UIAlertAction(title: "Speichern", style: .default) { [weak self] _ in
            guard let self else { return }
            let pw = alert.textFields?.first?.text ?? ""
            if pw.isEmpty {
                ShopStore.deletePassword(shopId: s.id, role: role)
            } else {
                ShopStore.savePassword(shopId: s.id, role: role, password: pw)
            }
            self.refresh()
        })
        present(alert, animated: true)
    }

    private func editTextField(label: String, current: String, save: @escaping (String) -> Void) {
        let alert = UIAlertController(title: label, message: nil, preferredStyle: .alert)
        alert.addTextField { tf in
            tf.text = current
            if label == "URL" {
                tf.keyboardType = .URL
                tf.autocorrectionType = .no
                tf.autocapitalizationType = .none
            }
        }
        alert.addAction(UIAlertAction(title: "Abbrechen", style: .cancel))
        alert.addAction(UIAlertAction(title: "Speichern", style: .default) { _ in
            let value = alert.textFields?.first?.text ?? ""
            save(value)
        })
        present(alert, animated: true)
    }

    private func confirmDelete() {
        guard let s = shop else { return }
        let alert = UIAlertController(
            title: "Shop entfernen?",
            message: "„\(s.displayLabel)\" wird mit allen Passwörtern gelöscht.",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "Abbrechen", style: .cancel))
        alert.addAction(UIAlertAction(title: "Entfernen", style: .destructive) { [weak self] _ in
            ShopStore.removeShop(id: s.id)
            self?.navigationController?.popViewController(animated: true)
        })
        present(alert, animated: true)
    }

    private func showError(_ msg: String) {
        let alert = UIAlertController(title: "Hinweis", message: msg, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        present(alert, animated: true)
    }
}

extension ShopDetailViewController: UITableViewDataSource, UITableViewDelegate {
    /// Section layout:
    ///   0: Shop — Name + URL (editable inline).
    ///   1: "Diese Shop aktivieren".
    ///   2: Aktive Rolle.
    ///   3: Gespeicherte Passwörter.
    ///   4: Shop entfernen.
    func numberOfSections(in tv: UITableView) -> Int { 5 }

    func tableView(_ tv: UITableView, titleForHeaderInSection section: Int) -> String? {
        switch section {
        case 0: return "Shop"
        case 2: return "Aktive Rolle"
        case 3: return "Gespeicherte Passwörter"
        default: return nil
        }
    }

    func tableView(_ tv: UITableView, titleForFooterInSection section: Int) -> String? {
        switch section {
        case 2: return "„Kunde\" entspricht dem öffentlichen Shop ohne Login."
        case 3: return "Passwörter werden lokal im Schlüsselbund verschlüsselt gespeichert."
        default: return nil
        }
    }

    func tableView(_ tv: UITableView, numberOfRowsInSection section: Int) -> Int {
        switch section {
        case 0: return 2  // name, url
        case 1: return 1  // activate
        case 2: return 1 + ShopStore.savedRoles(shopId: shopId).count  // customer + staff with creds
        case 3: return Role.staffRoles.count
        case 4: return 1
        default: return 0
        }
    }

    func tableView(_ tv: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = UITableViewCell(style: .value1, reuseIdentifier: "cell")
        cell.accessoryType = .none
        cell.textLabel?.textColor = .label
        cell.detailTextLabel?.text = nil
        cell.detailTextLabel?.textColor = .secondaryLabel
        cell.accessoryView = nil

        guard let s = shop else { return cell }

        switch indexPath.section {
        case 0:
            if indexPath.row == 0 {
                cell.textLabel?.text = "Name"
                cell.detailTextLabel?.text = s.name
            } else {
                cell.textLabel?.text = "URL"
                cell.detailTextLabel?.text = s.baseUrl
            }
            cell.accessoryType = .disclosureIndicator
        case 1:
            cell.textLabel?.text = "Diese Shop aktivieren"
            cell.textLabel?.textColor = .systemBlue
        case 2:
            let activatable = [Role.customer] + ShopStore.savedRoles(shopId: shopId)
            let role = activatable[indexPath.row]
            cell.textLabel?.text = role.label
            if role == s.activeRole {
                let mark = UILabel()
                mark.text = "✓"
                mark.textColor = .systemGreen
                mark.font = .systemFont(ofSize: 18, weight: .bold)
                mark.sizeToFit()
                cell.accessoryView = mark
            }
        case 3:
            let role = Role.staffRoles[indexPath.row]
            cell.textLabel?.text = role.label
            let saved = ShopStore.loadPassword(shopId: shopId, role: role) != nil
            let badge = UILabel()
            badge.text = saved ? "✓ gespeichert" : "+ hinzufügen"
            badge.textColor = saved ? .systemGreen : .systemBlue
            badge.font = .systemFont(ofSize: 14)
            badge.sizeToFit()
            cell.accessoryView = badge
        case 4:
            cell.textLabel?.text = "Shop entfernen"
            cell.textLabel?.textColor = .systemRed
        default:
            break
        }
        return cell
    }

    func tableView(_ tv: UITableView, didSelectRowAt indexPath: IndexPath) {
        tv.deselectRow(at: indexPath, animated: true)
        guard let s = shop else { return }
        switch indexPath.section {
        case 0:
            if indexPath.row == 0 {
                editTextField(label: "Name", current: s.name) { [weak self] newValue in
                    var updated = s
                    updated.name = newValue.trimmingCharacters(in: .whitespaces)
                    ShopStore.updateShop(updated)
                    self?.refresh()
                }
            } else {
                editTextField(label: "URL", current: s.baseUrl) { [weak self] newValue in
                    let normalised = Shop.normaliseUrl(newValue)
                    guard normalised.hasPrefix("http") else {
                        self?.showError("URL muss mit http:// oder https:// beginnen.")
                        return
                    }
                    var updated = s
                    updated.baseUrl = normalised
                    ShopStore.updateShop(updated)
                    self?.refresh()
                }
            }
        case 1:
            ShopStore.setActiveShop(id: s.id)
            onShopChanged?(s)
            navigationController?.popViewController(animated: true)
        case 2:
            let activatable = [Role.customer] + ShopStore.savedRoles(shopId: shopId)
            let role = activatable[indexPath.row]
            activate(role: role)
        case 3:
            editPassword(for: Role.staffRoles[indexPath.row])
        case 4:
            confirmDelete()
        default:
            break
        }
    }
}

