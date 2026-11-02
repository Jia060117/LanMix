package io.lanmix.receiver

import android.animation.ValueAnimator
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import java.util.Locale

internal data class MixerPalette(val ink: Int, val subtle: Int, val accent: Int, val dark: Boolean) {
    val quiet get() = if (dark) Color.rgb(37, 48, 64) else Color.rgb(240, 243, 249)
    val line get() = if (dark) Color.rgb(64, 78, 98) else Color.rgb(217, 224, 236)
}

internal fun Context.px(value: Float) = value * resources.displayMetrics.density
internal fun Context.dp(value: Int) = px(value.toFloat()).toInt()
internal fun decibels(value: Float) = String.format(Locale.US, "%+.1f dB", value)
internal fun animateUi() = ValueAnimator.areAnimatorsEnabled()
internal fun TextView.pulse() {
    if (!animateUi()) return
    animate().cancel()
    alpha = .65f
    animate().alpha(1f).setDuration(160).start()
}

internal fun Context.mixerText(value: String, size: Float, color: Int, bold: Boolean = false) = TextView(this).apply {
    text = value; textSize = size; setTextColor(color)
    if (bold) setTypeface(typeface, Typeface.BOLD)
    setPadding(0, dp(3), 0, dp(3))
}

internal class MixerChip(context: Context, label: String, private val palette: MixerPalette, action: () -> Unit) : TextView(context) {
    init {
        text = label; textSize = 12f; gravity = Gravity.CENTER
        setPadding(context.dp(14), context.dp(8), context.dp(14), context.dp(8))
        minHeight = context.dp(48); isFocusable = true
        maxLines = 1; maxEms = 12; ellipsize = android.text.TextUtils.TruncateAt.END
        layoutParams = LinearLayout.LayoutParams(-2, -2).apply { rightMargin = context.dp(6) }
        setOnClickListener { action(); pulse() }
        selected(false)
    }

    fun selected(value: Boolean) {
        if (isSelected == value && background != null) return
        isSelected = value
        setTextColor(if (value) palette.accent else palette.subtle)
        setTypeface(typeface, if (value) Typeface.BOLD else Typeface.NORMAL)
        background = RippleDrawable(ColorStateList.valueOf(palette.accent and 0x00ffffff or 0x18000000),
            GradientDrawable().apply {
                setColor(if (value) palette.accent and 0x00ffffff or (if (palette.dark) 0x28000000 else 0x14000000) else palette.quiet)
                cornerRadius = context.px(16f)
                setStroke(context.dp(1), if (value) palette.accent and 0x00ffffff or 0x80000000.toInt() else Color.TRANSPARENT)
            }, null)
    }

    override fun getAccessibilityClassName(): CharSequence = "android.widget.Button"
}
