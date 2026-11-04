package io.lanmix.receiver

import android.animation.ValueAnimator
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.os.Bundle
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.SeekBar
import android.widget.Switch
import android.widget.TextView
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.roundToInt

/** UI animations only interpolate drawing values; one preset applies one DSP update. */
internal class EqualizerView(
    context: Context, private val palette: MixerPalette,
    private val changed: (MixControl, Boolean) -> Unit, private val summaryChanged: (String) -> Unit
) : LinearLayout(context) {
    private var control = MixControl()
    private var binding = false
    private var editOrigin: MixControl? = null
    private val enabledSwitch = Switch(context)
    private var headroomOrigin: MixControl? = null
    private val presetChips = mutableMapOf<EqPreset, MixerChip>()
    private val curve = Curve(context)
    val faders = List(10) { index ->
        BandFader(context, index, palette, {
            val origin = control
            editOrigin = origin
            origin.bands[index] = it
            origin.eqEnabled = true
            updateSummary()
            changed(origin, false)
        }, { editOrigin?.let { changed(it, true) }; editOrigin = null }, { curve.invalidate() })
    }
    private val headroomLabel = context.mixerText("", 12f, palette.subtle)
    private val headroom = SeekBar(context)
    private val advanced = LinearLayout(context)

    init {
        orientation = VERTICAL
        enabledSwitch.apply {
            text = "均衡器"; textSize = 14f; setTextColor(palette.ink); minHeight = context.dp(48)
            thumbTintList = ColorStateList(arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()), intArrayOf(palette.accent, palette.subtle))
            trackTintList = ColorStateList(arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()),
                intArrayOf(palette.accent and 0x00ffffff or 0x66000000, palette.line))
            setOnCheckedChangeListener { _, enabled ->
                if (!binding) { val origin = control; origin.eqEnabled = enabled; updateSummary(); changed(origin, true) }
            }
        }
        addView(enabledSwitch)
        val presets = LinearLayout(context)
        EqPreset.entries.forEach { preset ->
            val chip = MixerChip(context, preset.label, palette) {
                val origin = control
                preset.applyTo(origin)
                render(true)
                changed(origin, true)
            }
            presetChips[preset] = chip; presets.addView(chip)
        }
        addView(HorizontalScrollView(context).apply {
            isHorizontalScrollBarEnabled = false
            addView(presets)
        }, LayoutParams(-1, -2))
        addView(context.mixerText("频段增益 · dB     低频 → 高频 · Hz", 11f, palette.subtle))
        val bank = LinearLayout(context).apply {
            orientation = VERTICAL
            minimumWidth = faders.maxOf { it.minimumBandWidth } * 10
        }
        bank.addView(curve, LayoutParams(-1, context.dp(68)))
        val row = LinearLayout(context)
        faders.forEach { row.addView(it, LayoutParams(0, context.dp(204) + context.dp(((resources.configuration.fontScale - 1).coerceAtLeast(0f) * 28).toInt()), 1f)) }
        bank.addView(row)
        addView(HorizontalScrollView(context).apply {
            isFillViewport = true; isHorizontalScrollBarEnabled = false
            contentDescription = "十段均衡器，小屏或大字体时可横向滚动"
            addView(bank)
        }, LayoutParams(-1, -2))
        val more = MixerChip(context, "高级 · 输出余量", palette) {
            advanced.visibility = if (advanced.visibility == GONE) VISIBLE else GONE
        }
        addView(more)
        advanced.orientation = VERTICAL; advanced.visibility = GONE
        advanced.addView(context.mixerText("提升频段时降低预放大，给混音留出余量。", 11f, palette.subtle))
        advanced.addView(headroomLabel)
        headroom.apply {
            max = 72; minimumHeight = context.dp(48); contentDescription = "均衡器预放大，单位分贝"
            progressTintList = ColorStateList.valueOf(palette.accent); thumbTintList = ColorStateList.valueOf(palette.accent)
            setOnTouchListener { _, event ->
                if (event.actionMasked == MotionEvent.ACTION_DOWN) headroomOrigin = control
                false
            }
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(bar: SeekBar, progress: Int, fromUser: Boolean) {
                    if (!fromUser || binding) return
                    val origin = headroomOrigin ?: control
                    if (origin !== control) return
                    headroomOrigin = origin
                    origin.preampDb = -24f + progress / 2f
                    origin.eqEnabled = true
                    headroomLabel.text = "预放大  ${decibels(control.preampDb)}"
                    updateSummary(); changed(origin, false)
                }
                override fun onStartTrackingTouch(bar: SeekBar) { headroomLabel.pulse() }
                override fun onStopTrackingTouch(bar: SeekBar) {
                    headroomOrigin?.let { changed(it, true) }; headroomOrigin = null
                    bar.progress = ((control.preampDb + 24f) * 2).roundToInt()
                    headroomLabel.text = "预放大  ${decibels(control.preampDb)}"
                }
            })
        }
        advanced.addView(headroom)
        addView(advanced)
    }

    fun bind(value: MixControl, target: String) {
        editOrigin = null
        faders.forEach { it.cancelInteraction() }
        control = value
        setTargetName(target)
        advanced.visibility = GONE
        render(false)
    }

    fun setTargetName(target: String) { enabledSwitch.text = "均衡器 · $target" }

    private fun render(animate: Boolean) {
        faders.forEachIndexed { index, fader -> fader.setValue(control.bands[index], animate) }
        headroom.progress = ((control.preampDb + 24f) * 2).roundToInt()
        headroomLabel.text = "预放大  ${decibels(control.preampDb)}"
        updateSummary()
        curve.invalidate()
    }

    private fun updateSummary() {
        binding = true
        enabledSwitch.isChecked = control.eqEnabled
        binding = false
        val preset = EqPreset.matching(control)
        presetChips.forEach { (candidate, chip) -> chip.selected(control.eqEnabled && candidate == preset) }
        curve.alpha = if (control.eqEnabled) 1f else .4f
        summaryChanged(if (control.eqEnabled) preset?.label ?: "自定义" else "关闭")
    }

    private inner class Curve(context: Context) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val path = Path()
        override fun onDraw(canvas: Canvas) {
            super.onDraw(canvas)
            paint.strokeWidth = context.px(1f); paint.color = palette.line
            for (y in listOf(.12f, .5f, .88f)) canvas.drawLine(0f, height * y, width.toFloat(), height * y, paint)
            val xs = faders.mapIndexed { i, _ -> width * (i + .5f) / 10 }
            val ys = faders.map { height * (.5f - it.displayValue / 12f * .38f) }
            path.rewind(); path.moveTo(xs.first(), ys.first())
            for (i in 1..9) {
                val mid = (xs[i - 1] + xs[i]) / 2
                path.cubicTo(mid, ys[i - 1], mid, ys[i], xs[i], ys[i])
            }
            paint.style = Paint.Style.STROKE; paint.strokeWidth = context.px(2f); paint.color = palette.accent
            canvas.drawPath(path, paint)
            paint.style = Paint.Style.FILL
            for (i in 0..9) canvas.drawCircle(xs[i], ys[i], context.px(2.5f), paint)
        }
    }
}

