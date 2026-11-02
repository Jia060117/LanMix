package io.lanmix.receiver

import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.os.Looper
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.SeekBar
import android.widget.TextView
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.time.Duration

internal fun descendants(view: View): List<View> = listOf(view) +
    ((view as? ViewGroup)?.let { group -> (0 until group.childCount).flatMap { descendants(group.getChildAt(it)) } } ?: emptyList())

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 32, 35])
class MixerPanelTest {
    private class Fixture {
        val context: Context = RuntimeEnvironment.getApplication()
        val prefs = context.getSharedPreferences("panel-test", Context.MODE_PRIVATE).apply { edit().clear().commit() }
        val mix = MixState(prefs)
        val applied = mutableListOf<Int?>()
        var saves = 0
        val panel = MixerPanel(context, mix, MixerPalette(Color.BLACK, Color.GRAY, Color.BLUE, false),
            { applied.add(it) }, { saves++; mix.save() }, {})
        val sources = listOf(MixerSource("192.168.1.20/1", 0, "台式机", true), MixerSource("192.168.1.30/1", 1, "笔记本", true))
        init {
            mix.bindTrack(0, "192.168.1.20", 1); mix.bindTrack(1, "192.168.1.30", 1)
            mix.tracks[0].gainDb = -3f; mix.tracks[1].gainDb = -8f
            panel.setSources(sources)
        }
        fun click(text: String) { descendants(panel).filterIsInstance<TextView>().single { it.text.toString() == text }.performClick() }
        fun volume() = descendants(panel).filterIsInstance<SeekBar>().single { it.contentDescription?.startsWith("当前调音对象") == true }
        fun progress(view: View, value: Float) = view.performAccessibilityAction(
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS.id,
            Bundle().apply { putFloat(AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE, value) })
    }

    @Test fun oneEditorTargetsOnlyTheSelectedDeviceAndSwitchingNeverWritesAudio() {
        val f = Fixture()
        assertEquals(1, descendants(f.panel).filterIsInstance<SeekBar>().count { it.contentDescription?.startsWith("当前调音对象") == true })
        f.click("台式机")
        assertTrue(f.applied.isEmpty())
        assertTrue(f.progress(f.volume(), 96f)) // -12 dB
        assertEquals(-12f, f.mix.tracks[0].gainDb)
        assertEquals(-8f, f.mix.tracks[1].gainDb)
        assertEquals(-6f, f.mix.master.gainDb)
        assertEquals(listOf(0), f.applied)
        f.click("整体")
        assertEquals(108, f.volume().progress)
        assertEquals(listOf(0), f.applied)
        assertTrue(f.progress(f.volume(), 100f))
        assertEquals(-10f, f.mix.master.gainDb)
        assertEquals(-12f, f.mix.tracks[0].gainDb)
    }

