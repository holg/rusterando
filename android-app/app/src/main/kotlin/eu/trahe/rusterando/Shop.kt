package eu.trahe.rusterando

import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/**
 * One configured shop on this device. Mirrors the cross-platform
 * `shops_v1.json` schema:
 *
 * ```jsonc
 * {
 *   "id": "<uuid>",
 *   "name": "My Shop",
 *   "base_url": "https://myshop.example.com",
 *   "active_role": "admin",
 *   "credentials": { "admin": "...", "kitchen": "...", "driver": "..." }
 * }
 * ```
 *
 * Credentials live inside the Shop record so they're encrypted along
 * with the rest of the shop list (one blob in EncryptedSharedPreferences)
 * — cheaper than one prefs key per `(shop, role)` pair.
 */
data class Shop(
    val id: String,
    val name: String,
    /** Without trailing slash. e.g. `https://davidspizzeria.de`. */
    val baseUrl: String,
    val activeRole: Role,
    val credentials: Map<Role, String>,
) {
    val displayLabel: String get() = name.ifBlank { baseUrl }

    fun withCredential(role: Role, password: String?): Shop {
        val next = credentials.toMutableMap()
        if (password.isNullOrEmpty()) next.remove(role) else next[role] = password
        return copy(credentials = next)
    }

    fun withActiveRole(role: Role): Shop = copy(activeRole = role)

    /** Staff roles that currently have a stored password. */
    fun savedStaffRoles(): List<Role> =
        Role.staffRoles.filter { credentials[it] != null }

    fun toJson(): JSONObject = JSONObject().apply {
        put("id", id)
        put("name", name)
        put("base_url", baseUrl)
        put("active_role", activeRole.rawValue)
        val creds = JSONObject()
        for ((role, pw) in credentials) creds.put(role.rawValue, pw)
        put("credentials", creds)
    }

    companion object {
        fun new(name: String, baseUrl: String): Shop = Shop(
            id = UUID.randomUUID().toString(),
            name = name,
            baseUrl = baseUrl.trimEnd('/'),
            activeRole = Role.Customer,
            credentials = emptyMap(),
        )

        fun fromJson(obj: JSONObject): Shop {
            val creds = mutableMapOf<Role, String>()
            obj.optJSONObject("credentials")?.let { c ->
                for (key in c.keys()) {
                    val role = Role.fromRaw(key) ?: continue
                    creds[role] = c.optString(key, "")
                }
            }
            return Shop(
                id = obj.getString("id"),
                name = obj.getString("name"),
                baseUrl = obj.getString("base_url").trimEnd('/'),
                activeRole = Role.fromRaw(obj.optString("active_role"))
                    ?: Role.Customer,
                credentials = creds,
            )
        }
    }
}
