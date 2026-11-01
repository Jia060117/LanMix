package io.lanmix.receiver

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.*
import android.graphics.drawable.Drawable
import android.os.Build
import android.view.Gravity
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.view.animation.PathInterpolator

/** A small native backdrop: only records the bar-sized slice, never the audio/UI thread's PCM. */
class GlassNavigation(context: Context, private val backdrop: View, private val dark: Boolean,
    private val accent: Int, private val onSelected: (Int) -> Unit) : LinearLayout(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val bounds = RectF()
    private val clip = Path()
    private var selected = 0
    private var indicator = 0f
    private var animator: ValueAnimator? = null
    private val node = if (Build.VERSION.SDK_INT >= 31) RenderNode("LanMix navigation backdrop") else null
    private val tabs = mutableListOf<LinearLayout>()
    private val here = IntArray(2)
    private val there = IntArray(2)
    private var surfaceGradient: LinearGradient? = null
    private var rimGradient: LinearGradient? = null
    private var selectionGradient: LinearGradient? = null
    private var blur: RenderEffect? = null
    internal var refractionAvailable = false
        private set
    private val capturePadding = dp(16f).toInt()
    private fun dp(n: Float) = n * resources.displayMetrics.density

    init {
        orientation = HORIZONTAL; gravity = Gravity.CENTER; setWillNotDraw(false)
        setPadding(dp(5f).toInt(), dp(5f).toInt(), dp(5f).toInt(), dp(5f).toInt())
        elevation = dp(9f)
        outlineProvider = object : android.view.ViewOutlineProvider() {
            override fun getOutline(view: View, outline: Outline) { outline.setRoundRect(0, 0, width, height, height / 2f) }
        }
        listOf("混音", "设备", "设置").forEachIndexed { index, label ->
            val tab = LinearLayout(context).apply {
                orientation = VERTICAL; gravity = Gravity.CENTER
                contentDescription = label
                isFocusable = true
                background = RippleDrawable(ColorStateList.valueOf(if (dark) 0x20FFFFFF else 0x16000000), null,
                    GradientDrawable().apply { setColor(Color.WHITE); cornerRadius = dp(28f) })
                setOnClickListener { onSelected(index) }
                accessibilityDelegate = object : View.AccessibilityDelegate() {
                    override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                        super.onInitializeAccessibilityNodeInfo(host, info)
                        info.className = "android.widget.Button"; info.isSelected = index == selected
                    }
                }
            }
            tab.addView(ImageView(context).apply {
                importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
                isFocusable = false
            }, LayoutParams(dp(22f).toInt(), dp(22f).toInt()))
            tab.addView(TextView(context).apply {
                text = label; textSize = 11f; gravity = Gravity.CENTER
                includeFontPadding = false; setSingleLine()
                typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
                importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
            }, LayoutParams(-2, -2).apply { topMargin = dp(3f).toInt() })
            tabs.add(tab); addView(tab, LayoutParams(0, -1, 1f))
        }
        updateTabs()
    }
    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val width = minOf(MeasureSpec.getSize(widthMeasureSpec), dp(336f).toInt())
        super.onMeasure(MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY), heightMeasureSpec)
    }
    fun select(index: Int, animate: Boolean = true) {
        selected = index; animator?.cancel()
        if (animate) {
            animator = ValueAnimator.ofFloat(indicator, index.toFloat()).apply {
                duration = 280; interpolator = PathInterpolator(.22f, 1f, .36f, 1f)
                addUpdateListener { indicator = it.animatedValue as Float; invalidate() }; start()
            }
        } else { indicator = index.toFloat(); invalidate() }
        updateTabs()
    }
    private fun updateTabs() {
        tabs.forEachIndexed { index, tab ->
            val color = if (index == selected) accent else if (dark) Color.rgb(190, 200, 215) else Color.rgb(76, 88, 108)
            (tab.getChildAt(1) as TextView).setTextColor(color); tab.isSelected = index == selected
            (tab.getChildAt(0) as ImageView).setImageDrawable(NavigationIcon(index, color, dp(22f).toInt()))
        }
    }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        surfaceGradient = LinearGradient(0f, 0f, w.toFloat(), h.toFloat(),
            if (dark) intArrayOf(0x303D4C60, 0x18121B29, 0x38232F42)
            else intArrayOf(0x66FFFFFF, 0x30EDF3FF, 0x55FFFFFF), null, Shader.TileMode.CLAMP)
        rimGradient = LinearGradient(0f, 0f, w.toFloat(), h.toFloat(),
            intArrayOf(0xAAFFFFFF.toInt(), 0x18FFFFFF, 0x09FFFFFF, 0x60CBDFFF),
            floatArrayOf(0f, .38f, .66f, 1f), Shader.TileMode.CLAMP)
        selectionGradient = LinearGradient(0f, dp(5f), 0f, h - dp(5f),
            if (dark) 0x28D9EBFF else 0x88FFFFFF.toInt(), if (dark) 0x0AA9CCFF else 0x183E67DE, Shader.TileMode.CLAMP)
        if (Build.VERSION.SDK_INT >= 31) {
            val base = RenderEffect.createBlurEffect(dp(1.5f), dp(1.5f), Shader.TileMode.CLAMP)
            // Shader support/driver failures fall back to ordinary backdrop blur.
            refractionAvailable = false
            blur = if (Build.VERSION.SDK_INT >= 33) runCatching { lens(w, h, base) }
                .onSuccess { refractionAvailable = true }.getOrDefault(base) else base
        }
    }
    @android.annotation.TargetApi(33)
    private fun lens(w: Int, h: Int, base: RenderEffect): RenderEffect {
        val shader = RuntimeShader("""
            uniform shader backdrop;
            uniform float2 extent;
            uniform float inset;
            uniform float depth;
            half4 main(float2 p) {
                float2 center = extent * 0.5;
                float2 delta = p - inset - center;
                float2 axis = float2(clamp(delta.x, -center.x + center.y, center.x - center.y), 0.0);
                float2 radial = delta - axis;
                float d = length(radial) - center.y;
                float2 n = radial / max(length(radial), 0.001);
                float edge = 1.0 - smoothstep(0.0, depth * 2.0, -d);
                float2 samplePoint = p - n * edge * edge * depth;
                half4 c = backdrop.eval(samplePoint);
                // Subpixel dispersion and a narrow light-catching bevel, not a frosted sheet.
                float dispersion = edge * edge * 0.45;
                c.r = backdrop.eval(samplePoint + n * dispersion).r;
                c.b = backdrop.eval(samplePoint - n * dispersion).b;
                float rim = 1.0 - smoothstep(0.0, 1.8, abs(d + 1.0));
                float light = 0.35 + 0.65 * abs(dot(n, normalize(float2(-0.65, -1.0))));
                c.rgb = mix(c.rgb, half3(1.0), half(rim * light * 0.32));
                return c;
            }
        """.trimIndent())
        shader.setFloatUniform("extent", w.toFloat(), h.toFloat())
        shader.setFloatUniform("inset", capturePadding.toFloat())
        shader.setFloatUniform("depth", dp(7f))
        return RenderEffect.createChainEffect(RenderEffect.createRuntimeShaderEffect(shader, "backdrop"), base)
    }
    override fun onDraw(canvas: Canvas) {
        val radius = height / 2f
        bounds.set(0f, 0f, width.toFloat(), height.toFloat())
        clip.reset(); clip.addRoundRect(bounds, radius, radius, Path.Direction.CW)
        val saved = canvas.save(); canvas.clipPath(clip)
        if (Build.VERSION.SDK_INT >= 31 && canvas.isHardwareAccelerated && node != null && backdrop.width > 0) {
            getLocationInWindow(here); backdrop.getLocationInWindow(there)
            // Include pixels beyond the capsule so the lens/blur never samples a clipped edge.
            node.setPosition(-capturePadding, -capturePadding, width + capturePadding, height + capturePadding)
            val recording = node.beginRecording(width + capturePadding * 2, height + capturePadding * 2)
            recording.translate((there[0] - here[0] + capturePadding).toFloat(), (there[1] - here[1] + capturePadding).toFloat())
            backdrop.draw(recording)
            node.endRecording()
            node.setRenderEffect(blur)
            canvas.drawRenderNode(node)
        } else {
            // Readable translucent fallback on older systems and software canvases.
            paint.color = if (dark) 0xE51D2735.toInt() else 0xE5F5F8FF.toInt()
            canvas.drawRect(bounds, paint)
        }
        paint.alpha = 255; paint.shader = surfaceGradient
        canvas.drawRect(bounds, paint); paint.shader = null
        val cell = (width - paddingLeft - paddingRight) / 3f
        paint.shader = selectionGradient
        val left = paddingLeft + cell * indicator
        canvas.drawRoundRect(left, dp(5f), left + cell, height - dp(5f), radius, radius, paint)
        paint.shader = null
        paint.style = Paint.Style.STROKE; paint.strokeWidth = dp(.65f)
        paint.color = if (dark) 0x26FFFFFF else 0xA0FFFFFF.toInt()
        canvas.drawRoundRect(left + dp(.5f), dp(5.5f), left + cell - dp(.5f), height - dp(5.5f), radius, radius, paint)
        paint.style = Paint.Style.FILL
        canvas.restoreToCount(saved)
        paint.style = Paint.Style.STROKE; paint.strokeWidth = dp(1f); paint.alpha = 255
        paint.shader = rimGradient
        bounds.inset(dp(.6f), dp(.6f)); canvas.drawRoundRect(bounds, radius, radius, paint)
        bounds.inset(dp(1f), dp(1f)); paint.alpha = 55
        canvas.drawRoundRect(bounds, radius, radius, paint)
        paint.alpha = 255
        paint.shader = null; paint.style = Paint.Style.FILL
        super.onDraw(canvas)
    }
    override fun onDetachedFromWindow() {
        animator?.cancel()
        if (Build.VERSION.SDK_INT >= 31) node?.discardDisplayList()
        super.onDetachedFromWindow()
    }
}

