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
import androidx.activity.result.ActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity

/**
 * Top-level settings: list of configured shops + "add" + destructive
 * "wipe everything". Tap a shop row to switch to it (returns RESULT_OK
 * + EXTRA_SHOP_ID so MainActivity reloads the WebView). The chevron-style
 * 'Bearbeiten' link opens [ShopDetailActivity] for credentials + per-shop
 * config.
 */
class ShopsActivity : AppCompatActivity() {

    companion object {
        const val EXTRA_SHOP_ID = "switched_shop_id"
    }

    private lateinit var root: LinearLayout
    private lateinit var detailLauncher: androidx.activity.result.ActivityResultLauncher<Intent>

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = "Shops"
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

        detailLauncher = registerForActivityResult(
            ActivityResultContracts.StartActivityForResult(),
            ::onDetailClosed,
        )
    }

    override fun onResume() {
        super.onResume()
        rebuild()
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    // ------------------------------------------------------------------
    // Render
    // ------------------------------------------------------------------

    private fun rebuild() {
        root.removeAllViews()

        root.addView(header("Konfigurierte Shops"))
        val shops = ShopStore.all(this)
        val active = ShopStore.activeShop(this)
        if (shops.isEmpty()) {
            root.addView(emptyHint("Noch kein Shop konfiguriert."))
        } else {
            for (s in shops) {
                root.addView(shopRow(s, isActive = s.id == active?.id))
            }
        }
        root.addView(footer(
            "Tippen, um zu wechseln. „Bearbeiten\" öffnet Passwörter & URL.",
        ))

        root.addView(addButton())

        root.addView(header(" "))
        root.addView(destructiveRow("Alle Shops und Passwörter löschen") {
            confirmWipeAll()
        })
    }

    private fun header(text: String): TextView = TextView(this).apply {
        this.text = text
        textSize = 13f
        setTextColor(0xFF888888.toInt())
        setPadding(dp(4), dp(20), dp(4), dp(6))
    }

    private fun footer(text: String): TextView = TextView(this).apply {
        this.text = text
        textSize = 12f
        setTextColor(0xFF888888.toInt())
        setPadding(dp(4), dp(6), dp(4), dp(12))
    }

    private fun emptyHint(text: String): TextView = TextView(this).apply {
        this.text = text
        textSize = 14f
        setTextColor(0xFF888888.toInt())
        setPadding(dp(12))
    }

    private fun shopRow(shop: Shop, isActive: Boolean): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(12))
            setBackgroundResource(android.R.drawable.list_selector_background)
        }

        // Tappable label area: switches to this shop.
        val labelArea = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
            isClickable = true
            isFocusable = true
            setBackgroundResource(android.R.drawable.list_selector_background)
            setOnClickListener { activate(shop) }
        }
        labelArea.addView(TextView(this).apply {
            text = shop.displayLabel
            textSize = 16f
        })
        labelArea.addView(TextView(this).apply {
            text = shop.baseUrl
            textSize = 12f
            setTextColor(0xFF888888.toInt())
        })

        val check = TextView(this).apply {
            text = if (isActive) "✓" else ""
            textSize = 18f
            setTextColor(0xFF2E7D32.toInt())
            setPadding(dp(8), 0, dp(12), 0)
        }
        val edit = TextView(this).apply {
            text = "Bearbeiten ›"
            textSize = 14f
            setTextColor(0xFF1565C0.toInt())
            isClickable = true
            isFocusable = true
            setBackgroundResource(android.R.drawable.list_selector_background)
            setPadding(dp(8), dp(8), dp(8), dp(8))
            setOnClickListener { openDetail(shop) }
        }

        row.addView(labelArea)
        row.addView(check)
        row.addView(edit)
        return row
    }

    private fun addButton(): View = TextView(this).apply {
        text = "+ Shop hinzufügen"
        textSize = 16f
        setTextColor(0xFF1565C0.toInt())
        setPadding(dp(12))
        isClickable = true
        isFocusable = true
        setBackgroundResource(android.R.drawable.list_selector_background)
        setOnClickListener { promptAddShop() }
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

    private fun activate(shop: Shop) {
        ShopStore.setActiveShop(this, shop.id)
        setResult(
            Activity.RESULT_OK,
            Intent().putExtra(EXTRA_SHOP_ID, shop.id),
        )
        finish()
    }

    private fun openDetail(shop: Shop) {
        val intent = Intent(this, ShopDetailActivity::class.java)
            .putExtra(ShopDetailActivity.EXTRA_SHOP_ID, shop.id)
        detailLauncher.launch(intent)
    }

    private fun onDetailClosed(result: ActivityResult) {
        // If the detail screen activated this shop or switched roles,
        // it returns RESULT_OK + shopId so we finish back to MainActivity
        // with the same payload (which will reload the WebView).
        if (result.resultCode == Activity.RESULT_OK) {
            val shopId = result.data?.getStringExtra(ShopDetailActivity.EXTRA_SHOP_ID)
            if (shopId != null) {
                setResult(
                    Activity.RESULT_OK,
                    Intent().putExtra(EXTRA_SHOP_ID, shopId),
                )
                finish()
                return
            }
        }
        rebuild()
    }

    private fun promptAddShop() {
        val nameInput = EditText(this).apply {
            hint = "Name (z.B. Mein Restaurant)"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_WORDS
        }
        val urlInput = EditText(this).apply {
            hint = "https://meinshop.de"
            inputType = InputType.TYPE_TEXT_VARIATION_URI
        }
        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(8), dp(16), 0)
            addView(nameInput)
            addView(urlInput)
        }
        AlertDialog.Builder(this)
            .setTitle("Shop hinzufügen")
            .setMessage("URL inklusive https:// — Passwörter danach im Shop-Detail.")
            .setView(container)
            .setPositiveButton("Speichern") { _, _ ->
                val name = nameInput.text.toString().trim()
                val url = urlInput.text.toString().trim()
                if (url.isEmpty() || !url.startsWith("http")) {
                    showError("URL muss mit http:// oder https:// beginnen.")
                    return@setPositiveButton
                }
                ShopStore.addShop(this, name.ifEmpty { url }, url)
                rebuild()
            }
            .setNegativeButton("Abbrechen", null)
            .show()
    }

    private fun confirmWipeAll() {
        AlertDialog.Builder(this)
            .setTitle("Alles löschen?")
            .setMessage(
                "Alle Shops, gespeicherten Passwörter und Session-Cookies werden entfernt.",
            )
            .setPositiveButton("Löschen") { _, _ ->
                ShopStore.wipeAll(this)
                // Clear WebView cookies too so the next launch is anonymous.
                android.webkit.CookieManager.getInstance().removeAllCookies(null)
                android.webkit.CookieManager.getInstance().flush()
                rebuild()
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
