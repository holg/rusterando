package eu.trahe.rusterando

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.json.JSONArray
import org.json.JSONObject

/**
 * Persisted shop list. One encrypted SharedPreferences entry holds the
 * whole `shops_v1.json` blob (see `Shop.toJson`). EncryptedSharedPreferences
 * uses an AndroidKeyStore-held master key — payloads are AES-GCM at rest
 * and survive app updates.
 *
 * On first launch the flavor's `BuildConfig.START_URL` decides the
 * seed: non-empty pre-configures one shop with that URL + the
 * flavor's app_name; empty boots with an empty list so the UI
 * invites the user to add their own shop. Configured per flavor
 * in build.gradle.kts / build.local.gradle.kts.
 *
 * Mutations go through `commit { ... }` so the JSON write is single-shot
 * and the in-memory cache stays in sync.
 */
object ShopStore {

    private const val SECURE_PREFS = "rusterando.shops"
    /** Non-secret per-device settings (kitchen display), outside the shop blob. */
    private const val PLAIN_PREFS = "rusterando.prefs"
    private const val KEY_PAYLOAD = "shops_v1"
    private const val SCHEMA_VERSION = 1

    @Volatile
    private var cached: State? = null

    private data class State(
        val activeShopId: String?,
        val shops: List<Shop>,
    )

