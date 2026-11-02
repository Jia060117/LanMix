package io.lanmix.receiver

import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28])
class EqPresetTest {
    @Test fun presetsLeaveListeningControlsAloneAndProvideBoostHeadroom() {
        EqPreset.entries.forEach { preset ->
            val control = MixControl().apply { gainDb = -9f; mute = true; solo = true }
            preset.applyTo(control)
            assertEquals(preset, EqPreset.matching(control))
            assertTrue(control.eqEnabled)
            assertEquals(10, control.bands.size)
            assertTrue(control.bands.all { it.isFinite() && it in -12f..12f })
            assertTrue(control.preampDb <= -(control.bands.maxOrNull() ?: 0f))
            assertEquals(-9f, control.gainDb); assertTrue(control.mute); assertTrue(control.solo)
        }
    }

    @Test fun manualCurveAndPreampRoundTripWithoutDependingOnAPresetName() {
        val first = MixControl()
        EqPreset.POP.applyTo(first)
        first.bands[8] += .5f
        val second = MixControl().apply { load(first.json()) }
        assertArrayEquals(first.bands, second.bands, 0f)
        assertNull(EqPreset.matching(second))
        EqPreset.FLAT.applyTo(second)
        assertEquals(EqPreset.FLAT, EqPreset.matching(second))
        assertEquals(0f, second.preampDb)
        assertArrayEquals(FloatArray(10), second.bands, 0f)
    }
}
