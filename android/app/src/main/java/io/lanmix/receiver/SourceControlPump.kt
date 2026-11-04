package io.lanmix.receiver

import android.os.Handler
import org.json.JSONArray

data class ReceiverSource(val slot: Int, val address: String, val streamId: Long,
    val sampleRate: Int, val online: Boolean) {
    val identity get() = "$address/$streamId"
}

/** Main-thread control work, independent of diagnostic formatting and Activity lifetime. */
internal class SourceControlPump(
    private val handler: Handler,
    private val read: () -> String,
    private val bind: (Int, String, Long) -> Unit,
    private val publish: (List<ReceiverSource>) -> Unit,
    private val failure: (Exception) -> Unit
) {
    private var active = false
    private var published: List<ReceiverSource>? = null
    private var lastError: String? = null
    private val poll = object : Runnable {
        override fun run() {
            if (!active) return
            try {
                val json = JSONArray(read())
                val next = mutableListOf<ReceiverSource>()
                repeat(json.length()) { index ->
                    val item = json.getJSONObject(index)
                    val slot = item.getInt("slot")
                    if (slot !in 0..15 || next.any { it.slot == slot }) return@repeat
                    val source = ReceiverSource(slot, item.getString("source"), item.getLong("stream_id"),
                        item.getInt("sample_rate"), item.getBoolean("online"))
                    // Restore every source's complete saved settings before exposing its editor.
                    bind(slot, source.address, source.streamId)
                    next.add(source)
                }
                if (next != published) {
                    published = next
                    publish(next)
                }
                lastError = null
            } catch (error: Exception) {
                // Binder retains failed applications for the next tick. Do not
                // flood history at 20 Hz if a native session is being replaced.
                if (error.message != lastError) { lastError = error.message; failure(error) }
            }
            if (active) handler.postDelayed(this, 50)
        }
    }

    fun start() {
        stop()
        active = true
        published = null
        lastError = null
        handler.post(poll)
    }

    fun stop() {
        active = false
        handler.removeCallbacks(poll)
    }
}