    private fun prefs(context: Context): SharedPreferences {
        val key = MasterKey.Builder(context.applicationContext)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        return EncryptedSharedPreferences.create(
            context.applicationContext,
            SECURE_PREFS,
            key,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    private fun load(context: Context): State {
        cached?.let { return it }
        synchronized(this) {
            cached?.let { return it }
            val raw = prefs(context).getString(KEY_PAYLOAD, null)
            // DATA-SAFETY CONTRACT (see iOS ShopStore.all() for the twin):
            //   - raw == null (true first launch) → seed from flavor + persist.
            //   - raw present but unparseable → return an EMPTY state WITHOUT
            //     writing. A schema change or corruption must NEVER re-seed on
            //     top of the user's real data — that's how an update wiped saved
            //     accounts. The encrypted blob is left byte-for-byte intact so a
            //     future migration can still recover it.
            //   - raw present + parseable → use it.
            val state: State
            if (raw == null) {
                state = seedFromFlavor(context)
                write(context, state)   // persist the seed only on first launch
            } else {
                state = runCatching { parse(raw) }.getOrElse {
                    android.util.Log.w(
                        "ShopStore",
                        "stored shops present but failed to parse — NOT overwriting " +
                            "(preserving ${raw.length} chars for recovery)",
                        it,
                    )
                    State(activeShopId = null, shops = emptyList())
                }
            }
            cached = state
            return state
        }
    }

    private fun parse(raw: String): State {
        val obj = JSONObject(raw)
        val shopArr = obj.optJSONArray("shops") ?: JSONArray()
        val shops = (0 until shopArr.length()).map { i ->
            Shop.fromJson(shopArr.getJSONObject(i))
        }
        val active = obj.optString("active_shop_id").takeIf { it.isNotEmpty() }
            ?: shops.firstOrNull()?.id
        return State(active, shops)
    }

    /**
     * Seed strategy on first launch. Driven entirely by the flavor's
     * BuildConfig.START_URL:
     *
     *   - Non-empty URL — pre-seed one shop with that URL + the
     *     flavor's app_name. Suits a single-tenant branded build
     *     (customer never opens Settings).
     *   - Empty URL — boot with an empty shop list so the UI invites
     *     the user to "+ Shop hinzufügen". Suits a brand-neutral
     *     multi-shop build for drivers / clients picking their own.
     *
     * Configure per flavor in build.gradle.kts (or build.local.gradle.kts)
     * via `buildConfigField("String", "START_URL", "\"…\"")` plus
     * `resValue("string", "app_name", "…")`.
     */
    private fun seedFromFlavor(context: Context): State {
        val url = BuildConfig.START_URL.trimEnd('/')
        if (url.isEmpty()) {
            return State(activeShopId = null, shops = emptyList())
        }
        val appName = context.getString(R.string.app_name)
        val seed = Shop.seed(name = appName, baseUrl = url)
        return State(activeShopId = seed.id, shops = listOf(seed))
    }

    private fun write(context: Context, state: State) {
        val obj = JSONObject().apply {
            put("version", SCHEMA_VERSION)
            put("active_shop_id", state.activeShopId)
            put("shops", JSONArray().apply {
                for (s in state.shops) put(s.toJson())
            })
        }
        prefs(context).edit().putString(KEY_PAYLOAD, obj.toString()).apply()
        cached = state
    }

    private inline fun commit(context: Context, block: (State) -> State) {
        synchronized(this) {
            val current = load(context)
            val next = block(current)
            write(context, next)
        }
    }

    // ---------------------------------------------------------------
    // Reads
    // ---------------------------------------------------------------

    fun all(context: Context): List<Shop> = load(context).shops

    fun activeShop(context: Context): Shop? {
        val s = load(context)
        return s.shops.firstOrNull { it.id == s.activeShopId } ?: s.shops.firstOrNull()
    }

    fun shop(context: Context, id: String): Shop? =
        load(context).shops.firstOrNull { it.id == id }

    fun hasAny(context: Context): Boolean = load(context).shops.isNotEmpty()

    // ---------------------------------------------------------------
    // Mutations
    // ---------------------------------------------------------------

    /** Add a shop and return its id. */
    fun addShop(context: Context, name: String, baseUrl: String): String {
        val shop = Shop.new(name, baseUrl)
        commit(context) { state ->
            state.copy(
                shops = state.shops + shop,
                // First-ever shop becomes active.
                activeShopId = state.activeShopId ?: shop.id,
            )
        }
        return shop.id
    }

    fun updateShop(context: Context, updated: Shop) {
        commit(context) { state ->
            state.copy(
                shops = state.shops.map { if (it.id == updated.id) updated else it },
            )
        }
    }

    fun removeShop(context: Context, id: String) {
        commit(context) { state ->
            val remaining = state.shops.filterNot { it.id == id }
            val nextActive = when {
                state.activeShopId != id -> state.activeShopId
                else -> remaining.firstOrNull()?.id
            }
            state.copy(shops = remaining, activeShopId = nextActive)
        }
        plain(context).edit().remove(kitchenDisplayKey(id)).apply()
    }

    fun setActiveShop(context: Context, id: String) {
        commit(context) { state ->
            if (state.shops.none { it.id == id }) state
            else state.copy(activeShopId = id)
        }
    }

    fun setActiveRole(context: Context, shopId: String, role: Role) {
        commit(context) { state ->
            state.copy(
                shops = state.shops.map {
                    if (it.id == shopId) it.withActiveRole(role) else it
                },
            )
        }
    }

    // ---------------------------------------------------------------
    // Kitchen display (plain prefs, per shop)
    // ---------------------------------------------------------------

    /**
     * Whether the Küche role starts on the kitchen display
     * (`/kitchen/display`, the board screen for a kitchen tablet) instead
     * of the order list (`/kitchen`). Kept outside the `shops_v1` blob on
     * purpose: that schema is shared with iOS and every installed version
     * must keep parsing it. Twin of iOS `ShopStore.kitchenOpensDisplay`.
     */
    fun kitchenOpensDisplay(context: Context, shopId: String): Boolean =
        plain(context).getBoolean(kitchenDisplayKey(shopId), false)

    fun setKitchenOpensDisplay(context: Context, shopId: String, on: Boolean) {
        plain(context).edit().putBoolean(kitchenDisplayKey(shopId), on).apply()
    }

    /** Where the shop opens for its active role. */
    fun homePath(context: Context, shop: Shop): String =
        if (shop.activeRole == Role.Kitchen && kitchenOpensDisplay(context, shop.id)) {
            "/kitchen/display"
        } else {
            shop.activeRole.homePath
        }

    private fun plain(context: Context): SharedPreferences =
        context.applicationContext.getSharedPreferences(PLAIN_PREFS, Context.MODE_PRIVATE)

    private fun kitchenDisplayKey(shopId: String) = "kitchen_display.$shopId"

    fun setCredential(context: Context, shopId: String, role: Role, password: String?) {
        commit(context) { state ->
            state.copy(
                shops = state.shops.map {
                    if (it.id == shopId) it.withCredential(role, password) else it
                },
            )
        }
    }

    /** Drop every shop, every credential, every active-role hint. */
    fun wipeAll(context: Context) {
        synchronized(this) {
            prefs(context).edit().remove(KEY_PAYLOAD).apply()
            plain(context).edit().clear().apply()
            cached = null
        }
    }
}
