package eu.trahe.rusterando

import android.content.Context
import android.content.pm.PackageManager
import android.util.Log
import com.sunmi.peripheral.printer.InnerPrinterCallback
import com.sunmi.peripheral.printer.InnerPrinterManager
import com.sunmi.peripheral.printer.InnerResultCallback
import com.sunmi.peripheral.printer.SunmiPrinterService
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The device's built-in receipt printer — today the Sunmi handhelds (V2:
 * 58 mm, tear bar). Bound once per process through Sunmi's system service;
 * on every other device [isPresent] is false and nothing here runs.
 *
 * The Bon comes ready-made from the server (`/api/orders/{id}/bon-58mm`,
 * receipt-escpos `Paper::Mm58`): this only fetches it with the WebView's
 * session, hands the bytes to the printer and reports what happened.
 */
object ReceiptPrinter {
    private const val TAG = "ReceiptPrinter"
    private const val SERVICE_PACKAGE = "woyou.aidlservice.jiuiv5"

    /** A Bon is a few KB; anything near the 1 MB binder limit is wrong. */
    private const val MAX_BON_BYTES = 512 * 1024

    @Volatile
    private var service: SunmiPrinterService? = null
    private var binding = false

    private val connection = object : InnerPrinterCallback() {
        override fun onConnected(s: SunmiPrinterService) {
            service = s
            Log.i(TAG, "connected, printer ${stateText(state())}")
        }

        override fun onDisconnected() {
            service = null
            binding = false
            Log.i(TAG, "disconnected")
        }
    }

    /** Whether this device has the Sunmi printer service at all. */
    fun isPresent(context: Context): Boolean = try {
        context.packageManager.getPackageInfo(SERVICE_PACKAGE, 0)
        true
    } catch (e: PackageManager.NameNotFoundException) {
        false
    }

    /** Bind the printer service once. Main thread. */
    fun bind(context: Context) {
        if (binding || service != null || !isPresent(context)) return
        binding = try {
            InnerPrinterManager.getInstance().bindService(context.applicationContext, connection)
        } catch (e: Exception) {
            Log.w(TAG, "bind failed", e)
            false
        }
    }

    /** Sunmi's printer state code, or null while not connected. */
    private fun state(): Int? = try {
        service?.updatePrinterState()
    } catch (e: Exception) {
        null
    }

    private fun stateText(state: Int?): String = when (state) {
        null -> "nicht verbunden"
        1 -> "bereit"
        2 -> "startet noch"
        3 -> "Verbindungsfehler"
        4 -> "kein Papier"
        5 -> "überhitzt"
        6 -> "Klappe offen"
        505 -> "nicht gefunden"
        else -> "Fehler $state"
    }

    /**
     * Fetch the 58-mm Bon of [orderId] from [origin] (a shop origin the page
     * came from) with [cookie], and print it. Returns a short German
     * message for a toast.
     */
    suspend fun printOrder(origin: String, orderId: String, cookie: String?, userAgent: String): String =
        withContext(Dispatchers.IO) {
            if (service == null) return@withContext "Drucker nicht verbunden"
            val bytes = try {
                val conn = (URL("$origin/api/orders/$orderId/bon-58mm").openConnection() as HttpURLConnection)
                    .apply {
                        connectTimeout = 10_000
                        readTimeout = 15_000
                        cookie?.let { setRequestProperty("Cookie", it) }
                        setRequestProperty("User-Agent", userAgent)
                    }
                try {
                    when (val code = conn.responseCode) {
                        200 -> conn.inputStream.use { it.readBytes() }
                        401, 403 -> return@withContext "Nicht angemeldet (Admin oder Küche)"
                        404 -> return@withContext "Bestellung nicht gefunden"
                        else -> return@withContext "Server-Fehler $code"
                    }
                } finally {
                    conn.disconnect()
                }
            } catch (e: Exception) {
                Log.w(TAG, "fetch failed", e)
                return@withContext "Keine Verbindung zum Server"
            }
            if (bytes.isEmpty() || bytes.size > MAX_BON_BYTES) {
                return@withContext "Ungültiger Bon (${bytes.size} Bytes)"
            }
            printRaw(bytes)
        }

    /**
     * Print raw ESC/POS and wait for the printer's verdict. Transaction
     * mode, so the answer says whether the Bon actually came out (paper
     * running out halfway included), not just whether the bytes arrived.
     */
    private fun printRaw(bytes: ByteArray): String {
        val s = service ?: return "Drucker nicht verbunden"
        val st = state()
        if (st != 1) return "Drucker: ${stateText(st)}"

        val result = java.util.concurrent.ArrayBlockingQueue<String>(1)
        val once = AtomicBoolean(false)
        fun finish(msg: String) {
            if (once.compareAndSet(false, true)) result.offer(msg)
        }
        try {
            s.enterPrinterBuffer(true)
            s.sendRAWData(bytes, null)
            s.exitPrinterBufferWithCallback(true, object : InnerResultCallback() {
                override fun onRunResult(isSuccess: Boolean) {}
                override fun onReturnString(result: String?) {}
                override fun onRaiseException(code: Int, msg: String?) {
                    finish("Druckfehler: ${msg ?: code}")
                }
                override fun onPrintResult(code: Int, msg: String?) {
                    finish(if (code == 0) "Bon gedruckt" else "Druckfehler: ${msg ?: code}")
                }
            })
        } catch (e: Exception) {
            Log.w(TAG, "print failed", e)
            finish("Druckfehler: ${e.message}")
        }
        return result.poll(30, java.util.concurrent.TimeUnit.SECONDS)
            ?: "Drucker antwortet nicht"
    }
}
