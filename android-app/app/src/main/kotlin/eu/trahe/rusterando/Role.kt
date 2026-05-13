package eu.trahe.rusterando

/**
 * Counterpart of the iOS `Role` enum. Customer is the absence of a
 * session cookie (no password); the three staff roles each require a
 * password persisted in [RoleStore].
 */
enum class Role(
    val rawValue: String,
    val label: String,
    val homePath: String,
    /** API endpoint to POST `password=<value>` to. null for customer. */
    val loginPath: String?,
) {
    Customer("customer", "Kunde", "/", null),
    Admin("admin", "Admin", "/admin", "/api/admin_login"),
    Kitchen("kitchen", "Küche", "/kitchen", "/api/kitchen_login"),
    Driver("driver", "Fahrer", "/driver", "/api/driver_login"),
    ;

    val requiresPassword: Boolean get() = loginPath != null

    companion object {
        val staffRoles: List<Role> get() = values().filter { it.requiresPassword }

        fun fromRaw(raw: String?): Role? =
            raw?.let { v -> values().firstOrNull { it.rawValue == v } }
    }
}