private class NavigationIcon(private val kind: Int, private val color: Int, size: Int) : Drawable() {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    init { setBounds(0, 0, size, size) }
    override fun draw(canvas: Canvas) {
        val save = canvas.save()
        canvas.translate(bounds.left.toFloat(), bounds.top.toFloat()); canvas.scale(bounds.width() / 24f, bounds.height() / 24f)
        paint.color = color; paint.style = Paint.Style.STROKE; paint.strokeWidth = 1.8f; paint.strokeCap = Paint.Cap.ROUND
        when (kind) {
            0 -> { for ((x, y) in listOf(5f to 8f, 12f to 15f, 19f to 10f)) {
                canvas.drawLine(x, 3f, x, y - 3f, paint); canvas.drawLine(x, y + 3f, x, 21f, paint)
                canvas.drawCircle(x, y, 2.8f, paint)
            } }
            1 -> { canvas.drawRoundRect(2f, 3f, 16f, 14f, 2f, 2f, paint)
                canvas.drawLine(9f, 15f, 9f, 19f, paint); canvas.drawLine(5f, 20f, 12f, 20f, paint)
                canvas.drawRoundRect(16f, 10f, 22f, 21f, 1.5f, 1.5f, paint) }
            else -> { canvas.drawCircle(12f, 12f, 7f, paint); canvas.drawCircle(12f, 12f, 2.5f, paint)
                repeat(8) { canvas.save(); canvas.rotate(it * 45f, 12f, 12f); canvas.drawLine(12f, 2f, 12f, 5f, paint); canvas.restore() } }
        }
        canvas.restoreToCount(save)
    }
    override fun setAlpha(alpha: Int) { paint.alpha = alpha }
    override fun setColorFilter(filter: ColorFilter?) { paint.colorFilter = filter }
    @Deprecated("Deprecated in Java") override fun getOpacity() = PixelFormat.TRANSLUCENT
}
