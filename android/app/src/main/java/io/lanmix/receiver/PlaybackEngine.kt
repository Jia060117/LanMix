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
import java.util.concurrent.atomic.AtomicReference

/** Lifecycle work runs serially. Only the audio worker renders and writes PCM. */
class PlaybackEngine(
    private val manager: AudioManager,
    private val onStatus: (String, Boolean) -> Unit,
    private val onActual: (String) -> Unit,
    private val onReady: () -> Unit,
    private val onFocusLost: () -> Unit,
    private val onEvent: (String) -> Unit = {}
) {
    data class Config(val port: Int, val bufferMs: Int, val device: AudioDeviceInfo?, val rate: Int,
                      val mode: PlaybackMode = PlaybackMode.STABLE)
    private data class PlaybackReport(
        val rate: Int, val channels: Int, val format: Int, val route: AudioDeviceInfo?,
        val size: Int, val capacity: Int, val queued: Long?, val underruns: Int,
        val mode: PlaybackMode, val increases: Int, val fallback: Boolean, val blockFrames: Int,
        val maxRenderNs: Long, val maxWriteNs: Long, val maxGapNs: Long, val zeroWrites: Long,
        val silentFrames: Long, val maxSilentFrames: Long,
        val intervalNs: Long, val intervalSilentFrames: Long, val intervalMaxSilentFrames: Long
    )
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

    fun start(next: Config, autoRateHint: Int? = null, reason: String = "用户启动或设置变更") {
        if (closed) return
        config = next
        val token = generation.incrementAndGet()
        controller.execute {
            try {
                stopInternal()
                if (generation.get() != token || closed) return@execute
                post(token) { onEvent("接收会话 #$token 启动：$reason；${next.mode.label}") }
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
            val route = AtomicReference<AudioDeviceInfo?>(null)
            val routePolicy = PlaybackRoutePolicy(track.sampleRate, next.rate == 0)
            val checkRoute = Runnable {
                if (generation.get() != token || !live.get() || closed) return@Runnable
                val actual = track.routedDevice ?: return@Runnable
                val autoRate = OutputDevices.autoRate(manager, actual)
                route.set(actual)
                when (routePolicy.observe(actual.id, autoRate)) {
                    PlaybackRoutePolicy.Action.NONE -> Unit
                    PlaybackRoutePolicy.Action.RETUNE -> {
                        retuneRoute.set(true)
                        onEvent("路由确认：${OutputDevices.label(actual)}；保留接收会话")
                    }
                    PlaybackRoutePolicy.Action.REBUILD ->
                        start(next, autoRate, "路由自动采样率 ${track.sampleRate} → $autoRate Hz")
                }
            }
            val routingListener = AudioRouting.OnRoutingChangedListener {
                if (generation.get() != token || !live.get() || closed) return@OnRoutingChangedListener
                handler.removeCallbacks(checkRoute)
                handler.postDelayed(checkRoute, 200) // ignore transient and duplicate route notifications
            }
            track.addOnRoutingChangedListener(routingListener, handler)
            val worker = Thread({
                var failure: String? = null
                var finalUnderruns = 0
                var finalIncreases = 0
                try {
                    Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
                    val rate = track.sampleRate
                    val samples = FloatArray(PlaybackBufferTuner.blockFrames(rate, next.mode) * 2)
                    val counter = PlaybackQueueCounter()
                    val timing = PlaybackTiming(rate)
                    var previousSilenceEpisodes = 0L
                    var reportWindowStart = System.nanoTime()
                    val tuner = PlaybackBufferTuner(rate, next.mode, object : PlaybackBuffer {
                        override val capacityFrames get() = track.bufferCapacityInFrames
                        override val sizeFrames get() = track.bufferSizeInFrames
                        override fun resize(frames: Int) = track.setBufferSizeInFrames(frames)
                    })
                    track.play()
                    handler.post(checkRoute)
                    post(token) { onReady(); onStatus("接收中 · UDP ${next.port} · 网络初始缓冲 ${next.bufferMs} ms · 自适应", true) }
                    var nextReport = SystemClock.elapsedRealtime()
                    while (live.get()) {
                        if (retuneRoute.getAndSet(false)) tuner.refreshRoute()
                        val fillStart = System.nanoTime()
                        timing.beginFill(fillStart)
                        NativeBridge.nativeFill(samples)
                        timing.rendered(System.nanoTime() - fillStart, samples)
                        var offset = 0
                        while (offset < samples.size && live.get()) {
                            val writeStart = System.nanoTime()
                            val written = track.write(samples, offset, samples.size - offset, AudioTrack.WRITE_BLOCKING)
                            timing.wrote(System.nanoTime() - writeStart, written)
                            check(written >= 0 || !live.get()) { "AudioTrack write failed ($written)" }
                            if (written > 0) {
                                offset += written
                                counter.acceptedSamples(written.toLong())
                            }
                            else if (live.get()) Thread.sleep(2)
                        }
                        val now = SystemClock.elapsedRealtime()
                        if (now >= nextReport && live.get()) {
                            val observedUnderruns = track.underrunCount
                            tuner.observeUnderruns(observedUnderruns)
                            val outputChanged = observedUnderruns != finalUnderruns || tuner.increases != finalIncreases
                            val newSilenceEpisodes = timing.silenceEpisodes - previousSilenceEpisodes
                            previousSilenceEpisodes = timing.silenceEpisodes
                            finalUnderruns = observedUnderruns
                            finalIncreases = tuner.increases
                            val queued = counter.queuedFrames(track.playbackHeadPosition)
                            val report = PlaybackReport(rate, track.channelCount, track.audioFormat, route.get(),
                                track.bufferSizeInFrames, track.bufferCapacityInFrames, queued, finalUnderruns,
                                tuner.mode, tuner.increases, tuner.fallback, samples.size / 2,
                                timing.maxRenderNs, timing.maxWriteNs, timing.maxFillGapNs, timing.zeroWrites,
                                timing.silentFrames, timing.maxSilentFrames,
                                System.nanoTime() - reportWindowStart, timing.intervalSilentFrames, timing.intervalMaxSilentFrames)
                            timing.resetInterval()
                            reportWindowStart = System.nanoTime()
                            // Formatting belongs to the UI handler, outside the blocking writer.
                            post(token) {
                                if (live.get()) {
                                    onActual(actualText(report))
                                    if (outputChanged) onEvent("输出缺粮 ${report.underruns}，扩容 ${report.increases}，有效缓冲 ${report.size} 帧")
                                    if (newSilenceEpisodes > 0) onEvent("渲染出现 $newSilenceEpisodes 段至少 2ms 的近静音，统计窗最长 ${report.intervalMaxSilentFrames} 帧；也可能是开播、静音或原曲停顿")
                                }
                            }
                            nextReport = now + 500
                        }
                    }
                } catch (e: Exception) {
                    if (live.get()) failure = e.message ?: e.javaClass.simpleName
                } finally {
                    live.set(false)
                    handler.removeCallbacks(checkRoute)
                    track.removeOnRoutingChangedListener(routingListener)
                    finalUnderruns = runCatching { track.underrunCount }.getOrDefault(finalUnderruns)
                    runCatching { track.pause(); track.flush(); track.stop() }
                    runCatching { NativeBridge.nativeStop() }
                    track.release()
                    manager.abandonAudioFocusRequest(focus)
                    val summary = "会话 #$token 结束：输出缺粮 $finalUnderruns，扩容 $finalIncreases${failure?.let { "，错误 $it" } ?: ""}"
                    handler.post { if (!closed) onEvent(summary) } // survive generation change/rebuild
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

    private fun actualText(report: PlaybackReport): String = with(report) { buildString {
        fun ms(frames: Long) = String.format(Locale.US, "%.1f", frames * 1000.0 / rate)
        fun elapsed(ns: Long) = String.format(Locale.US, "%.2f", ns / 1_000_000.0)
        appendLine("实际输出：${route?.let(OutputDevices::label) ?: "等待系统路由"}")
        appendLine("应用输出：$rate Hz · $channels 声道 · ${OutputDevices.encoding(format)}")
        appendLine("播放缓冲上限：${ms(size.toLong())} ms · $size 帧")
        appendLine("当前排队估算：${queued?.let { "${ms(it)} ms" } ?: "暂不可用"} · 系统容量 ${ms(capacity.toLong())} ms")
        appendLine("输出缺粮 $underruns 次 · 自动扩容 $increases 次${if (fallback) " · 设备拒绝缓冲调节" else ""}")
        appendLine("模式：${mode.label} · 每块 $blockFrames 帧")
        appendLine("本会话最大耗时：渲染 ${elapsed(maxRenderNs)} / 写入 ${elapsed(maxWriteNs)} / 渲染间隔 ${elapsed(maxGapNs)} ms；零写入 $zeroWrites 次")
        appendLine("写入前静音累计 ${ms(silentFrames)} ms / 最长连续 ${ms(maxSilentFrames)} ms（含开播等待、静音和原曲静音）")
        appendLine("最近 ${elapsed(intervalNs)} ms 统计窗：近静音 ${ms(intervalSilentFrames)} ms / 窗内最长 ${ms(intervalMaxSilentFrames)} ms")
        append("排队估算仅覆盖应用 AudioTrack；不代表端到端延迟或耳机 DAC 格式。")
    } }
}
