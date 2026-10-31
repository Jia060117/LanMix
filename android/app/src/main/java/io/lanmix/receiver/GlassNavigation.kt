package io.lanmix.receiver

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.*
import android.graphics.drawable.Drawable
import android.os.Build
import android.view.Gravity
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.LinearLayout
import android.widget.TextView

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
    private val tabs = mutableListOf<TextView>()
    private val here = IntArray(2)
    private val there = IntArray(2)
    private var surfaceGradient: LinearGradient? = null
    private var rimGradient: LinearGradient? = null
    private var blur: RenderEffect? = null
    private fun dp(n: Float) = n * resources.displayMetrics.density

    init {
        orientation = HORIZONTAL; gravity = Gravity.CENTER; setWillNotDraw(false)
        setPadding(dp(5f).toInt(), dp(5f).toInt(), dp(5f).toInt(), dp(5f).toInt())
        elevation = dp(16f)
        outlineProvider = object : android.view.ViewOutlineProvider() {
            override fun getOutline(view: View, outline: Outline) { outline.setRoundRect(0, 0, width, height, height / 2f) }
        }
        listOf("混音", "设备", "设置").forEachIndexed { index, label ->
            val tab = TextView(context).apply {
                text = label; textSize = 11f; gravity = Gravity.CENTER
                setPadding(0, dp(3f).toInt(), 0, 0)
                compoundDrawablePadding = dp(3f).toInt()
                contentDescription = label
                isFocusable = true
                setOnClickListener { onSelected(index) }
                accessibilityDelegate = object : View.AccessibilityDelegate() {
                    override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                        super.onInitializeAccessibilityNodeInfo(host, info)
                        info.className = "android.widget.Button"; info.isSelected = index == selected
                    }
                }
            }
            tabs.add(tab); addView(tab, LayoutParams(0, -1, 1f))
        }
        updateTabs()
    }
    fun select(index: Int, animate: Boolean = true) {
        selected = index; animator?.cancel()
        if (animate) {
            animator = ValueAnimator.ofFloat(indicator, index.toFloat()).apply {
                duration = 220; addUpdateListener { indicator = it.animatedValue as Float; invalidate() }; start()
            }
        } else { indicator = index.toFloat(); invalidate() }
        updateTabs()
    }
    private fun updateTabs() {
        tabs.forEachIndexed { index, tab ->
            val color = if (index == selected) accent else if (dark) Color.rgb(161, 171, 190) else Color.rgb(108, 115, 132)
            tab.setTextColor(color); tab.isSelected = index == selected
            tab.setCompoundDrawables(null, NavigationIcon(index, color, dp(22f).toInt()), null, null)
        }
    }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        surfaceGradient = LinearGradient(0f, 0f, w.toFloat(), h.toFloat(),
            if (dark) intArrayOf(0xCF38414F.toInt(), 0xB81D2532.toInt(), 0xCF2A3341.toInt())
            else intArrayOf(0xDFFFFFFF.toInt(), 0xCDEDF2FA.toInt(), 0xDFFFFFFF.toInt()), null, Shader.TileMode.CLAMP)
        rimGradient = LinearGradient(0f, 0f, w.toFloat(), h.toFloat(),
            if (dark) 0x77FFFFFF else 0xFFFFFFFF.toInt(), if (dark) 0x15466B98 else 0x558A9DBF, Shader.TileMode.CLAMP)
        if (Build.VERSION.SDK_INT >= 31) {
            val base = RenderEffect.createBlurEffect(dp(14f), dp(14f), Shader.TileMode.CLAMP)
            // Shader support/driver failures fall back to ordinary backdrop blur.
            blur = if (Build.VERSION.SDK_INT >= 33) runCatching { lens(w, h, base) }.getOrDefault(base) else base
        }
    }
    @android.annotation.TargetApi(33)
    private fun lens(w: Int, h: Int, base: RenderEffect): RenderEffect {
        val shader = RuntimeShader("""
            uniform shader backdrop;
            uniform float2 extent;
            uniform float depth;
            half4 main(float2 p) {
                float2 center = extent * 0.5;
                float2 delta = p - center;
                float2 q = abs(delta) - float2(center.x - center.y, 0.0);
                float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - center.y;
                float edge = 1.0 - smoothstep(0.0, depth * 2.5, -d);
                float2 n = normalize(float2(sign(delta.x) * max(q.x, 0.0), delta.y) + float2(0.0001));
                return backdrop.eval(p - n * edge * edge * depth);
            }
        """.trimIndent())
        shader.setFloatUniform("extent", w.toFloat(), h.toFloat())
        shader.setFloatUniform("depth", dp(4.5f))
        return RenderEffect.createChainEffect(RenderEffect.createRuntimeShaderEffect(shader, "backdrop"), base)
    }
    override fun onDraw(canvas: Canvas) {
        val radius = height / 2f
        bounds.set(0f, 0f, width.toFloat(), height.toFloat())
        clip.reset(); clip.addRoundRect(bounds, radius, radius, Path.Direction.CW)
        val saved = canvas.save(); canvas.clipPath(clip)
        if (Build.VERSION.SDK_INT >= 31 && canvas.isHardwareAccelerated && node != null && backdrop.width > 0) {
            getLocationInWindow(here); backdrop.getLocationInWindow(there)
            node.setPosition(0, 0, width, height)
            val recording = node.beginRecording(width, height)
            recording.translate((there[0] - here[0]).toFloat(), (there[1] - here[1]).toFloat())
            backdrop.draw(recording)
            node.endRecording()
            node.setRenderEffect(blur)
            canvas.drawRenderNode(node)
        }
        paint.shader = surfaceGradient
        canvas.drawRect(bounds, paint); paint.shader = null
        val cell = (width - paddingLeft - paddingRight) / 3f
        paint.color = if (dark) 0x305EA7EF else 0x1D3869DB
        val left = paddingLeft + cell * indicator
        canvas.drawRoundRect(left, dp(5f), left + cell, height - dp(5f), radius, radius, paint)
        canvas.restoreToCount(saved)
        paint.style = Paint.Style.STROKE; paint.strokeWidth = dp(1f)
        paint.shader = rimGradient
        bounds.inset(dp(.6f), dp(.6f)); canvas.drawRoundRect(bounds, radius, radius, paint)
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
