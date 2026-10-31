package io.lanmix.receiver

import android.app.Application
import android.os.SystemClock

/** Process-owned UI state. Audio and sockets belong exclusively to ReceiverService. */
class LanMixApplication : Application() {
    val receiver by lazy { ReceiverState(MixState(getSharedPreferences("lanmix", MODE_PRIVATE))) }
}

class ReceiverState(val mix: MixState) {
    val startedAt = SystemClock.elapsedRealtime()
    val history = DiagnosticHistory()
    var requested = false
    var running = false
    var status = "准备就绪 · 点击开始接收"
    var actual = "等待启动音频输出"
    var diagnostics = "{}"
    var sampledAt = 0L
    var marked = "尚未标记闪断"
    var epoch = 0L
    var config: ReceiverSettings? = null
    private val observers = mutableSetOf<() -> Unit>()

    fun observe(observer: () -> Unit) { observers.add(observer); observer() }
    fun removeObserver(observer: () -> Unit) { observers.remove(observer) }
    fun changed() { observers.toList().forEach { it() } }
    fun event(message: String) {
        history.record(SystemClock.elapsedRealtime() - startedAt, message)
    }
}

data class ReceiverSettings(val port: Int, val bufferMs: Int, val deviceId: Int?,
    val rate: Int, val mode: PlaybackMode)