/** A vertical native View: keyboard and TalkBack expose the actual dB range. */
internal class BandFader(
    context: Context, private val band: Int, private val palette: MixerPalette,
    private val change: (Float) -> Unit, private val commit: () -> Unit, private val redraw: () -> Unit
) : View(context) {
    companion object {
        val labels = listOf("31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k")
        val frequencies = listOf("31.25", "62.5", "125", "250", "500", "1000", "2000", "4000", "8000", "16000")
    }
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val scaledText = 10f * resources.displayMetrics.scaledDensity
    val minimumBandWidth get() = max(context.dp(28), (paint.apply { textSize = scaledText }.measureText("+0.5") + context.px(6f)).roundToInt())
    private var value = 0f
    var displayValue = 0f; private set
    private var movement: ValueAnimator? = null
    private var haloAnimation: ValueAnimator? = null
    private var halo = 0f
    private var downX = 0f
    private var downY = 0f
    private var dragging = false
    private var horizontalSwipe = false
    private val slop = ViewConfiguration.get(context).scaledTouchSlop
    private val trackTop get() = context.px(32f) + (scaledText - context.px(10f)).coerceAtLeast(0f)
    private val trackBottom get() = height - context.px(28f) - (scaledText - context.px(10f)).coerceAtLeast(0f)

    init {
        isFocusable = true; isClickable = true
        contentDescription = "${frequencies[band]} 赫兹"
    }

    fun setValue(next: Float, animate: Boolean) {
        value = next.coerceIn(-12f, 12f)
        movement?.cancel()
        if (animate && animateUi()) {
            movement = ValueAnimator.ofFloat(displayValue, value).apply {
                duration = 180
                addUpdateListener { displayValue = it.animatedValue as Float; invalidate(); redraw() }
                start()
            }
        } else { displayValue = value; invalidate(); redraw() }
    }

    fun cancelInteraction() {
        dragging = false; horizontalSwipe = true; feedback(false)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val x = width / 2f
        val y = trackTop + (12 - displayValue) / 24f * (trackBottom - trackTop)
        paint.textAlign = Paint.Align.CENTER; paint.textSize = scaledText; paint.color = if (isPressed || isFocused) palette.accent else palette.subtle
        val number = if (value % 1f == 0f) "%+.0f" else "%+.1f"
        canvas.drawText(String.format(java.util.Locale.US, number, value), x, context.px(15f) + scaledText / 3, paint)
        canvas.drawText(labels[band], x, height - context.px(7f), paint)
        paint.strokeCap = Paint.Cap.ROUND; paint.strokeWidth = context.px(3f); paint.color = palette.line
        canvas.drawLine(x, trackTop, x, trackBottom, paint)
        paint.color = palette.accent; canvas.drawLine(x, (trackTop + trackBottom) / 2f, x, y, paint)
        paint.strokeWidth = context.px(1f); paint.color = palette.subtle
        canvas.drawLine(x - context.px(3f), (trackTop + trackBottom) / 2f, x + context.px(3f), (trackTop + trackBottom) / 2f, paint)
        paint.color = palette.accent and 0x00ffffff or ((halo * 36).toInt() shl 24)
        canvas.drawCircle(x, y, context.px(11f), paint)
        paint.color = palette.accent
        canvas.drawCircle(x, y, context.px(4.5f + halo), paint)
    }

    private fun feedback(pressed: Boolean) {
        isPressed = pressed
        haloAnimation?.cancel()
        val target = if (pressed) 1f else 0f
        if (!animateUi()) { halo = target; invalidate(); return }
        haloAnimation = ValueAnimator.ofFloat(halo, target).apply {
            duration = 120; addUpdateListener { halo = it.animatedValue as Float; invalidate() }; start()
        }
    }

    private fun input(next: Float) {
        val rounded = (next.coerceIn(-12f, 12f) * 2).roundToInt() / 2f
        if (rounded == value) return
        setValue(rounded, false)
        change(value)
        sendAccessibilityEvent(AccessibilityEvent.TYPE_VIEW_SELECTED)
    }

    private fun inputAt(y: Float) = input(12f - ((y - trackTop) / (trackBottom - trackTop).coerceAtLeast(1f)) * 24f)

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (!isEnabled) return false
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x; downY = event.y; dragging = false; horizontalSwipe = false; feedback(true)
                // Delay value changes so a horizontal bank swipe does not alter a band.
                parent.requestDisallowInterceptTouchEvent(true)
                return true
            }
            MotionEvent.ACTION_MOVE -> {
                if (horizontalSwipe) return true
                if (!dragging && abs(event.x - downX) > slop && abs(event.x - downX) > abs(event.y - downY)) {
                    horizontalSwipe = true
                    feedback(false); parent.requestDisallowInterceptTouchEvent(false); return true
                }
                if (abs(event.y - downY) > slop) dragging = true
                if (dragging) inputAt(event.y)
                return true
            }
            MotionEvent.ACTION_UP -> {
                if (isPressed) { inputAt(event.y); performClick(); commit() }
                feedback(false); parent.requestDisallowInterceptTouchEvent(false); return true
            }
            MotionEvent.ACTION_CANCEL -> {
                if (dragging) commit()
                feedback(false); parent.requestDisallowInterceptTouchEvent(false); return true
            }
        }
        return super.onTouchEvent(event)
    }

    override fun performClick(): Boolean { super.performClick(); return true }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean = when (keyCode) {
        KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_PLUS -> { input(value + .5f); commit(); true }
        KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_MINUS -> { input(value - .5f); commit(); true }
        else -> super.onKeyDown(keyCode, event)
    }

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        info.className = "android.widget.SeekBar"
        info.rangeInfo = AccessibilityNodeInfo.RangeInfo.obtain(AccessibilityNodeInfo.RangeInfo.RANGE_TYPE_FLOAT, -12f, 12f, value)
        info.text = decibels(value)
        info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS)
        info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_FORWARD)
        info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SCROLL_BACKWARD)
    }

    override fun performAccessibilityAction(action: Int, arguments: Bundle?): Boolean {
        val next = when (action) {
            AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_PROGRESS.id -> arguments?.getFloat(AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE) ?: return false
            AccessibilityNodeInfo.ACTION_SCROLL_FORWARD -> value + .5f
            AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD -> value - .5f
            else -> return super.performAccessibilityAction(action, arguments)
        }
        if (!next.isFinite()) return false
        input(next); commit(); return true
    }

    override fun onDetachedFromWindow() {
        movement?.cancel(); haloAnimation?.cancel()
        super.onDetachedFromWindow()
    }
}
