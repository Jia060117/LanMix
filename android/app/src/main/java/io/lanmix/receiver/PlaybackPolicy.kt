package io.lanmix.receiver

/** Output scheduling only: these modes never alter PCM, EQ, or network buffering. */
enum class PlaybackMode(val key: String, val label: String, val targetMs: Int?) {
    BALANCED("balanced", "均衡 · 播放队列 60 ms", 60),
    STABLE("stable", "稳定优先 · 系统播放缓冲", null),
    RESPONSIVE("responsive", "响应优先 · 播放队列 30 ms", 30),
    COMPATIBLE("compatible", "兼容基线 · 0.1.1 写入方式", null);

    companion object {
        fun fromKey(key: String?) = entries.find { it.key == key } ?: STABLE
    }
}

/** A small fakeable boundary; only the AudioTrack writer calls these methods. */
interface PlaybackBuffer {
    val capacityFrames: Int
    val sizeFrames: Int
    fun resize(frames: Int): Int
}

/** Preserve capacity for recovery; limit the effective write queue instead. */
class PlaybackBufferTuner(
    private val rate: Int,
    val mode: PlaybackMode,
    private val buffer: PlaybackBuffer
) {
    private var requestedFrames = mode.targetMs?.let { framesForMs(rate, it) } ?: 0
    private var lastUnderruns = 0
    var increases = 0
        private set
    var fallback = false
        private set

    init {
        require(rate in 8000..192000)
        refreshRoute()
    }

    fun refreshRoute() {
        if (mode.targetMs == null || fallback) return
        val capacity = buffer.capacityFrames.coerceAtLeast(1)
        val desired = requestedFrames.coerceIn(1, capacity)
        if (buffer.sizeFrames == desired) return
        val result = runCatching { buffer.resize(desired) }.getOrDefault(-1)
        if (result <= 0) {
            // A rejected/unsupported limit must not prevent playback. Try to
            // restore the original capacity and keep compatibility until restart.
            fallback = true
            runCatching { buffer.resize(capacity) }
        } else {
            // Some routes enforce a larger minimum only while being opened.
            // Retain the actual accepted floor after later notifications too.
            requestedFrames = maxOf(requestedFrames, result)
        }
    }

    fun observeUnderruns(total: Int) {
        val delta = total - lastUnderruns
        lastUnderruns = total
        if (delta <= 0 || mode.targetMs == null || fallback) return
        val capacity = buffer.capacityFrames.coerceAtLeast(1)
        val current = maxOf(buffer.sizeFrames, requestedFrames).coerceIn(1, capacity)
        if (current == capacity) return
        val increment = maxOf(framesForMs(rate, 10), if (delta > 1) current else current / 2)
        requestedFrames = (current.toLong() + increment).coerceAtMost(capacity.toLong()).toInt()
        increases++
        refreshRoute()
    }

    companion object {
        fun framesForMs(rate: Int, ms: Int): Int = ((rate.toLong() * ms + 999) / 1000).toInt().coerceAtLeast(1)
        fun blockFrames(rate: Int, mode: PlaybackMode = PlaybackMode.BALANCED): Int =
            if (mode == PlaybackMode.COMPATIBLE) 1024 else framesForMs(rate, 10).coerceIn(1, 4096)
    }
}

/** Counts only accepted samples and extends Android's unsigned 32-bit head. */
class PlaybackQueueCounter {
    private var writtenSamples = 0L
    private var previousHead = 0L
    private var playedFrames = 0L

    fun acceptedSamples(count: Long) {
        require(count >= 0 && count <= Long.MAX_VALUE - writtenSamples)
        writtenSamples += count
    }

    fun queuedFrames(rawHead: Int): Long? {
        val head = rawHead.toLong() and 0xffff_ffffL
        val delta = (head - previousHead) and 0xffff_ffffL
        val candidate = playedFrames + delta
        // A regressing, reset, or otherwise inconsistent device counter is
        // unknown, not a fabricated multi-hour queue. Never use it to pace DSP.
        if (candidate > writtenSamples / 2) return null
        previousHead = head
        playedFrames = candidate
        return writtenSamples / 2 - candidate
    }
}
