package eu.trahe.rusterando

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.FrameLayout
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
    private lateinit var settingsLauncher: androidx.activity.result.ActivityResultLauncher<Intent>

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

            webViewClient = ShopWebViewClient(BuildConfig.ASSOCIATED_DOMAIN)
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
            addView(buildGearButton())
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
        val role = shop?.activeRole ?: Role.Customer
        val url = if (role != Role.Customer) base + role.homePath else "$base/"
        webView.loadUrl(url)
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
    }

    override fun onDestroy() {
        // Detach + destroy so the WebView's render process doesn't leak
        // beyond the Activity's lifetime.
        (webView.parent as? ViewGroup)?.removeView(webView)
        webView.destroy()
        super.onDestroy()
    }

    /**
     * Tacked onto the WebView user-agent so the server can tell native
     * traffic apart from the browser. Mirrors the iOS app pattern.
     */
    private fun userAgentSuffix(): String =
        "RusterandoAndroid/${BuildConfig.VERSION_NAME}"
}

/**
 * Keeps in-app navigation inside the WebView for the configured host;
 * everything else (mailto:, tel:, off-domain links) is handed to the
 * OS so the user lands in their dialer / mail client.
 */
private class ShopWebViewClient(private val shopHost: String) : WebViewClient() {

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
