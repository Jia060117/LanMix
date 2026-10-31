package io.lanmix.receiver

import org.junit.Assert.*
import org.junit.Test

class SourceControlBinderTest {
    // No Activity, UI observer or Android runtime: exercise the same binding
    // gate the background service calls, with distinct saved source settings.
    private class Fixture {
        val identities = mutableMapOf<Int, String>()
        val savedGain = mapOf("desktop/1" to -12f, "laptop/2" to -3f)
        val audibleGain = mutableMapOf<Int, Float>()
        var applied = 0
        var masterApplications = 0
        val binder = SourceControlBinder(
            { identities.clear(); audibleGain.clear() },
            { masterApplications++ },
            { slot, source, stream -> identities.put(slot, "$source/$stream") != "$source/$stream" },
            { slot -> audibleGain[slot] = savedGain.getValue(identities.getValue(slot)); applied++ }
        )
    }

    @Test fun sourcesJoiningWithoutAnyScreenReceiveTheirOwnSavedGain() {
        val f = Fixture()
        f.binder.ready()
        f.binder.source(0, "desktop", 1)
        f.binder.source(1, "laptop", 2)
        assertEquals(-12f, f.audibleGain.getValue(0))
        assertEquals(-3f, f.audibleGain.getValue(1))
        assertEquals(1, f.masterApplications)
    }

    @Test fun pollingDoesNotOverwriteLiveControlsAndSlotReuseAppliesNewIdentity() {
        val f = Fixture()
        f.binder.ready()
        f.binder.source(0, "desktop", 1)
        f.audibleGain[0] = -20f // user moves a slider while playing
        repeat(20) { f.binder.source(0, "desktop", 1) }
        assertEquals(1, f.applied)
        assertEquals(-20f, f.audibleGain.getValue(0))
        f.binder.source(0, "laptop", 2)
        assertEquals(-3f, f.audibleGain.getValue(0))
        assertEquals(2, f.applied)
    }

    @Test fun nativeRestartReappliesKnownSourcesAndInvalidSlotsCannotTouchControls() {
        val f = Fixture()
        f.binder.ready()
        f.binder.source(0, "desktop", 1)
        f.binder.ready() // new native mixer is muted again
        f.binder.source(0, "desktop", 1)
        f.binder.source(-1, "missing", 0)
        f.binder.source(16, "missing", 0)
        assertEquals(2, f.applied)
        assertEquals(2, f.masterApplications)
        assertEquals(mapOf(0 to -12f), f.audibleGain)
    }

    @Test fun aTemporaryControlFailureDoesNotLeaveTheAlreadyBoundSourcePermanentlyMuted() {
        val identities = mutableMapOf<Int, String>()
        var attempts = 0
        var audible = false
        val binder = SourceControlBinder({ identities.clear() }, {},
            { slot, source, stream -> identities.put(slot, "$source/$stream") != "$source/$stream" },
            { attempts++; if (attempts == 1) error("native receiver changing") else audible = true })
        binder.ready()
        runCatching { binder.source(0, "desktop", 1) }
        assertFalse(audible)
        binder.source(0, "desktop", 1)
        assertTrue(audible)
        binder.source(0, "desktop", 1)
        assertEquals(2, attempts)
    }
}
