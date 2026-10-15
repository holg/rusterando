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

import QuickLook
import UIKit
import WebKit

final class WebViewController: UIViewController {

    private var webView: WKWebView!
    private var settingsButton: UIButton!
    private var reloadButton: UIButton!
    private let refreshControl = UIRefreshControl()

    /// Files (Buchhaltung PDFs, CSV) are downloaded with the WebView's own
    /// session and shown in QuickLook (share / save to Files / print).
    /// Handing them to Safari instead would hit the login: Safari has no
    /// staff session, on purpose.
    private let downloadSpinner = UIActivityIndicatorView(style: .large)
    private var downloadDestinations: [ObjectIdentifier: URL] = [:]
    private var previewFileURL: URL?

    /// Follows the page URL, including the web app's client-side route
    /// changes (pushState), which never reach the navigation delegate.
    private var urlObservation: NSKeyValueObservation?
    /// When the app last went to the background; see `appWillEnterForeground`.
    private var backgroundedAt: Date?

    /// Away longer than this, a live board reloads on return.
    private static let resumeReloadAfter: TimeInterval = 120

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.allowsInlineMediaPlayback = true
        // The kitchen display's gong plays without a first tap on the page.
        // Browsers require one, so a kitchen iPad left on the display would
        // only flash. The audio session (AppDelegate) lets it sound with the
        // device muted.
        config.mediaTypesRequiringUserActionForPlayback = []

        webView = WKWebView(frame: .zero, configuration: config)
        webView.translatesAutoresizingMaskIntoConstraints = false
        webView.allowsBackForwardNavigationGestures = true
        webView.navigationDelegate = self
        webView.uiDelegate = self
        view.addSubview(webView)

        // Pull-to-refresh. The app is a chrome-less WebView (no address bar,
        // no browser reload), so without this a hung or post-deploy-stale page
        // can't be reloaded. Attaching the control to the WebView's own scroll
        // view gives the familiar pull-down gesture on every page.
        refreshControl.addTarget(self, action: #selector(reloadPage), for: .valueChanged)
        webView.scrollView.refreshControl = refreshControl

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

        // Reload FAB, directly under the gear. Same visual language (round,
        // brand-red) so the two read as one control stack. Backs up the
        // pull-to-refresh gesture — a page that's scrolled or has no scroll
        // area still has a one-tap reload.
        reloadButton = UIButton(type: .system)
        reloadButton.translatesAutoresizingMaskIntoConstraints = false
        let arrows = UIImage(systemName: "arrow.clockwise")?
            .withConfiguration(UIImage.SymbolConfiguration(pointSize: 20, weight: .regular))
        reloadButton.setImage(arrows, for: .normal)
        reloadButton.tintColor = .white
        reloadButton.backgroundColor = UIColor(red: 0.78, green: 0.06, blue: 0.18, alpha: 0.92)
        reloadButton.layer.cornerRadius = 22
        reloadButton.layer.shadowColor = UIColor.black.cgColor
        reloadButton.layer.shadowOpacity = 0.25
        reloadButton.layer.shadowOffset = CGSize(width: 0, height: 2)
        reloadButton.layer.shadowRadius = 4
        reloadButton.accessibilityLabel = "Seite neu laden"
        reloadButton.addTarget(self, action: #selector(reloadPage), for: .touchUpInside)
        view.addSubview(reloadButton)

        // Big PDFs (Belege over a month) take a few seconds to render on
        // the server — show that the tap registered.
        downloadSpinner.translatesAutoresizingMaskIntoConstraints = false
        downloadSpinner.hidesWhenStopped = true
        view.addSubview(downloadSpinner)

        NSLayoutConstraint.activate([
            webView.topAnchor.constraint(equalTo: view.topAnchor),
            webView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            webView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: view.trailingAnchor),

            settingsButton.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            settingsButton.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
            settingsButton.widthAnchor.constraint(equalToConstant: 44),
            settingsButton.heightAnchor.constraint(equalToConstant: 44),

            reloadButton.topAnchor.constraint(equalTo: settingsButton.bottomAnchor, constant: 10),
            reloadButton.trailingAnchor.constraint(equalTo: settingsButton.trailingAnchor),
            reloadButton.widthAnchor.constraint(equalToConstant: 44),
            reloadButton.heightAnchor.constraint(equalToConstant: 44),

            downloadSpinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            downloadSpinner.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])

        urlObservation = webView.observe(\.url, options: [.new]) { [weak self] _, _ in
            DispatchQueue.main.async { self?.applyPageMode() }
        }
        NotificationCenter.default.addObserver(
            self, selector: #selector(appDidEnterBackground),
            name: UIApplication.didEnterBackgroundNotification, object: nil
        )
        NotificationCenter.default.addObserver(
            self, selector: #selector(appWillEnterForeground),
            name: UIApplication.willEnterForegroundNotification, object: nil
        )

        loadInitialURL()
    }