    @Test fun presetsManualChangesAndSavedSettingsRemainIndependentAcrossSources() {
        val f = Fixture()
        f.click("台式机"); f.click("人声")
        assertEquals(listOf(0), f.applied) // Visual interpolation must not issue more DSP writes.
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(250))
        assertEquals("Animation only draws; it never reapplies audio", listOf(0), f.applied)
        assertEquals(EqPreset.VOCAL, EqPreset.matching(f.mix.tracks[0]))
        assertFalse(f.mix.master.eqEnabled); assertFalse(f.mix.tracks[1].eqEnabled)
        assertEquals(-3f, f.mix.tracks[0].gainDb)
        val toggle = descendants(f.panel).filterIsInstance<android.widget.Switch>().single()
        toggle.isChecked = false
        assertFalse(f.mix.tracks[0].eqEnabled)
        assertEquals(EqPreset.VOCAL, EqPreset.matching(f.mix.tracks[0]))
        toggle.isChecked = true
        val firstBand = descendants(f.panel).filterIsInstance<BandFader>().first()
        assertTrue(f.progress(firstBand, 1.5f))
        assertNull(EqPreset.matching(f.mix.tracks[0]))
        assertTrue(descendants(f.panel).filterIsInstance<TextView>().any { it.text.toString().startsWith("音效 · 自定义") })
        f.click("笔记本"); f.click("低音增强")
        assertEquals(EqPreset.BASS, EqPreset.matching(f.mix.tracks[1]))
        val restored = MixState(f.prefs)
        restored.bindTrack(4, "192.168.1.20", 1); restored.bindTrack(5, "192.168.1.30", 1)
        assertEquals(1.5f, restored.tracks[4].bands[0]); assertNull(EqPreset.matching(restored.tracks[4]))
        assertEquals(EqPreset.BASS, EqPreset.matching(restored.tracks[5]))
        assertEquals(-6f, restored.master.gainDb)
    }

    @Test fun pollingKeepsTheSelectedEditorAndRecycledSlotsCannotReceiveStaleEdits() {
        val f = Fixture()
        f.click("台式机")
        repeat(5) { f.panel.setSources(f.sources) }
        assertEquals("192.168.1.20/1", f.panel.selectedSourceKey)
        assertTrue(f.applied.isEmpty())
        f.panel.markOffline()
        assertEquals("192.168.1.20/1", f.panel.selectedSourceKey)
        assertEquals(-3f, f.mix.tracks[0].gainDb)
        f.panel.setSources(f.sources)
        f.mix.bindTrack(0, "192.168.1.99", 2)
        f.progress(descendants(f.panel).filterIsInstance<BandFader>().first(), 9f)
        assertTrue(f.applied.isEmpty())
        assertEquals(0f, f.mix.tracks[0].bands[0])
        assertNull(f.panel.selectedSourceKey)
        f.panel.setSources(listOf(MixerSource("192.168.1.99/2", 0, "新电脑", true)))
        f.click("新电脑"); f.click("摇滚")
        assertEquals(listOf(0), f.applied)
        assertEquals(EqPreset.ROCK, EqPreset.matching(f.mix.tracks[0]))
        f.panel.clearSources()
        assertNull(f.panel.selectedSourceKey)
    }

    @Test fun faderDistinguishesHorizontalSwipesFromVerticalAdjustmentAndExposesDbRange() {
        val f = Fixture()
        val band = descendants(f.panel).filterIsInstance<BandFader>()[3]
        band.layout(0, 0, 80, 500)
        fun touch(action: Int, x: Float, y: Float) {
            val event = MotionEvent.obtain(0, 0, action, x, y, 0)
            try { band.onTouchEvent(event) } finally { event.recycle() }
        }
        touch(MotionEvent.ACTION_DOWN, 40f, 250f)
        touch(MotionEvent.ACTION_MOVE, 240f, 252f)
        touch(MotionEvent.ACTION_UP, 240f, 252f)
        assertTrue(f.applied.isEmpty())
        touch(MotionEvent.ACTION_DOWN, 40f, 250f)
        touch(MotionEvent.ACTION_MOVE, 40f, 100f)
        touch(MotionEvent.ACTION_UP, 40f, 100f)
        assertTrue(f.mix.master.bands[3] > 0f)
        assertTrue(f.mix.master.eqEnabled)
        val node = band.createAccessibilityNodeInfo()
        assertEquals("android.widget.SeekBar", node.className)
        assertEquals(-12f, node.rangeInfo.min); assertEquals(12f, node.rangeInfo.max)
        assertTrue(f.progress(band, 40f)); assertEquals(12f, f.mix.master.bands[3])
        assertFalse(f.progress(band, Float.NaN)); assertEquals(12f, f.mix.master.bands[3])
    }

    @Test fun targetRemovalFallsBackToOverallAndRestorationDoesNotApplySettings() {
        val f = Fixture()
        f.panel.restoreSelection("192.168.1.30/1", true)
        f.panel.setSources(f.sources)
        assertEquals("192.168.1.30/1", f.panel.selectedSourceKey)
        assertTrue(f.panel.equalizerExpanded)
        f.panel.setSources(f.sources.take(1))
        assertNull(f.panel.selectedSourceKey)
        assertEquals(108, f.volume().progress)
        assertTrue(f.applied.isEmpty())
    }

    @Test fun changingTargetDuringAnEqGestureCancelsTheOldGesture() {
        val f = Fixture()
        val band = descendants(f.panel).filterIsInstance<BandFader>()[2]
        band.layout(0, 0, 80, 500)
        fun touch(action: Int, y: Float) {
            val event = MotionEvent.obtain(0, 0, action, 40f, y, 0)
            try { band.onTouchEvent(event) } finally { event.recycle() }
        }
        touch(MotionEvent.ACTION_DOWN, 250f)
        f.click("笔记本")
        touch(MotionEvent.ACTION_MOVE, 80f); touch(MotionEvent.ACTION_UP, 80f)
        assertTrue(f.applied.isEmpty())
        assertEquals(0f, f.mix.master.bands[2]); assertEquals(0f, f.mix.tracks[1].bands[2])
    }
}
