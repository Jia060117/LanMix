package io.lanmix.receiver

import org.junit.Assert.*
import org.junit.Test

class PlaybackDiagnosticsTest {
    @Test fun duplicateAndTransientRoutesNeverRestartReception() {
        val policy = PlaybackRoutePolicy(48000, true)
        assertEquals(PlaybackRoutePolicy.Action.RETUNE, policy.observe(7, 48000))
        repeat(10) { assertEquals(PlaybackRoutePolicy.Action.NONE, policy.observe(7, 48000)) }
        assertEquals(PlaybackRoutePolicy.Action.NONE, policy.observe(null, 44100))
        assertEquals(PlaybackRoutePolicy.Action.NONE, policy.observe(7, 48000))
        assertEquals(PlaybackRoutePolicy.Action.RETUNE, policy.observe(8, 48000))
    }

    @Test fun onlyAnAutomaticRateChangeRequiresRebuildingNativeMixer() {
        assertEquals(PlaybackRoutePolicy.Action.REBUILD, PlaybackRoutePolicy(48000, true).observe(7, 44100))
        assertEquals(PlaybackRoutePolicy.Action.RETUNE, PlaybackRoutePolicy(48000, false).observe(7, 44100))
    }

    @Test fun historyIsBoundedAndRetainsChronologicalEvidenceAcrossRestarts() {
        val history = DiagnosticHistory(3)
        history.record(0, "start")
        history.record(1, "loss 48")
        history.record(2, "restart")
        history.record(3, "underruns 0")
        assertEquals("+1ms loss 48\n+2ms restart\n+3ms underruns 0", history.text())
    }

    @Test fun silenceAndWritePausesAreCapturedWithoutInventingUnderruns() {
        val timing = PlaybackTiming()
        timing.beginFill(0)
        timing.rendered(100, floatArrayOf(.1f, .1f, 0f, 0f))
        timing.wrote(50, 4)
        timing.beginFill(25_000_000)
        timing.rendered(50, floatArrayOf(0f, 0f, 0f, 0f, .1f, .1f))
        timing.wrote(20, 0)
        timing.rendered(60, floatArrayOf(0f, 0f))
        assertEquals(100L, timing.maxRenderNs)
        assertEquals(50L, timing.maxWriteNs)
        assertEquals(25_000_000L, timing.maxFillGapNs)
        assertEquals(1L, timing.zeroWrites)
        assertEquals(4L, timing.silentFrames)
        assertEquals(3L, timing.maxSilentFrames)
    }

    @Test fun aShortDipIsRetainedBetweenReportsAndContinuousSilenceDoesNotSpamEpisodes() {
        val timing = PlaybackTiming(8000)
        timing.rendered(0, FloatArray(12 * 2))
        timing.rendered(0, FloatArray(12 * 2))
        assertEquals(1L, timing.silenceEpisodes) // one continuous run crossed the 2 ms threshold
        assertEquals(24L, timing.intervalSilentFrames)
        assertEquals(24L, timing.intervalMaxSilentFrames)
        timing.resetInterval()
        timing.rendered(0, FloatArray(10 * 2))
        assertEquals(1L, timing.silenceEpisodes)
        assertEquals(10L, timing.intervalMaxSilentFrames)
        assertEquals(34L, timing.maxSilentFrames)
        timing.rendered(0, floatArrayOf(.1f, .1f))
        timing.rendered(0, FloatArray(16 * 2))
        assertEquals(2L, timing.silenceEpisodes)
    }
}