    // MARK: - Page modes (boards, kitchen display)

    /// What the current page needs from the shell.
    private struct PageMode {
        /// `/kitchen/display`: one full-window touch canvas.
        var isKitchenDisplay = false
        /// Keep the screen on: the kitchen pages and the admin order board,
        /// the screens left open on a kitchen / counter iPad. Not the driver
        /// board — that's a phone on the road.
        var keepAwake = false
        /// A live board: its server stream dies while the app is in the
        /// background, so it reloads on return. Lists only — no page with a
        /// half-typed message gets reloaded.
        var reloadOnResume = false
    }

    private static func pageMode(for url: URL?) -> PageMode {
        guard let url, let scheme = url.scheme, scheme.hasPrefix("http") else { return PageMode() }
        var path = url.path
        while path.count > 1 && path.hasSuffix("/") { path.removeLast() }
        return PageMode(
            isKitchenDisplay: path == "/kitchen/display",
            keepAwake: path == "/kitchen" || path.hasPrefix("/kitchen/")
                || path == "/admin/orders" || path.hasPrefix("/admin/orders/"),
            reloadOnResume: ["/kitchen", "/kitchen/display", "/admin/orders", "/driver"].contains(path)
        )
    }

    private func applyPageMode() {
        let mode = Self.pageMode(for: webView.url)
        UIApplication.shared.isIdleTimerDisabled = mode.keepAwake

        // The display is a touch canvas: no pull-to-refresh, rubber band or
        // edge-swipe "back" to yank it away mid-service. The buttons stay —
        // they're the way to settings — but recede; on most screens they sit
        // in the black bars around the 480×272 canvas anyway.
        let display = mode.isKitchenDisplay
        webView.scrollView.refreshControl = display ? nil : refreshControl
        webView.scrollView.bounces = !display
        webView.allowsBackForwardNavigationGestures = !display
        settingsButton.alpha = display ? 0.35 : 1
        reloadButton.alpha = display ? 0.35 : 1
    }

    @objc private func appDidEnterBackground() {
        backgroundedAt = Date()
    }

