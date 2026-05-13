//
//  WebViewController.swift
//  Davids Pizzeria
//
//  The main UI: a full-screen WKWebView pointed at davidspizzeria.de.
//  A small gear-shaped floating button (top-right) opens the native
//  settings screen where the user manages saved role passwords and
//  switches between Admin / Küche / Fahrer without re-typing.
//
//  Persistent cookies + this role-switching are why the native shell
//  exists at all.
//

import UIKit
import WebKit

final class WebViewController: UIViewController {

    private var webView: WKWebView!
    private var settingsButton: UIButton!

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.allowsInlineMediaPlayback = true

        webView = WKWebView(frame: .zero, configuration: config)
        webView.translatesAutoresizingMaskIntoConstraints = false
        webView.allowsBackForwardNavigationGestures = true
        webView.navigationDelegate = self
        webView.uiDelegate = self
        view.addSubview(webView)

        // Gear FAB. Sits over the WebView in the top-right; ~44pt
        // square so it's a solid tap target.
        settingsButton = UIButton(type: .system)
        settingsButton.translatesAutoresizingMaskIntoConstraints = false
        let gear = UIImage(systemName: "gearshape.fill")?
            .withConfiguration(UIImage.SymbolConfiguration(pointSize: 22, weight: .regular))
        settingsButton.setImage(gear, for: .normal)
        settingsButton.tintColor = .white
        settingsButton.backgroundColor = UIColor(red: 0.78, green: 0.06, blue: 0.18, alpha: 0.92)
        settingsButton.layer.cornerRadius = 22
        settingsButton.layer.shadowColor = UIColor.black.cgColor
        settingsButton.layer.shadowOpacity = 0.25
        settingsButton.layer.shadowOffset = CGSize(width: 0, height: 2)
        settingsButton.layer.shadowRadius = 4
        settingsButton.addTarget(self, action: #selector(openSettings), for: .touchUpInside)
        view.addSubview(settingsButton)

        NSLayoutConstraint.activate([
            webView.topAnchor.constraint(equalTo: view.topAnchor),
            webView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            webView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: view.trailingAnchor),

            settingsButton.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            settingsButton.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            settingsButton.widthAnchor.constraint(equalToConstant: 44),
            settingsButton.heightAnchor.constraint(equalToConstant: 44),
        ])

        loadInitialURL()
    }

    /// Decide where to land on first paint. The active shop's
    /// activeRole picks the home_path. When no shop is configured
    /// (Rusterando first launch), render an inline empty-state page
    /// instead of jumping to any specific URL — the user adds a
    /// shop via the gear button.
    private func loadInitialURL() {
        guard let shop = ShopStore.activeShop() else {
            renderEmptyState()
            return
        }
        loadShopHome(shop)
    }

    /// Load any URL. Used by SceneDelegate (universal link cold/warm
    /// start), the push-tap handler, and after a successful role switch.
    func loadURL(_ url: URL) {
        let req = URLRequest(url: url, cachePolicy: .useProtocolCachePolicy, timeoutInterval: 30)
        webView.load(req)
    }

    /// Load shop.baseUrl + activeRole.homePath. Called after the user
    /// closes settings with a shop / role change.
    func loadShopHome(_ shop: Shop) {
        guard let base = URL(string: shop.baseUrl) else {
            renderEmptyState()
            return
        }
        if let url = URL(string: shop.activeRole.homePath, relativeTo: base) {
            loadURL(url)
        }
    }

    /// First-launch / no-shop placeholder. A static HTML page that
    /// invites the user to tap the gear button and add a shop.
    /// Avoids hardcoding any specific tenant URL in the binary.
    private func renderEmptyState() {
        let html = """
        <!doctype html><html lang="de"><head><meta charset="utf-8">
        <meta name="viewport" content="width=device-width,initial-scale=1">
        <style>
          html,body{margin:0;height:100%;font-family:-apple-system,sans-serif;
            background:#fafafa;color:#222;display:flex;align-items:center;
            justify-content:center;text-align:center;padding:24px}
          h1{font-size:24px;margin:0 0 12px}
          p{margin:0;color:#666;line-height:1.4}
          .hint{margin-top:20px;font-size:14px;color:#888}
        </style></head><body><div>
          <h1>Willkommen</h1>
          <p>Noch kein Shop eingerichtet.</p>
          <p class="hint">Tippe oben rechts auf das Zahnrad, um einen Shop hinzuzufügen.</p>
        </div></body></html>
        """
        webView.loadHTMLString(html, baseURL: nil)
    }

    @objc private func openSettings() {
        let shopsVC = ShopsViewController()
        shopsVC.onShopChanged = { [weak self] shop in
            self?.loadShopHome(shop)
        }
        let nav = UINavigationController(rootViewController: shopsVC)
        nav.modalPresentationStyle = .formSheet
        present(nav, animated: true)
    }
}

extension WebViewController: WKNavigationDelegate {
    /// Route external URL schemes out of the WebView into the system:
    ///   - `tel:` → dialer
    ///   - `mailto:` → Mail
    ///   - `maps://` / `comgooglemaps://` / `google.navigation:` → Maps
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.allow); return
        }
        let scheme = url.scheme ?? ""
        if ["tel", "mailto", "maps", "comgooglemaps", "google.navigation"].contains(scheme) {
            UIApplication.shared.open(url, options: [:], completionHandler: nil)
            decisionHandler(.cancel)
            return
        }
        decisionHandler(.allow)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        NSLog("[webview] load failed: \(error)")
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        NSLog("[webview] provisional load failed: \(error)")
    }
}

extension WebViewController: WKUIDelegate {
    /// Open target="_blank" links in the same WebView (default
    /// behaviour ignores them). Matters for the "PDF in new tab" flow.
    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        if navigationAction.targetFrame == nil, let url = navigationAction.request.url {
            UIApplication.shared.open(url, options: [:], completionHandler: nil)
        }
        return nil
    }
}
