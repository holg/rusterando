package eu.trahe.rusterando

import android.annotation.SuppressLint
import android.app.Activity
import android.app.DownloadManager
import android.content.Intent
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.os.SystemClock
import android.util.Log
import android.view.Gravity
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.CookieManager
import android.webkit.URLUtil
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.FrameLayout
import android.widget.Toast
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.ActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.WindowCompat

/**
 * Single-Activity WebView shell pointed at the configured shop
 * (BuildConfig.START_URL, set per product flavor in build.gradle.kts).
 *
 * Counterpart to ios-app/Sources/WebViewController.swift. Full-screen
 * WebView, persistent cookies for role logins, back gesture wired to
 * WebView history, and a top-right gear FAB that opens
 * [SettingsActivity]. When the settings activity finishes with a new
 * active role, the WebView reloads to that role's home path.
 */
class MainActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private lateinit var gearButton: Button
    private lateinit var reloadButton: Button
    private lateinit var settingsLauncher: androidx.activity.result.ActivityResultLauncher<Intent>

    /** The current page's needs, kept in step with the URL (see [applyPageMode]). */
    private var pageMode = PageMode()
    /** `elapsedRealtime` at the last onPause; see [onResume]. */
    private var pausedAt: Long? = null

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Edge-to-edge: the WebView extends behind the status bar so the
        // site's red header bleeds into it like on iOS. WindowCompat is
        // the modern replacement for the deprecated SYSTEM_UI_FLAG_* API.
        WindowCompat.setDecorFitsSystemWindows(window, false)

        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.databaseEnabled = true
            settings.mediaPlaybackRequiresUserGesture = false
            settings.userAgentString =
                "${settings.userAgentString} ${userAgentSuffix()}"

            webViewClient = ShopWebViewClient(BuildConfig.ASSOCIATED_DOMAIN, ::applyPageMode)
            setDownloadListener(::download)
        }

        // Persistent cookies — the whole point of running native instead
        // of a PWA. Without this, the role session would die on every
        // cold start because the WebView's cookie jar resets.
        CookieManager.getInstance().apply {
            setAcceptCookie(true)
            setAcceptThirdPartyCookies(webView, true)
        }

        // Compose: WebView fills the activity, gear FAB overlays top-right.
        val container = FrameLayout(this).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
            addView(
                webView,
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                ),
            )
            gearButton = buildGearButton()
            reloadButton = buildReloadButton()
            addView(gearButton)
            addView(reloadButton)
        }
        setContentView(container)

        // First-launch URL: pick the active role's home_path if one is
        // signed in, else the public site root. Warm starts restore
        // WebView history via onRestoreInstanceState below.
        if (savedInstanceState == null) {
            loadInitialUrl()
        }

        // Pre-register the settings launcher so the listener has it ready.
        settingsLauncher = registerForActivityResult(
            ActivityResultContracts.StartActivityForResult(),
            ::onSettingsClosed,
        )

        onBackPressedDispatcher.addCallback(
            this,
            object : OnBackPressedCallback(true) {
                override fun handleOnBackPressed() {
                    // The kitchen display is a kiosk screen: a stray edge
                    // swipe must not navigate away from it mid-service.
                    if (pageMode.isKitchenDisplay) return
                    if (webView.canGoBack()) {
                        webView.goBack()
                    } else {
                        isEnabled = false
                        onBackPressedDispatcher.onBackPressed()
                    }
                }
            },
        )
    }

    private fun loadInitialUrl() {
        val shop = ShopStore.activeShop(this)
        val base = (shop?.baseUrl ?: BuildConfig.START_URL).trimEnd('/')
        val url = if (shop != null && shop.activeRole != Role.Customer) {
            base + ShopStore.homePath(this, shop)
        } else {
            "$base/"
        }
        webView.loadUrl(url)
    }

    /**
     * Follow the page: keep the screen on for the kitchen pages and the admin
     * order board, and on the kitchen display let the buttons recede (they
     * stay — they're the way to settings). Twin of the iOS `applyPageMode`.
     */
    private fun applyPageMode(url: String?) {
        pageMode = pageModeFor(url)
        if (pageMode.keepAwake) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
        val alpha = if (pageMode.isKitchenDisplay) 0.35f else 1f
        gearButton.alpha = alpha
        reloadButton.alpha = alpha
    }

    /** Build the gear FAB. ~44 dp square, top-right with safe-area inset. */
    private fun buildGearButton(): Button {
        val sizeDp = dp(44)
        val marginDp = dp(12)
        // We place it 12dp below the status-bar inset; the default
        // FrameLayout Gravity respects the cutout area.
        val params = FrameLayout.LayoutParams(sizeDp, sizeDp).apply {
            gravity = Gravity.TOP or Gravity.END
            topMargin = marginDp + statusBarHeight()
            rightMargin = marginDp
        }

        // Circular red disc matching the iOS app's gear button colour.
        val bg = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(Color.parseColor("#C61D2EEB"))  // 0xEB alpha = ~0.92
        }

        return Button(this).apply {
            layoutParams = params
            text = "⚙"
            textSize = 20f
            setTextColor(Color.WHITE)
            background = bg
            elevation = dp(4).toFloat()
            contentDescription = getString(R.string.settings_open_content_description)
            setOnClickListener {
                settingsLauncher.launch(Intent(this@MainActivity, ShopsActivity::class.java))
            }
        }
    }

    /**
     * Reload FAB, stacked directly under the gear. The app is a chrome-less
     * WebView (no address bar, no browser reload), so without this a hung or
     * post-deploy-stale page can't be reloaded. Matches the iOS reload button.
     */
    private fun buildReloadButton(): Button {
        val sizeDp = dp(44)
        val marginDp = dp(12)
        val params = FrameLayout.LayoutParams(sizeDp, sizeDp).apply {
            gravity = Gravity.TOP or Gravity.END
            // Gear is at (statusBar + 12); sit 44 + 10dp below it.
            topMargin = marginDp + statusBarHeight() + sizeDp + dp(10)
            rightMargin = marginDp
        }

        val bg = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(Color.parseColor("#C61D2EEB"))  // same red as the gear
        }

        return Button(this).apply {
            layoutParams = params
            text = "⟳"
            textSize = 20f
            setTextColor(Color.WHITE)
            background = bg
            elevation = dp(4).toFloat()
            contentDescription = getString(R.string.reload_page_content_description)
            setOnClickListener { reloadPage() }
        }
    }

    /**
     * Reload the current page. Falls back to the active shop's home when the
     * WebView has no committed URL yet, so the button is never a dead no-op.
     */
    private fun reloadPage() {
        if (webView.url != null) {
            webView.reload()
        } else {
            loadInitialUrl()
        }
    }

    private fun onSettingsClosed(result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) return
        // Reload using the new active shop's baseUrl + role. Whichever
        // path the user took inside ShopsActivity / ShopDetailActivity,
        // the contract is the same: RESULT_OK + EXTRA_SHOP_ID means the
        // active shop changed (or its role did).
        result.data?.getStringExtra(ShopsActivity.EXTRA_SHOP_ID)
        loadInitialUrl()
    }

    private fun statusBarHeight(): Int {
        val id = resources.getIdentifier("status_bar_height", "dimen", "android")
        return if (id > 0) resources.getDimensionPixelSize(id) else dp(24)
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        webView.saveState(outState)
    }

    override fun onRestoreInstanceState(savedInstanceState: Bundle) {
        super.onRestoreInstanceState(savedInstanceState)
        webView.restoreState(savedInstanceState)
    }

    override fun onPause() {
        super.onPause()
        // Flush cookies to disk so a process kill in the background
        // (Doze, low-memory) doesn't lose role sessions.
        CookieManager.getInstance().flush()
        pausedAt = SystemClock.elapsedRealtime()
    }

    /**
     * A live board's server stream dies while the app is in the background.
     * It reconnects on its own, but with a backoff, and the board can show
     * stale orders until then — so after a longer break, reload outright.
     */
    override fun onResume() {
        super.onResume()
        val since = pausedAt ?: return
        pausedAt = null
        val away = SystemClock.elapsedRealtime() - since
        if (away > RESUME_RELOAD_AFTER_MS && pageMode.reloadOnResume) {
            Log.i("MainActivity", "back after ${away / 1000}s on a live board — reloading")
            webView.reload()
        }
    }

    override fun onDestroy() {
        // Detach + destroy so the WebView's render process doesn't leak
        // beyond the Activity's lifetime.
        (webView.parent as? ViewGroup)?.removeView(webView)
        webView.destroy()
        super.onDestroy()
    }

    /**
     * Buchhaltung PDFs / CSV export. The WebView can't show a PDF and
     * the system browser has no staff session (on purpose), so fetch it
     * with the WebView's cookies via DownloadManager — the "done"
     * notification opens it.
     */
    private fun download(
        url: String,
        userAgent: String,
        contentDisposition: String?,
        mimeType: String?,
        @Suppress("UNUSED_PARAMETER") contentLength: Long,
    ) {
        val name = URLUtil.guessFileName(url, contentDisposition, mimeType)
        val request = DownloadManager.Request(Uri.parse(url)).apply {
            CookieManager.getInstance().getCookie(url)?.let { addRequestHeader("Cookie", it) }
            addRequestHeader("User-Agent", userAgent)
            setMimeType(mimeType)
            setTitle(name)
            setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
            // The public Downloads folder needs no permission from Android 10;
            // before that, the app's own folder (still opened by the notification).
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
            } else {
                setDestinationInExternalFilesDir(this@MainActivity, Environment.DIRECTORY_DOWNLOADS, name)
            }
        }
        try {
            (getSystemService(DOWNLOAD_SERVICE) as DownloadManager).enqueue(request)
            Toast.makeText(this, "Download: $name", Toast.LENGTH_SHORT).show()
        } catch (e: Exception) {
            Toast.makeText(this, "Download fehlgeschlagen: ${e.message}", Toast.LENGTH_LONG).show()
        }
    }

    /**
     * Tacked onto the WebView user-agent so the server can tell native
     * traffic apart from the browser. Mirrors the iOS app pattern.
     */
    private fun userAgentSuffix(): String =
        "RusterandoAndroid/${BuildConfig.VERSION_NAME}"

    /** What the current page needs from the shell. */
    private data class PageMode(
        /** `/kitchen/display`: one full-window touch canvas. */
        val isKitchenDisplay: Boolean = false,
        /**
         * Keep the screen on: the kitchen pages and the admin order board,
         * the screens left open on a kitchen / counter tablet. Not the
         * driver board — that's a phone on the road.
         */
        val keepAwake: Boolean = false,
        /** A live list board, reloaded on return (no half-typed drafts there). */
        val reloadOnResume: Boolean = false,
    )

    private companion object {
        /** Away longer than this, a live board reloads on return. */
        const val RESUME_RELOAD_AFTER_MS = 120_000L

        fun pageModeFor(url: String?): PageMode {
            val uri = url?.let(Uri::parse) ?: return PageMode()
            if (uri.scheme?.startsWith("http") != true) return PageMode()
            val path = (uri.path ?: "/").trimEnd('/').ifEmpty { "/" }
            return PageMode(
                isKitchenDisplay = path == "/kitchen/display",
                keepAwake = path == "/kitchen" || path.startsWith("/kitchen/") ||
                    path == "/admin/orders" || path.startsWith("/admin/orders/"),
                reloadOnResume = path in setOf("/kitchen", "/kitchen/display", "/admin/orders", "/driver"),
            )
        }
    }
}

/**
 * Keeps in-app navigation inside the WebView for the configured host;
 * everything else (mailto:, tel:, off-domain links) is handed to the
 * OS so the user lands in their dialer / mail client.
 */
private class ShopWebViewClient(
    private val shopHost: String,
    /** Every URL change, including the web app's own pushState routing. */
    private val onUrlChanged: (String?) -> Unit,
) : WebViewClient() {

    override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) {
        super.doUpdateVisitedHistory(view, url, isReload)
        onUrlChanged(url)
    }

    override fun shouldOverrideUrlLoading(
        view: WebView,
        request: WebResourceRequest,
    ): Boolean {
        val url = request.url
        return if (isInternal(url)) {
            false
        } else {
            val intent = Intent(Intent.ACTION_VIEW, url)
            view.context.startActivity(intent)
            true
        }
    }

    private fun isInternal(uri: Uri): Boolean {
        val host = uri.host ?: return false
        return host.equals(shopHost, ignoreCase = true) ||
            host.endsWith(".$shopHost", ignoreCase = true)
    }
}
