package io.lanmix.receiver

import org.junit.Assert.*
import org.junit.Test

class PlaybackPolicyTest {
    private class Buffer(var capacity: Int = 26936, var minimum: Int = 1) : PlaybackBuffer {
        var size = capacity
        var reject = false
        var throws = false
        val requests = mutableListOf<Int>()
        override val capacityFrames get() = capacity
        override val sizeFrames get() = size
        override fun resize(frames: Int): Int {
            requests.add(frames)
            if (throws) throw IllegalStateException("device rejected tuning")
            if (reject) return -3
            size = frames.coerceIn(minimum, capacity)
            return size
        }
    }

    @Test fun limitsEffectiveQueueWhileRetainingRecoveryCapacity() {
        val buffer = Buffer()
        PlaybackBufferTuner(48000, PlaybackMode.BALANCED, buffer)
        assertEquals(2880, buffer.size)
        assertEquals(26936, buffer.capacity)
    }

    @Test fun deviceClampIsUsedAsTheGrowthBaseline() {
        val buffer = Buffer(minimum = 4800)
        val tuner = PlaybackBufferTuner(48000, PlaybackMode.BALANCED, buffer)
        assertEquals(4800, buffer.size)
        tuner.observeUnderruns(1)
        assertEquals(7200, buffer.size)
        tuner.observeUnderruns(1)
        assertEquals(7200, buffer.size) // cumulative counts must not repeatedly grow
        assertEquals(1, tuner.increases)
    }

    @Test fun repeatedUnderrunsQuicklyRecoverAndStopAtCapacity() {
        val buffer = Buffer()
        val tuner = PlaybackBufferTuner(48000, PlaybackMode.BALANCED, buffer)
        tuner.observeUnderruns(3)
        assertEquals(5760, buffer.size)
        repeat(20) { tuner.observeUnderruns(6 + it * 3) }
        assertEquals(buffer.capacity, buffer.size)
        val count = buffer.requests.size
        tuner.observeUnderruns(1000)
        assertEquals(count, buffer.requests.size)
    }

    @Test fun aRouteCanChangeCapacityAndNeedsItsLimitReapplied() {
        val buffer = Buffer(capacity = 2048)
        val tuner = PlaybackBufferTuner(48000, PlaybackMode.BALANCED, buffer)
        assertEquals(2048, buffer.size)
        buffer.capacity = 26936
        buffer.size = buffer.capacity
        tuner.refreshRoute()
        assertEquals(2880, buffer.size)
        tuner.observeUnderruns(1)
        assertEquals(4320, buffer.size)
        buffer.size = buffer.capacity
        tuner.refreshRoute()
        assertEquals(4320, buffer.size) // route refresh must preserve learned safety
    }

    @Test fun aRejectedResizeFallsBackWithoutFailingPlayback() {
        for (throws in listOf(false, true)) {
            val buffer = Buffer().apply { reject = true; this.throws = throws }
            val tuner = PlaybackBufferTuner(48000, PlaybackMode.BALANCED, buffer)
            assertTrue(tuner.fallback)
            assertEquals(listOf(2880, 26936), buffer.requests)
            tuner.observeUnderruns(3)
            tuner.refreshRoute()
            assertEquals(2, buffer.requests.size)
        }
    }

    @Test fun compatibilityLeavesPlatformBufferAlone() {
        val buffer = Buffer()
        val tuner = PlaybackBufferTuner(48000, PlaybackMode.COMPATIBLE, buffer)
        tuner.observeUnderruns(20)
        tuner.refreshRoute()
        assertTrue(buffer.requests.isEmpty())
        assertEquals(buffer.capacity, buffer.size)
    }

    @Test fun blocksAndModesCoverAllSupportedRates() {
        for (rate in listOf(8000, 44100, 48000, 96000, 192000)) {
            val frames = PlaybackBufferTuner.blockFrames(rate)
            assertTrue(frames in 1..4096)
            assertTrue(frames * 1000L >= rate * 10L)
        }
        assertEquals(PlaybackMode.BALANCED, PlaybackMode.fromKey("unknown"))
        assertEquals(PlaybackMode.STABLE, PlaybackMode.fromKey("stable"))
    }

    @Test fun queueCountsPartialWritesInStereoFrames() {
        val counter = PlaybackQueueCounter()
        counter.acceptedSamples(3)
        assertEquals(1L, counter.queuedFrames(0))
        counter.acceptedSamples(3)
        assertEquals(2L, counter.queuedFrames(1))
        assertEquals(0L, counter.queuedFrames(3))
    }

    @Test fun queueHandlesSignedBoundaryAndUnsignedWrap() {
        val counter = PlaybackQueueCounter()
        val writtenFrames = 0x1_0000_0010L
        counter.acceptedSamples(writtenFrames * 2)
        assertEquals(writtenFrames - 0x7fff_ffffL, counter.queuedFrames(0x7fff_ffff))
        assertEquals(writtenFrames - 0x8000_0000L, counter.queuedFrames(Int.MIN_VALUE))
        assertEquals(17L, counter.queuedFrames(-1))
        assertEquals(14L, counter.queuedFrames(2))
    }

    @Test fun invalidHeadIsUnknownAndCanRecoverFromATransientRegression() {
        val counter = PlaybackQueueCounter()
        counter.acceptedSamples(2000)
        assertEquals(900L, counter.queuedFrames(100))
        assertNull(counter.queuedFrames(90))
        assertNull(counter.queuedFrames(1500))
        assertEquals(800L, counter.queuedFrames(200))
    }
}
