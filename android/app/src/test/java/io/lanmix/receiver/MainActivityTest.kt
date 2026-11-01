package io.lanmix.receiver

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 32, 35])
class MainActivityTest {
    @Test fun opensDrawsAndRecreatesAllPages() {
        Robolectric.buildActivity(MainActivity::class.java).use { controller ->
            val activity = controller.setup().get()
            fun texts(view: View): List<TextView> =
                (if (view is TextView) listOf(view) else emptyList()) +
                    (if (view is ViewGroup) (0 until view.childCount).flatMap { texts(view.getChildAt(it)) } else emptyList())
            val root = activity.window.decorView
            fun draw() {
                root.measure(View.MeasureSpec.makeMeasureSpec(1080, View.MeasureSpec.EXACTLY),
                    View.MeasureSpec.makeMeasureSpec(1920, View.MeasureSpec.EXACTLY))
                root.layout(0, 0, 1080, 1920)
                val bitmap = Bitmap.createBitmap(1080, 1920, Bitmap.Config.ARGB_8888)
                try { root.draw(Canvas(bitmap)) } finally { bitmap.recycle() }
            }
            for ((tab, heading) in listOf("混音" to "让声音汇合", "设备" to "附近的设备", "设置" to "按你的习惯")) {
                (texts(root).single { it.text.toString() == tab }.parent as View).performClick()
                draw()
                assertTrue(texts(root).single { it.text.toString() == heading }.isShown)
            }
            controller.recreate()
            assertTrue(texts(controller.get().window.decorView).single { it.text.toString() == "按你的习惯" }.isShown)
        }
    }
}