    /// iOS cuts the page's live streams while the app is in the background.
    /// They reconnect on their own, but with a backoff, and a board can show
    /// stale orders until then — so after a longer break, reload outright.
    @objc private func appWillEnterForeground() {
        defer { backgroundedAt = nil }
        guard let since = backgroundedAt,
              Date().timeIntervalSince(since) > Self.resumeReloadAfter,
              Self.pageMode(for: webView.url).reloadOnResume
        else { return }
        NSLog("[webview] back after \(Int(Date().timeIntervalSince(since)))s on a live board — reloading")
        webView.reload()
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

    /// Load the shop's home for its active role (see `ShopStore.homePath`).
    /// Called after the user closes settings with a shop / role change.
    func loadShopHome(_ shop: Shop) {
        guard let base = URL(string: shop.baseUrl) else {
            renderEmptyState()
            return
        }
        if let url = URL(string: ShopStore.homePath(for: shop), relativeTo: base) {
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

    /// Reload the current page. Reused by the reload FAB and the pull-to-
    /// refresh control. If the WebView has no committed page yet (e.g. the
    /// empty-state HTML, or a failed first load), fall back to re-loading the
    /// active shop's home so the button is never a dead no-op.
    @objc private func reloadPage() {
        if webView.url != nil {
            webView.reload()
        } else if let shop = ShopStore.activeShop() {
            loadShopHome(shop)
        } else {
            // No shop and no page — nothing to reload; just stop the spinner.
            refreshControl.endRefreshing()
        }
    }

    /// Same host as the page we're on, or the active shop — links there
    /// carry the staff session, so they must stay in this WebView.
    private func isShopURL(_ url: URL) -> Bool {
        guard let host = url.host?.lowercased() else { return false }
        let shopHost = ShopStore.activeShop().flatMap { URL(string: $0.baseUrl)?.host }
        return [webView.url?.host, shopHost].compactMap { $0?.lowercased() }.contains(host)
    }

    private func showPreview(_ file: URL) {
        previewFileURL = file
        let preview = QLPreviewController()
        preview.dataSource = self
        (presentedViewController ?? self).present(preview, animated: true)
    }

    private func stopSpinnerUnlessDownloading() {
        if downloadDestinations.isEmpty { downloadSpinner.stopAnimating() }
    }

    private func showDownloadError(_ error: Error) {
        let alert = UIAlertController(
            title: "Download fehlgeschlagen",
            message: error.localizedDescription,
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        present(alert, animated: true)
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
        // <a download> (CSV export).
        if navigationAction.shouldPerformDownload {
            decisionHandler(.download)
            return
        }
        decisionHandler(.allow)
    }

    /// PDFs and CSVs become downloads instead of replacing the page: the
    /// admin stays where it was and the file opens in QuickLook.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationResponse: WKNavigationResponse,
        decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void
    ) {
        let mime = navigationResponse.response.mimeType?.lowercased() ?? ""
        let disposition = (navigationResponse.response as? HTTPURLResponse)?
            .value(forHTTPHeaderField: "Content-Disposition")?.lowercased() ?? ""
        let isFile = mime == "application/pdf" || mime == "text/csv"
            || disposition.hasPrefix("attachment")
        if navigationResponse.isForMainFrame && isFile {
            decisionHandler(.download)
        } else {
            decisionHandler(.allow)
        }
    }

    func webView(
        _ webView: WKWebView,
        navigationAction: WKNavigationAction,
        didBecome download: WKDownload
    ) {
        download.delegate = self
    }

    func webView(
        _ webView: WKWebView,
        navigationResponse: WKNavigationResponse,
        didBecome download: WKDownload
    ) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        refreshControl.endRefreshing()
        stopSpinnerUnlessDownloading()
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        NSLog("[webview] load failed: \(error)")
        refreshControl.endRefreshing()
    }

    /// iOS kills the page's content process under memory pressure, often
    /// while the app is in the background; the WebView then stays blank.
    /// Reload rather than leave a kitchen screen empty.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        NSLog("[webview] content process terminated — reloading")
        reloadPage()
    }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        NSLog("[webview] provisional load failed: \(error)")
        refreshControl.endRefreshing()
        // Also how a navigation ends when it turned into a download.
        stopSpinnerUnlessDownloading()
    }
}

extension WebViewController: WKUIDelegate {
    /// target="_blank" links: our own shop's (the Buchhaltung PDFs) load
    /// in this WebView, where the session is — a PDF then becomes a
    /// download (see navigationResponse above). Other sites go to Safari.
    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        if navigationAction.targetFrame == nil, let url = navigationAction.request.url {
            if isShopURL(url) {
                // Spins from the tap: the server renders the PDF before
                // its first byte arrives.
                downloadSpinner.startAnimating()
                webView.load(navigationAction.request)
            } else {
                UIApplication.shared.open(url, options: [:], completionHandler: nil)
            }
        }
        return nil
    }
}

extension WebViewController: WKDownloadDelegate {
    func download(
        _ download: WKDownload,
        decideDestinationUsing response: URLResponse,
        suggestedFilename: String,
        completionHandler: @escaping (URL?) -> Void
    ) {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("downloads", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let dest = dir.appendingPathComponent(suggestedFilename)
        try? FileManager.default.removeItem(at: dest)
        downloadDestinations[ObjectIdentifier(download)] = dest
        downloadSpinner.startAnimating()
        completionHandler(dest)
    }

    func downloadDidFinish(_ download: WKDownload) {
        let dest = downloadDestinations.removeValue(forKey: ObjectIdentifier(download))
        stopSpinnerUnlessDownloading()
        if let dest { showPreview(dest) }
    }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        downloadDestinations.removeValue(forKey: ObjectIdentifier(download))
        stopSpinnerUnlessDownloading()
        showDownloadError(error)
    }
}

extension WebViewController: QLPreviewControllerDataSource {
    func numberOfPreviewItems(in controller: QLPreviewController) -> Int {
        previewFileURL == nil ? 0 : 1
    }

    func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem {
        (previewFileURL ?? URL(fileURLWithPath: "/")) as NSURL
    }
}
