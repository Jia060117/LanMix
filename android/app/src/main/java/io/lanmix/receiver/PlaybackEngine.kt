package io.lanmix.receiver

import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRouting
import android.media.AudioTrack
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.os.SystemClock
import java.util.Locale
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/** Lifecycle work runs serially. Only the audio worker renders and writes PCM. */
class PlaybackEngine(
    private val manager: AudioManager,
    private val onStatus: (String, Boolean) -> Unit,
    private val onActual: (String) -> Unit,
    private val onReady: () -> Unit,
    private val onFocusLost: () -> Unit
) {
    data class Config(val port: Int, val bufferMs: Int, val device: AudioDeviceInfo?, val rate: Int,
                      val mode: PlaybackMode = PlaybackMode.BALANCED)
    private class Session(val track: AudioTrack, val live: AtomicBoolean, val worker: Thread)
    private val handler = Handler(Looper.getMainLooper())
    private val controller = Executors.newSingleThreadExecutor { Thread(it, "LanMix-lifecycle") }
    private val generation = AtomicInteger()
    private var session: Session? = null // controller thread only
    @Volatile private var closed = false
    @Volatile private var config: Config? = null
    @Volatile private var focusRequest: AudioFocusRequest? = null
    private val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()

    fun start(next: Config, autoRateHint: Int? = null) {
        if (closed) return
        config = next
        val token = generation.incrementAndGet()
        controller.execute {
            try {
                stopInternal()
                if (generation.get() != token || closed) return@execute
                open(next, token, autoRateHint)
            } catch (e: Exception) {
                post(token) { onStatus("无法启动：${e.message}", false) }
            }
        }
    }

    fun stop() {
        if (closed) return
        config = null
        val token = generation.incrementAndGet()
        controller.execute {
            try {
                stopInternal()
                post(token) { onStatus("已停止接收", false); onActual("等待启动音频输出") }
            } catch (e: Exception) {
                post(token) { onStatus("停止失败：${e.message}", false) }
            }
        }
    }

    fun close() {
        if (closed) return
        closed = true
        config = null
        generation.incrementAndGet()
        controller.execute { runCatching { stopInternal() } }
        controller.shutdown()
    }

    private fun open(next: Config, token: Int, autoRateHint: Int?) {
        val rate = next.rate.takeIf { it != 0 } ?: autoRateHint ?: OutputDevices.autoRate(manager, next.device)
        val format = AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
            .setSampleRate(rate).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).build()
        val minimum = AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_FLOAT)
        check(minimum > 0) { "PCM float stereo unavailable at $rate Hz (code $minimum)" }
        // Retain platform capacity for recovery; the writer limits effective size.
        val track = AudioTrack.Builder().setAudioAttributes(attributes).setAudioFormat(format)
            .setTransferMode(AudioTrack.MODE_STREAM).setBufferSizeInBytes(maxOf(minimum * 2, 8192)).build()
        var nativeStarted = false
        try {
            check(track.state == AudioTrack.STATE_INITIALIZED) { "AudioTrack initialization failed" }
            if (next.device != null) check(track.setPreferredDevice(next.device)) { "Output preference rejected" }
            val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN).setAudioAttributes(attributes)
                .setOnAudioFocusChangeListener({ change ->
                    if (change != AudioManager.AUDIOFOCUS_GAIN && generation.get() == token && !closed) onFocusLost()
                }, handler).build()
            check(manager.requestAudioFocus(focus) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) { "Audio focus denied" }
            focusRequest = focus
            NativeBridge.nativeStart(next.port, next.bufferMs, track.sampleRate)
            nativeStarted = true
            val live = AtomicBoolean(true)
            val retuneRoute = AtomicBoolean(false)
            var observedId: Int? = null // main thread routing callback only
            val routingListener = AudioRouting.OnRoutingChangedListener {
                if (generation.get() != token || !live.get() || closed) return@OnRoutingChangedListener
                val actual = track.routedDevice
                val id = actual?.id
                val changed = observedId != null && id != observedId
                observedId = id
                retuneRoute.set(true) // only the writer changes effective buffer size
                val autoRate = OutputDevices.autoRate(manager, actual)
                if (changed || (next.rate == 0 && actual != null && track.sampleRate != autoRate)) {
                    // For a system-selected output, carry the new device's native rate into
                    // this rebuild. Manual rate preferences remain explicit.
                    start(next, if (next.rate == 0) autoRate else null)
                }
            }
            track.addOnRoutingChangedListener(routingListener, handler)
            val worker = Thread({
                var failure: String? = null
                try {
                    Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
                    val rate = track.sampleRate
                    val samples = FloatArray(PlaybackBufferTuner.blockFrames(rate) * 2)
                    val counter = PlaybackQueueCounter()
                    val tuner = PlaybackBufferTuner(rate, next.mode, object : PlaybackBuffer {
                        override val capacityFrames get() = track.bufferCapacityInFrames
                        override val sizeFrames get() = track.bufferSizeInFrames
                        override fun resize(frames: Int) = track.setBufferSizeInFrames(frames)
                    })
                    track.play()
                    post(token) { onReady(); onStatus("接收中 · UDP ${next.port} · 网络初始缓冲 ${next.bufferMs} ms · 自适应", true) }
                    var nextReport = SystemClock.elapsedRealtime()
                    while (live.get()) {
                        if (retuneRoute.getAndSet(false)) tuner.refreshRoute()
                        NativeBridge.nativeFill(samples)
                        var offset = 0
                        while (offset < samples.size && live.get()) {
                            val written = track.write(samples, offset, samples.size - offset, AudioTrack.WRITE_BLOCKING)
                            check(written >= 0 || !live.get()) { "AudioTrack write failed ($written)" }
                            if (written > 0) {
                                offset += written
                                counter.acceptedSamples(written.toLong())
                            }
                            else if (live.get()) Thread.sleep(2)
                        }
                        val now = SystemClock.elapsedRealtime()
                        if (now >= nextReport && live.get()) {
                            tuner.observeUnderruns(track.underrunCount)
                            val queued = counter.queuedFrames(track.playbackHeadPosition)
                            val report = actualText(track, tuner, queued)
                            post(token) { if (live.get()) onActual(report) }
                            nextReport = now + 500
                        }
                    }
                } catch (e: Exception) {
                    if (live.get()) failure = e.message ?: e.javaClass.simpleName
                } finally {
                    live.set(false)
                    track.removeOnRoutingChangedListener(routingListener)
                    runCatching { track.pause(); track.flush(); track.stop() }
                    runCatching { NativeBridge.nativeStop() }
                    track.release()
                    manager.abandonAudioFocusRequest(focus)
                    failure?.let { message -> post(token) { onStatus("播放已停止：$message", false) } }
                }
            }, "LanMix-PCM-float")
            session = Session(track, live, worker)
            worker.start()
        } catch (e: Exception) {
            if (nativeStarted) NativeBridge.nativeStop()
            focusRequest?.let { manager.abandonAudioFocusRequest(it) }
            focusRequest = null
            track.release()
            throw e
        }
    }

    private fun stopInternal() {
        val current = session
        if (current != null) {
            current.live.set(false)
            // pause/flush interrupts a blocking write; release belongs to the worker.
            runCatching { current.track.pause(); current.track.flush() }
            current.worker.join(3000)
            check(!current.worker.isAlive) { "Audio worker did not stop; replacement refused" }
            session = null
        }
        focusRequest?.let { manager.abandonAudioFocusRequest(it) }
        focusRequest = null
    }

    private fun post(token: Int, action: () -> Unit) {
        handler.post { if (generation.get() == token && !closed) action() }
    }

    private fun actualText(track: AudioTrack, tuner: PlaybackBufferTuner, queued: Long?): String = buildString {
        fun ms(frames: Long) = String.format(Locale.US, "%.1f", frames * 1000.0 / track.sampleRate)
        appendLine("实际输出：${track.routedDevice?.let(OutputDevices::label) ?: "等待系统路由"}")
        appendLine("应用输出：${track.sampleRate} Hz · ${track.channelCount} 声道 · ${OutputDevices.encoding(track.audioFormat)}")
        appendLine("播放缓冲上限：${ms(track.bufferSizeInFrames.toLong())} ms · ${track.bufferSizeInFrames} 帧")
        appendLine("当前排队估算：${queued?.let { "${ms(it)} ms" } ?: "暂不可用"} · 系统容量 ${ms(track.bufferCapacityInFrames.toLong())} ms")
        appendLine("输出缺粮 ${track.underrunCount} 次 · 自动扩容 ${tuner.increases} 次${if (tuner.fallback) " · 设备拒绝缓冲调节" else ""}")
        appendLine("模式：${tuner.mode.label}")
        append("排队估算仅覆盖应用 AudioTrack；不代表端到端延迟或耳机 DAC 格式。")
    }
}
