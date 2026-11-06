package io.lanmix.receiver

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import org.json.JSONObject

/** Owns playback independently of screens; only an explicit user start launches it. */
class ReceiverService : Service() {
    companion object {
        const val START = "io.lanmix.receiver.START"
        const val STOP = "io.lanmix.receiver.STOP"
        private const val CHANNEL = "lanmix_receiver"
        private const val NOTIFICATION = 46210

        fun startIntent(context: android.content.Context, settings: ReceiverSettings, reason: String) =
            Intent(context, ReceiverService::class.java).setAction(START)
                .putExtra("port", settings.port).putExtra("buffer", settings.bufferMs)
                .putExtra("device", settings.deviceId ?: -1).putExtra("rate", settings.rate)
                .putExtra("mode", settings.mode.key).putExtra("reason", reason)
    }

    private val state get() = (application as LanMixApplication).receiver
    private val discovery get() = (application as LanMixApplication).discovery
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var manager: AudioManager
    private lateinit var engine: PlaybackEngine
    private lateinit var bindings: SourceControlBinder
    private lateinit var sources: SourceControlPump
    private var wakeLock: PowerManager.WakeLock? = null
    private var renewWakeAt = 0L
    private var lastStartId = 0
    private var foreground = false
    private var notificationKey: Pair<String, RemoteMediaTarget?>? = null
    private val notificationObserver: () -> Unit = { updateNotification() }
    private var destroyed = false
    private data class SourceDiagnostic(val session: Long, val lost: Long, val late: Long,
        val overflow: Long, val underruns: Long, val resets: Long, val primed: Boolean, val gapFilled: Long)
    private val sourceDiagnostics = mutableMapOf<String, SourceDiagnostic>()
    private val devices = object : AudioDeviceCallback() {
        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) {
            val current = state.config ?: return
            if (state.requested && removed.any { it.id == current.deviceId }) {
                begin(current.copy(deviceId = null), "原首选输出已移除，回到系统默认")
            }
        }
    }
    private val poll = object : Runnable {
        override fun run() {
            if (destroyed || !state.requested) return
            renewWakeLock()
            if (state.running) {
                runCatching {
                    val raw = NativeBridge.nativeTracks()
                    val json = JSONObject(raw)
                    val tracks = json.optJSONArray("tracks")
                    val seen = mutableSetOf<String>()
                    if (tracks != null) repeat(tracks.length()) { index ->
                        val source = tracks.getJSONObject(index)
                        val slot = source.optInt("slot", -1)
                        val identity = "${source.optString("source")}/${source.optLong("stream_id")}"
                        seen.add(identity)
                        val meter = json.optJSONArray("meters")?.optJSONObject(slot)
                        val diagnostic = SourceDiagnostic(source.optLong("session_id"), source.optLong("concealed_frames"),
                            source.optLong("late_packets"), source.optLong("queue_overflows"), meter?.optLong("underruns") ?: 0,
                            source.optLong("reset_count"), meter?.optBoolean("primed") == true, source.optLong("gap_filled_packets"))
                        if (sourceDiagnostics.put(identity, diagnostic) != diagnostic) {
                            state.event("音源 $identity：会话 ${diagnostic.session}，丢帧 ${diagnostic.lost}，迟到 ${diagnostic.late}，" +
                                "溢出 ${diagnostic.overflow}，缺粮 ${diagnostic.underruns}，重置 ${diagnostic.resets} " +
                                "(${source.optString("reset_reason")})，补回 ${diagnostic.gapFilled}，" +
                                "最近缺口 ${source.optString("last_loss_kind")} / ${source.optLong("last_loss_frames")} 帧，" +
                                "${if (diagnostic.primed) "播放中" else "缓冲中"}")
                        }
                    }
                    sourceDiagnostics.keys.retainAll(seen)
                    state.diagnostics = raw
                    state.sampledAt = SystemClock.elapsedRealtime() - state.startedAt
                }.onFailure { state.event("后台来源诊断/控制失败：${it.message}") }
                state.changed()
            }
            handler.postDelayed(this, 750)
        }
    }

    override fun onCreate() {
        super.onCreate()
        manager = getSystemService(AudioManager::class.java)
        val notifications = getSystemService(NotificationManager::class.java)
        notifications.createNotificationChannel(NotificationChannel(CHANNEL,
            getString(R.string.receiver_channel), NotificationManager.IMPORTANCE_LOW).apply {
            description = getString(R.string.receiver_channel_description)
            setSound(null, null)
            enableVibration(false)
        })
        bindings = SourceControlBinder(state.mix::resetBindings, state.mix::applyMaster,
            state.mix::bindTrack, state.mix::applyTrack)
        sources = SourceControlPump(handler, NativeBridge::nativeSources, bindings::source, { next ->
            state.sources = next
            state.changed()
        }, { error -> state.event("音源接入暂不可用：${error.message}") })
        engine = PlaybackEngine(manager,
            onStatus = { message, active ->
                state.running = active
                state.status = message
                if (!active) state.requested = false
                state.changed()
                if (active) {
                    sources.start()
                    handler.removeCallbacks(poll)
                    handler.post(poll)
                    (application as LanMixApplication).discovery.receiverReady()
                    updateNotification()
                } else finishReceiving()
            },
            onActual = { state.actual = it; state.changed() },
            onReady = {
                bindings.ready()
                sourceDiagnostics.clear()
                state.epoch++
                state.diagnostics = "{}"
                state.sources = emptyList()
                state.event("后台混音已就绪 · 共享播放，不申请音频焦点")
            },
            onEvent = { state.event(it); state.changed() }
        )
        manager.registerAudioDeviceCallback(devices, handler)
        state.observe(notificationObserver)
        discovery.observe(notificationObserver)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStartId = startId
        if (intent?.action == MEDIA_CONTROL) {
            if (foreground && state.requested && state.running) {
                // Re-evaluate the single-device rule at click time, not only while drawing.
                if (!dispatchMediaIntent(intent, discovery.mediaTarget, discovery::sendMedia)) {
                    state.event("媒体指令未发送：当前连接已变化或设备不支持")
                }
            } else if (!state.requested) stopSelfResult(startId)
        } else if (intent?.action == STOP) {
            (application as LanMixApplication).discovery.cancelPending()
            state.requested = false
            state.running = false
            state.status = "正在停止接收…"
            state.event("用户停止后台接收")
            handler.removeCallbacks(poll)
            sources.stop()
            state.changed()
            updateNotification()
            engine.stop() // foreground and wake lock stay until the writer has stopped
        } else if (intent?.action == START) {
            try {
                // Promote before opening audio/network or doing any blocking work.
                val notification = notification(getString(R.string.receiver_notification_starting))
                if (Build.VERSION.SDK_INT >= 29) {
                    startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
                } else startForeground(NOTIFICATION, notification)
                foreground = true
                val settings = ReceiverSettings(intent.getIntExtra("port", 46210),
                    intent.getIntExtra("buffer", 150), intent.getIntExtra("device", -1).takeIf { it >= 0 },
                    intent.getIntExtra("rate", 0), PlaybackMode.fromKey(intent.getStringExtra("mode")))
                require(settings.port in 1..65535 && settings.bufferMs in 20..200 &&
                    (settings.rate == 0 || settings.rate in 8000..192000)) { "接收参数无效" }
                check(NativeBridge.loadError == null) { NativeBridge.loadError ?: "音频核心不可用" }
                if (wakeLock == null) {
                    wakeLock = getSystemService(PowerManager::class.java)
                        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "LanMix:receiver").apply {
                            setReferenceCounted(false)
                            // Renewable two-minute lease; a stalled service cannot
                            // leave an indefinite CPU lock behind.
                            acquire(120_000)
                        }
                    renewWakeAt = SystemClock.elapsedRealtime() + 60_000
                }
                begin(settings, intent.getStringExtra("reason") ?: "用户启动")
            } catch (error: Exception) {
                state.event("后台启动失败：${error.message}")
                state.status = "无法启动后台接收：${error.message}"
                state.requested = false
                state.running = false
                state.changed()
                finishReceiving()
            }
        } else {
            // Never silently restart after a process kill or an unknown intent.
            if (!state.requested) stopSelfResult(startId)
        }
        return START_NOT_STICKY
    }

    private fun begin(settings: ReceiverSettings, reason: String) {
        sources.stop()
        (application as LanMixApplication).discovery.service(true)
        state.config = settings
        state.requested = true
        state.running = false
        state.status = "正在启动后台音频与网络接收…"
        state.actual = "正在打开共享音频输出…"
        state.event("后台接收启动：$reason")
        state.changed()
        val device = manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).find { it.id == settings.deviceId }
        engine.start(PlaybackEngine.Config(settings.port, settings.bufferMs, device, settings.rate, settings.mode),
            reason = reason)
        handler.removeCallbacks(poll)
        handler.post(poll)
        updateNotification()
    }

    private fun notification(message: String): Notification {
        return receiverNotification(this, CHANNEL, message, discovery.mediaTarget)
    }

    private fun updateNotification() {
        if (!foreground || destroyed) return
        val text = when {
            !state.requested -> getString(R.string.receiver_notification_stopping)
            state.running -> getString(R.string.receiver_notification_active, state.config?.port ?: 46210)
            else -> getString(R.string.receiver_notification_starting)
        }
        val key = text to discovery.mediaTarget
        if (key == notificationKey) return
        notificationKey = key
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification(text))
    }

    private fun finishReceiving() {
        (application as LanMixApplication).discovery.service(false)
        handler.removeCallbacks(poll)
        sources.stop()
        state.mix.save()
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        if (foreground) stopForeground(STOP_FOREGROUND_REMOVE)
        foreground = false
        notificationKey = null
        stopSelfResult(lastStartId)
    }

    private fun renewWakeLock() {
        val now = SystemClock.elapsedRealtime()
        if (now < renewWakeAt) return
        wakeLock?.let {
            if (it.isHeld) it.release()
            it.acquire(120_000)
        }
        renewWakeAt = now + 60_000
    }

    override fun onDestroy() {
        destroyed = true
        state.removeObserver(notificationObserver)
        discovery.removeObserver(notificationObserver)
        (application as LanMixApplication).discovery.service(false)
        handler.removeCallbacksAndMessages(null)
        sources.stop()
        manager.unregisterAudioDeviceCallback(devices)
        engine.close()
        state.mix.save()
        state.event("后台服务已结束")
        state.requested = false
        state.running = false
        if (state.status.startsWith("接收中") || state.status.startsWith("正在")) state.status = "已停止接收"
        state.changed()
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        if (foreground) stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
