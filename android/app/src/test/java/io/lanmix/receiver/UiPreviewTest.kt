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
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/** Native Skia exercises shader compilation; previews use the software fallback, not a GPU. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class UiPreviewTest {
    @Test fun rendersBothThemesAndCompilesTheGlassShader() {
        for (theme in listOf("night", "notnight")) {
            RuntimeEnvironment.setQualifiers("w412dp-h916dp-$theme-420dpi")
            Robolectric.buildActivity(MainActivity::class.java).use { controller ->
                val activity = controller.setup().get()
                val root = activity.window.decorView
                fun texts(view: View): List<TextView> = (if (view is TextView) listOf(view) else emptyList()) +
                    ((view as? ViewGroup)?.let { group -> (0 until group.childCount).flatMap { texts(group.getChildAt(it)) } } ?: emptyList())
                // Deterministic idle fixture: a host JVM cannot load Android ARM JNI libraries.
                texts(root).single { it.text.startsWith("无法加载音频核心") }.text = "准备就绪 · 点击开始接收"
                texts(root).single { it.text.toString().endsWith("UDP 46210") }.text = "192.168.1.184 · UDP 46210"
                root.measure(View.MeasureSpec.makeMeasureSpec(1080, View.MeasureSpec.EXACTLY),
                    View.MeasureSpec.makeMeasureSpec(2400, View.MeasureSpec.EXACTLY))
                root.layout(0, 0, 1080, 2400)
                fun findBar(view: View): GlassNavigation? = if (view is GlassNavigation) view else
                    (view as? ViewGroup)?.let { group -> (0 until group.childCount).firstNotNullOfOrNull { findBar(group.getChildAt(it)) } }
                assertTrue("AGSL must compile rather than silently fall back", findBar(root)?.refractionAvailable == true)
                val bitmap = Bitmap.createBitmap(1080, 2400, Bitmap.Config.ARGB_8888)
                try {
                    root.draw(Canvas(bitmap))
                    System.getenv("LANMIX_UI_PREVIEW_DIR")?.let { directory ->
                        val file = File(directory, "mixer-$theme.png")
                        file.parentFile?.mkdirs()
                        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                    }
                } finally { bitmap.recycle() }
            }
        }
    }
}
