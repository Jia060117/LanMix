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
    data class Config(val port: Int, val bufferMs: Int, val device: AudioDeviceInfo?, val rate: Int)
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
                post(token) { onStatus("Cannot start: ${e.message}", false) }
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
                post(token) { onStatus("Stopped · receiver socket closed", false); onActual("No active AudioTrack") }
            } catch (e: Exception) {
                post(token) { onStatus("Stop failed: ${e.message}", false) }
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
        // The network jitter buffer is independent of this small playback buffer.
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
            var observedId: Int? = null // main thread routing callback only
            val routingListener = AudioRouting.OnRoutingChangedListener {
                if (generation.get() != token || !live.get() || closed) return@OnRoutingChangedListener
                val actual = track.routedDevice
                val id = actual?.id
                val changed = observedId != null && id != observedId
                observedId = id
                runCatching { actualText(track) }.getOrNull()?.let(onActual)
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
                    val samples = FloatArray(1024 * 2) // allocated once, reused for every fill
                    track.play()
                    post(token) { onReady(); onStatus("Receiving · UDP ${next.port} · ${next.bufferMs} ms jitter buffer", true) }
                    var iteration = 0
                    while (live.get()) {
                        NativeBridge.nativeFill(samples)
                        var offset = 0
                        while (offset < samples.size && live.get()) {
                            val written = track.write(samples, offset, samples.size - offset, AudioTrack.WRITE_BLOCKING)
                            check(written >= 0 || !live.get()) { "AudioTrack write failed ($written)" }
                            if (written > 0) offset += written
                            else if (live.get()) Thread.sleep(2)
                        }
                        if (++iteration % 24 == 0) post(token) {
                            if (live.get()) runCatching { actualText(track) }.getOrNull()?.let(onActual)
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
                    failure?.let { message -> post(token) { onStatus("Playback stopped: $message", false) } }
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

    private fun actualText(track: AudioTrack): String = buildString {
        appendLine("Actual route: ${track.routedDevice?.let(OutputDevices::label) ?: "Pending Android routing"}")
        appendLine("AudioTrack: ${track.sampleRate} Hz · ${track.channelCount} channels · ${OutputDevices.encoding(track.audioFormat)}")
        appendLine("Playback buffer: ${track.bufferSizeInFrames} frames · underruns ${track.underrunCount}")
        append("Track format is observed here. Android may resample downstream; DAC format and end-to-end latency are not reported.")
    }
}
