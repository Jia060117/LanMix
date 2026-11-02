package io.lanmix.receiver

import android.content.Context
import android.content.res.Configuration
import android.graphics.Color
import android.view.View
import android.widget.TextView
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class EqualizerLayoutTest {
    @Test fun tenVerticalFadersRemainOrderedAndReadableWithNarrowScreensAndLargeText() {
        val app = RuntimeEnvironment.getApplication()
        for (font in listOf(1f, 1.3f, 2f)) for (width in listOf(320, 412, 800)) {
            val context = app.createConfigurationContext(Configuration(app.resources.configuration).apply { fontScale = font })
            val prefs = context.getSharedPreferences("layout-test", Context.MODE_PRIVATE)
            val panel = MixerPanel(context, MixState(prefs), MixerPalette(Color.BLACK, Color.GRAY, Color.BLUE, false), {}, {}, {})
            panel.restoreSelection(null, true)
            panel.measure(View.MeasureSpec.makeMeasureSpec(context.dp(width - 80), View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED))
            panel.layout(0, 0, panel.measuredWidth, panel.measuredHeight)
            val faders = descendants(panel).filterIsInstance<BandFader>()
            assertEquals(10, faders.size)
            faders.zipWithNext().forEach { (left, right) -> assertTrue(left.right <= right.left) }
            faders.forEach {
                assertTrue("Text width at $width / $font", it.width >= it.minimumBandWidth - 1)
                assertTrue("Vertical fader", it.height > it.width)
                assertTrue(it.isFocusable)
            }
            assertEquals(1, descendants(panel).filterIsInstance<TextView>().count { it.text.toString() == "音量" })
        }
    }
}
