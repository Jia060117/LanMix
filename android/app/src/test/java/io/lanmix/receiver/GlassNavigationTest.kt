package io.lanmix.receiver

import android.content.res.Configuration
import android.graphics.Color
import android.view.View
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import kotlin.math.abs
import kotlin.math.roundToInt

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 32, 35])
class GlassNavigationTest {
    @Test fun tabsKeepIconAndLabelTogetherAndCenteredAtDifferentWidthsAndFontSizes() {
        for (font in listOf(1f, 1.3f, 2f)) for (width in listOf(320, 412, 800)) {
            val context = RuntimeEnvironment.getApplication().createConfigurationContext(
                Configuration(RuntimeEnvironment.getApplication().resources.configuration).apply { fontScale = font })
            val density = context.resources.displayMetrics.density
            fun px(dp: Float) = (dp * density).roundToInt()
            val bar = GlassNavigation(context, View(context), true, Color.CYAN) {}
            val height = px(64f + ((font - 1f) * 18).toInt())
            bar.measure(View.MeasureSpec.makeMeasureSpec(px(width - 48f), View.MeasureSpec.AT_MOST),
                View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY))
            bar.layout(0, 0, bar.measuredWidth, height)
            assertTrue(bar.width <= px(336f))
            repeat(3) { index ->
                val tab = bar.getChildAt(index) as LinearLayout
                val icon = tab.getChildAt(0) as ImageView
                val label = tab.getChildAt(1) as TextView
                assertTrue("48 dp touch target", tab.height >= px(48f))
                assertTrue("Icon/label gap", abs(label.top - icon.bottom - px(3f)) <= 1)
                assertTrue("Vertical group center", abs((icon.top + label.bottom) / 2f - tab.height / 2f) <= 1)
                assertTrue("Icon center", abs((icon.left + icon.right) / 2f - tab.width / 2f) <= 1)
                assertTrue("Label center", abs((label.left + label.right) / 2f - tab.width / 2f) <= 1)
                assertTrue("Large text stays inside tab", icon.top >= 0 && label.bottom <= tab.height)
            }
        }
    }

    @Test fun everyTabRemainsClickableAndAnnouncesSelection() {
        val context = RuntimeEnvironment.getApplication()
        var selected = -1
        val bar = GlassNavigation(context, View(context), false, Color.BLUE) { selected = it }
        repeat(3) { index ->
            val tab = bar.getChildAt(index)
            assertTrue(tab.performClick())
            assertEquals(index, selected)
            bar.select(index, false)
            val node = tab.createAccessibilityNodeInfo()
            assertTrue(node.isSelected)
            assertEquals("android.widget.Button", node.className)
            assertTrue(tab.isFocusable)
        }
    }
}
