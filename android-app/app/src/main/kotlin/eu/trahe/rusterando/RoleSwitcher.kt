package eu.trahe.rusterando

import android.content.Context
import android.webkit.CookieManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.OutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

/**
 * Result of a role switch. Mirrors the iOS `RoleSwitchError` cases so
 * the UI layer can branch the same way.
 */
sealed class RoleSwitchResult {
    object Success : RoleSwitchResult()
    object NoPassword : RoleSwitchResult()
    object WrongPassword : RoleSwitchResult()
    data class Server(val code: Int) : RoleSwitchResult()
    data class Network(val message: String) : RoleSwitchResult()
}

/**
 * Switches the WebView's `dp_session` cookie for a given shop without
 * making the user retype the password:
 *
 *   1. Read the saved password from the [Shop]'s credentials map.
 *   2. POST `password=<value>` to `<shop.baseUrl>/api/<role>_login`.
 *   3. Server responds with `Set-Cookie: dp_session=...`; mirror every
 *      cookie header into [CookieManager] so the WebView sees the
 *      fresh session on its next request.
 *   4. Bump the shop's active-role hint in [ShopStore].
 *
 * Multi-shop note: every call is bound to a single `Shop` — its
 * baseUrl chooses where the POST goes, its credentials map provides
 * the password. Cookies persist per-host in CookieManager already, so
 * switching shops just means loading a different baseUrl.
 */
object RoleSwitcher {

    /**
     * Switch [shop] to [role]. For staff roles this signs in via the
     * stored password and writes the new `dp_session` cookie. For
     * Customer this is a logout against [shop]'s server, then a
     * defensive local cookie purge so the next page load is anonymous.
     */
    suspend fun switchTo(
        context: Context,
        shop: Shop,
        role: Role,
    ): RoleSwitchResult = withContext(Dispatchers.IO) {
        if (role == Role.Customer) {
            logoutToCustomer(context, shop)
            return@withContext RoleSwitchResult.Success
        }

        val password = shop.credentials[role]
            ?: return@withContext RoleSwitchResult.NoPassword
        val loginPath = role.loginPath
            ?: return@withContext RoleSwitchResult.NoPassword

        val url = URL(shop.baseUrl.trimEnd('/') + loginPath)
        val body = "password=" + URLEncoder.encode(password, "UTF-8")

        val conn = (url.openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = 15_000
            readTimeout = 15_000
            setRequestProperty(
                "Content-Type",
                "application/x-www-form-urlencoded",
            )
            CookieManager.getInstance().getCookie(url.toString())?.let {
                setRequestProperty("Cookie", it)
            }
        }

        try {
            conn.outputStream.use { os: OutputStream ->
                os.write(body.toByteArray(Charsets.UTF_8))
            }
            val code = conn.responseCode
            if (code !in 200..299) {
                return@withContext RoleSwitchResult.Server(code)
            }

            val responseText = conn.inputStream.bufferedReader().use { it.readText() }
                .trim()
                .lowercase()
            // Leptos server-fn convention: 200 OK with body `true`
            // for success, `false` for wrong password.
            if (responseText == "false") {
                return@withContext RoleSwitchResult.WrongPassword
            }

            mirrorSetCookieIntoWebView(url, conn)

            ShopStore.setActiveRole(context, shop.id, role)
            RoleSwitchResult.Success
        } catch (e: Exception) {
            RoleSwitchResult.Network(e.message ?: e::class.java.simpleName)
        } finally {
            conn.disconnect()
        }
    }

    /**
     * Copy every `Set-Cookie` response header from the HTTP login into
     * the WebView's cookie jar. Android's CookieManager shares the
     * same store the WebView reads from, so this makes the new
     * `dp_session` visible on the next page load.
     */
    private fun mirrorSetCookieIntoWebView(url: URL, conn: HttpURLConnection) {
        val cm = CookieManager.getInstance()
        val base = url.protocol + "://" + url.host + (if (url.port > 0) ":${url.port}" else "")
        conn.headerFields.forEach { (key, values) ->
            if (key != null && key.equals("Set-Cookie", ignoreCase = true)) {
                for (v in values) {
                    cm.setCookie(base, v)
                }
            }
        }
        cm.flush()
    }

    /**
     * Customer mode = logout. POSTs to `<shop.baseUrl>/api/session_logout`
     * (best-effort — even if the call fails, we defensively drop session
     * cookies locally so the next page load is anonymous).
     */
    private fun logoutToCustomer(context: Context, shop: Shop) {
        val cm = CookieManager.getInstance()
        val base = shop.baseUrl.trimEnd('/')
        val logoutUrl = "$base/api/session_logout"

        runCatching {
            val conn = (URL(logoutUrl).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                doOutput = true
                connectTimeout = 10_000
                readTimeout = 10_000
                setRequestProperty(
                    "Content-Type",
                    "application/x-www-form-urlencoded",
                )
                cm.getCookie(logoutUrl)?.let { setRequestProperty("Cookie", it) }
            }
            try {
                conn.outputStream.use { it.write(ByteArray(0)) }
                conn.responseCode  // touch to flush
            } finally {
                conn.disconnect()
            }
        }

        // Defensive purge: drop dp_session / admin_session cookies for
        // this shop's host regardless of whether the server call worked.
        for (name in listOf("dp_session", "admin_session")) {
            cm.setCookie(base, "$name=; Max-Age=0; Path=/")
        }
        cm.flush()

        ShopStore.setActiveRole(context, shop.id, Role.Customer)
    }
}
