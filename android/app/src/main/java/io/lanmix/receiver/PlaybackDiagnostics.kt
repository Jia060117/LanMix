package io.lanmix.receiver

/** Same-rate routing is handled by AudioTrack; only a new automatic rate rebuilds JNI. */
class PlaybackRoutePolicy(private val rate: Int, private val automatic: Boolean) {
    enum class Action { NONE, RETUNE, REBUILD }
    private var observedId: Int? = null

    fun observe(id: Int?, proposedRate: Int): Action {
        if (id == null) return Action.NONE // transient unavailable routing is not disconnection
        if (automatic && proposedRate != rate) return Action.REBUILD
        if (id == observedId) return Action.NONE
        observedId = id
        return Action.RETUNE
    }
}

/** Process-owned history survives screens and receiver restarts, never written to disk. */
class DiagnosticHistory(private val capacity: Int = 64) {
    private val events = ArrayDeque<String>()
    init { require(capacity > 0) }
    fun record(elapsedMs: Long, event: String) {
        if (events.size == capacity) events.removeFirst()
        events.addLast("+${elapsedMs}ms $event")
    }
    fun text(): String = events.joinToString("\n")
}

/** Writer-owned numeric metrics. Maxima are observations, not dropout verdicts. */
class PlaybackTiming(rate: Int = 48000) {
    var maxRenderNs = 0L
        private set
    var maxWriteNs = 0L
        private set
    var maxFillGapNs = 0L
        private set
    var zeroWrites = 0L
        private set
    var silentFrames = 0L
        private set
    var maxSilentFrames = 0L
        private set
    var intervalSilentFrames = 0L
        private set
    var intervalMaxSilentFrames = 0L
        private set
    var silenceEpisodes = 0L
        private set
    private val minimumSilenceFrames = PlaybackBufferTuner.framesForMs(rate, 2).toLong()
    private var silentRun = 0L
    private var intervalSilentRun = 0L
    private var previousFill = 0L
    private var hasPreviousFill = false

    fun beginFill(nowNs: Long) {
        if (hasPreviousFill) maxFillGapNs = maxOf(maxFillGapNs, (nowNs - previousFill).coerceAtLeast(0))
        previousFill = nowNs
        hasPreviousFill = true
    }
    fun rendered(elapsedNs: Long, samples: FloatArray) {
        maxRenderNs = maxOf(maxRenderNs, elapsedNs.coerceAtLeast(0))
        // Track exact/near-silence runs, including short dips between UI polls.
        // Natural silence, mute and startup also contribute; this is not an error count.
        var index = 0
        while (index + 1 < samples.size) {
            if (kotlin.math.abs(samples[index]) < 1e-7f && kotlin.math.abs(samples[index + 1]) < 1e-7f) {
                silentFrames++
                intervalSilentFrames++
                silentRun++
                intervalSilentRun++
                maxSilentFrames = maxOf(maxSilentFrames, silentRun)
                intervalMaxSilentFrames = maxOf(intervalMaxSilentFrames, intervalSilentRun)
                if (silentRun == minimumSilenceFrames) silenceEpisodes++
            } else {
                silentRun = 0
                intervalSilentRun = 0
            }
            index += 2
        }
    }
    fun wrote(elapsedNs: Long, acceptedSamples: Int) {
        maxWriteNs = maxOf(maxWriteNs, elapsedNs.coerceAtLeast(0))
        if (acceptedSamples == 0) zeroWrites++
    }
    fun resetInterval() {
        intervalSilentFrames = 0
        intervalMaxSilentFrames = 0
        intervalSilentRun = 0
    }
}
