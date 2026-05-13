package eu.trahe.rusterando

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * One shop's detail screen. Reached from [ShopsActivity]. Sections:
 *
 *  - Name + base URL (editable inline).
 *  - "Diese Shop aktivieren" — switches MainActivity to this shop's URL.
 *  - Aktive Rolle picker — switches role within the shop.
 *  - Gespeicherte Passwörter — Admin / Küche / Fahrer rows.
 *  - Shop entfernen.
 *
 * On role activation we POST to the shop's login endpoint via
 * [RoleSwitcher] and return RESULT_OK + EXTRA_SHOP_ID so the caller
 * chain reloads the WebView. Editing name/URL/passwords stays on the
 * detail screen (returns RESULT_CANCELED).
 */
class ShopDetailActivity : AppCompatActivity() {

    companion object {
        const val EXTRA_SHOP_ID = "shop_id"
    }

    private lateinit var root: LinearLayout
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var pendingJob: Job? = null
    private var shopId: String = ""

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        shopId = intent.getStringExtra(EXTRA_SHOP_ID).orEmpty()
        if (shopId.isEmpty() || ShopStore.shop(this, shopId) == null) {
            finish()
            return
        }

        supportActionBar?.setDisplayHomeAsUpEnabled(true)

        val scroll = ScrollView(this).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16))
        }
        scroll.addView(root)
        setContentView(scroll)

        rebuild()
    }

    override fun onDestroy() {
        pendingJob?.cancel()
        super.onDestroy()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    // ------------------------------------------------------------------
    // Render
    // ------------------------------------------------------------------

    private fun shop(): Shop = ShopStore.shop(this, shopId)!!

    private fun rebuild() {
        val s = shop()
        title = s.displayLabel
        root.removeAllViews()

        // Section: name + URL (tap each to edit).
        root.addView(header("Shop"))
        root.addView(editableRow("Name", s.name) { newVal ->
            ShopStore.updateShop(this, s.copy(name = newVal.trim()))
            rebuild()
        })
        root.addView(editableRow("URL", s.baseUrl) { newVal ->
            val trimmed = newVal.trim().trimEnd('/')
            if (trimmed.startsWith("http")) {
                ShopStore.updateShop(this, s.copy(baseUrl = trimmed))
                rebuild()
            } else {
                showError("URL muss mit http:// oder https:// beginnen.")
            }
        })

        // Section: activate this shop.
        root.addView(header(" "))
        root.addView(simpleRow("Diese Shop aktivieren") {
            ShopStore.setActiveShop(this, s.id)
            setResult(Activity.RESULT_OK, Intent().putExtra(EXTRA_SHOP_ID, s.id))
            finish()
        })

        // Section: active role picker.
        root.addView(header("Aktive Rolle"))
        val activatable = listOf(Role.Customer) + s.savedStaffRoles()
        for (r in activatable) {
            root.addView(activeRoleRow(r, isActive = r == s.activeRole))
        }
        root.addView(footer(
            "„Kunde\" entspricht dem öffentlichen Shop ohne Login.",
        ))

        // Section: stored passwords.
        root.addView(header("Gespeicherte Passwörter"))
        for (r in Role.staffRoles) {
            val saved = s.credentials[r] != null
            root.addView(passwordRow(r, saved))
        }
        root.addView(footer(
            "Passwörter werden lokal verschlüsselt gespeichert (AndroidKeyStore).",
        ))

        // Section: delete this shop.
        root.addView(header(" "))
        root.addView(destructiveRow("Shop entfernen") { confirmDelete() })
    }

    private fun header(text: String) = TextView(this).apply {
        this.text = text
        textSize = 13f
        setTextColor(0xFF888888.toInt())
        setPadding(dp(4), dp(20), dp(4), dp(6))
    }

    private fun footer(text: String) = TextView(this).apply {
        this.text = text
        textSize = 12f
        setTextColor(0xFF888888.toInt())
        setPadding(dp(4), dp(6), dp(4), dp(12))
    }

    private fun editableRow(
        label: String,
        currentValue: String,
        onSave: (String) -> Unit,
    ): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(12))
            isClickable = true
            isFocusable = true
            setBackgroundResource(android.R.drawable.list_selector_background)
        }
        row.addView(TextView(this).apply {
            text = label
            textSize = 16f
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        })
        row.addView(TextView(this).apply {
            text = currentValue
            textSize = 14f
            setTextColor(0xFF555555.toInt())
            maxLines = 1
            ellipsize = android.text.TextUtils.TruncateAt.MIDDLE
        })
        row.setOnClickListener {
            val input = EditText(this).apply {
                inputType = if (label == "URL")
                    InputType.TYPE_TEXT_VARIATION_URI
                else
                    InputType.TYPE_CLASS_TEXT
                setText(currentValue)
                setSelection(text.length)
            }
            val container = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(16), dp(8), dp(16), 0)
                addView(input)
            }
            AlertDialog.Builder(this)
                .setTitle(label)
                .setView(container)
                .setPositiveButton("Speichern") { _, _ -> onSave(input.text.toString()) }
                .setNegativeButton("Abbrechen", null)
                .show()
        }
        return row
    }

    private fun activeRoleRow(role: Role, isActive: Boolean): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(12))
            isClickable = true
            isFocusable = true
            setBackgroundResource(android.R.drawable.list_selector_background)
        }
        row.addView(TextView(this).apply {
            text = role.label
            textSize = 16f
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        })
        row.addView(TextView(this).apply {
            text = if (isActive) "✓" else ""
            textSize = 18f
            setTextColor(0xFF2E7D32.toInt())
            setPadding(dp(8), 0, 0, 0)
        })
        row.setOnClickListener { activate(role) }
        return row
    }

    private fun passwordRow(role: Role, saved: Boolean): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(12))
            isClickable = true
            isFocusable = true
            setBackgroundResource(android.R.drawable.list_selector_background)
        }
        row.addView(TextView(this).apply {
            text = role.label
            textSize = 16f
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        })
        row.addView(TextView(this).apply {
            text = if (saved) "✓ gespeichert" else "+ hinzufügen"
            textSize = 14f
            setTextColor(if (saved) 0xFF2E7D32.toInt() else 0xFF1565C0.toInt())
        })
        row.setOnClickListener { editPassword(role) }
        return row
    }

    private fun simpleRow(text: String, onClick: () -> Unit): View = TextView(this).apply {
        this.text = text
        textSize = 16f
        setTextColor(0xFF1565C0.toInt())
        setPadding(dp(12))
        isClickable = true
        isFocusable = true
        setBackgroundResource(android.R.drawable.list_selector_background)
        setOnClickListener { onClick() }
    }

    private fun destructiveRow(text: String, onClick: () -> Unit): View = TextView(this).apply {
        this.text = text
        textSize = 16f
        setTextColor(0xFFC62828.toInt())
        setPadding(dp(12))
        isClickable = true
        isFocusable = true
        setBackgroundResource(android.R.drawable.list_selector_background)
        setOnClickListener { onClick() }
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    private fun activate(role: Role) {
        pendingJob?.cancel()
        pendingJob = scope.launch {
            val s = shop()
            val result = withContext(Dispatchers.IO) {
                RoleSwitcher.switchTo(this@ShopDetailActivity, s, role)
            }
            when (result) {
                RoleSwitchResult.Success -> {
                    // Make this shop the active one too, so MainActivity
                    // picks the right baseUrl on return.
                    ShopStore.setActiveShop(this@ShopDetailActivity, s.id)
                    setResult(
                        Activity.RESULT_OK,
                        Intent().putExtra(EXTRA_SHOP_ID, s.id),
                    )
                    finish()
                }
                RoleSwitchResult.NoPassword -> {
                    showError("Kein Passwort gespeichert.")
                    editPassword(role)
                }
                RoleSwitchResult.WrongPassword -> {
                    showError("Gespeichertes Passwort wurde abgelehnt. Bitte erneut eingeben.")
                    editPassword(role)
                }
                is RoleSwitchResult.Server -> showError("Server-Fehler ${result.code}.")
                is RoleSwitchResult.Network -> showError("Netzwerk-Fehler: ${result.message}")
            }
        }
    }

    private fun editPassword(role: Role) {
        val s = shop()
        val existing = s.credentials[role]
        val input = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            hint = "Passwort"
            setText(existing.orEmpty())
        }
        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(8), dp(16), 0)
            addView(input)
        }
        AlertDialog.Builder(this)
            .setTitle(role.label)
            .setMessage(
                if (existing == null)
                    "Passwort eingeben — wird sicher auf dem Gerät gespeichert."
                else
                    "Passwort ändern. Leer lassen + Speichern, um es zu löschen.",
            )
            .setView(container)
            .setPositiveButton("Speichern") { _, _ ->
                val pw = input.text.toString()
                ShopStore.setCredential(this, s.id, role, pw.ifEmpty { null })
                rebuild()
            }
            .setNegativeButton("Abbrechen", null)
            .show()
    }

    private fun confirmDelete() {
        val s = shop()
        AlertDialog.Builder(this)
            .setTitle("Shop entfernen?")
            .setMessage("„${s.displayLabel}\" wird mit allen Passwörtern gelöscht.")
            .setPositiveButton("Entfernen") { _, _ ->
                ShopStore.removeShop(this, s.id)
                finish()
            }
            .setNegativeButton("Abbrechen", null)
            .show()
    }

    private fun showError(msg: String) {
        AlertDialog.Builder(this)
            .setTitle("Hinweis")
            .setMessage(msg)
            .setPositiveButton("OK", null)
            .show()
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()
}

private fun View.setPadding(all: Int) = setPadding(all, all, all, all)
