package io.lanmix.receiver

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import android.widget.ScrollView
import org.junit.Assert.assertTrue
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File
import java.time.Duration
import android.os.Looper
import org.robolectric.Shadows.shadowOf

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
                fun capture(name: String) {
                    val bitmap = Bitmap.createBitmap(1080, 2400, Bitmap.Config.ARGB_8888)
                    try {
                        root.draw(Canvas(bitmap))
                        System.getenv("LANMIX_UI_PREVIEW_DIR")?.let { directory ->
                            val file = File(directory, "$name-$theme.png")
                            file.parentFile?.mkdirs()
                            file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                        }
                    } finally { bitmap.recycle() }
                }
                capture("mixer")
                // Freeze background discovery before installing the offline UI fixture.
                // Otherwise its next poll removes these host-only demo sources.
                controller.pause().stop()
                val mix = (activity.application as LanMixApplication).receiver.mix
                mix.bindTrack(0, "192.168.1.20", 1); mix.bindTrack(1, "192.168.1.30", 1)
                mix.tracks[0].gainDb = -3f; mix.tracks[1].gainDb = -8f
                val panel = descendants(root).filterIsInstance<MixerPanel>().single()
                panel.setSources(listOf(MixerSource("192.168.1.20/1", 0, "台式机", true), MixerSource("192.168.1.30/1", 1, "笔记本", true)))
                texts(root).single { it.text.toString() == "笔记本" }.performClick()
                texts(root).single { it.text.toString().startsWith("音效 ·") }.performClick()
                descendants(root).filterIsInstance<MixerChip>().single { it.text.toString() == "人声" }.performClick()
                shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(250))
                assertEquals("192.168.1.30/1", panel.selectedSourceKey)
                assertTrue(mix.tracks[1].eqEnabled)
                root.measure(View.MeasureSpec.makeMeasureSpec(1080, View.MeasureSpec.EXACTLY),
                    View.MeasureSpec.makeMeasureSpec(2400, View.MeasureSpec.EXACTLY))
                root.layout(0, 0, 1080, 2400)
                var ancestor = panel.parent
                while (ancestor != null && ancestor !is ScrollView) ancestor = ancestor.parent
                (ancestor as ScrollView).scrollTo(0, activity.resources.displayMetrics.density.times(250).toInt())
                capture("equalizer")
            }
        }
    }
}
