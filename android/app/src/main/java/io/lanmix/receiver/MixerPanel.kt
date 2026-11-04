package io.lanmix.receiver

import android.content.Context
import android.content.res.ColorStateList
import android.view.Gravity
import android.view.MotionEvent
import android.widget.CheckBox
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.SeekBar

internal data class MixerSource(val key: String, val slot: Int, val name: String, val online: Boolean)

/** One editor with an explicit target. Neither switching targets nor polling writes audio settings. */
internal class MixerPanel(
    context: Context, private val mix: MixState, private val palette: MixerPalette,
    private val apply: (Int?) -> Unit, private val save: () -> Unit, connect: () -> Unit
) : LinearLayout(context) {
    private var sources = emptyList<MixerSource>()
    private var selectedKey: String? = null
    private var restoredKey: String? = null
    private var bound = mix.master
    private var binding = false
    private var volumeOrigin: MixControl? = null
    private val selectors = LinearLayout(context)
    private val chips = mutableMapOf<String?, MixerChip>()
    private val description = context.mixerText("所有音源一起调整", 12f, palette.subtle)
    private val sourceStatus = context.mixerText("还没有音源，在「设备」中连接电脑。", 12f, palette.subtle)
    private val volumeValue = context.mixerText("", 17f, palette.accent, true)
    private val volume = SeekBar(context)
    private val flags = LinearLayout(context)
    private val mute = CheckBox(context)
    private val solo = CheckBox(context)
    private val soundButton = MixerChip(context, "音效 · 关闭  ›", palette) { setExpanded(!equalizerExpanded, true) }
    private val equalizer = EqualizerView(context, palette, ::changed) { summary ->
        soundButton.text = "音效 · $summary  ${if (equalizerExpanded) "⌃" else "⌄"}"
    }
    private val delayedSave = Runnable { save() }
    val selectedSourceKey get() = selectedKey
    var equalizerExpanded = false; private set

    init {
        orientation = VERTICAL
        addView(HorizontalScrollView(context).apply {
            isHorizontalScrollBarEnabled = false; addView(selectors)
        })
        addView(description, LayoutParams(-1, -2).apply { topMargin = context.dp(8); bottomMargin = context.dp(12) })
        val volumeHeader = LinearLayout(context).apply { gravity = Gravity.CENTER_VERTICAL }
        volumeHeader.addView(context.mixerText("音量", 14f, palette.ink, true), LayoutParams(0, -2, 1f))
        volumeHeader.addView(volumeValue)
        addView(volumeHeader)
        volume.apply {
            max = 144; minimumHeight = context.dp(48); contentDescription = "当前调音对象的音量，单位分贝"
            progressTintList = ColorStateList.valueOf(palette.accent); thumbTintList = ColorStateList.valueOf(palette.accent)
            setOnTouchListener { _, event ->
                if (event.actionMasked == MotionEvent.ACTION_DOWN) volumeOrigin = bound
                false
            }
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(bar: SeekBar, progress: Int, fromUser: Boolean) {
                    val origin = volumeOrigin ?: bound
                    if (!fromUser || binding || origin !== bound || !isCurrentBinding()) return
                    origin.gainDb = -60f + progress / 2f
                    volumeValue.text = decibels(origin.gainDb)
                    changed(origin, false)
                }
                override fun onStartTrackingTouch(bar: SeekBar) { volumeValue.pulse() }
                override fun onStopTrackingTouch(bar: SeekBar) {
                    changed(volumeOrigin ?: bound, true); volumeOrigin = null
                    bar.progress = ((bound.gainDb + 60f) * 2).toInt()
                    volumeValue.text = decibels(bound.gainDb)
                }
            })
        }
        addView(volume, LayoutParams(-1, -2))
        fun flag(view: CheckBox, label: String, action: (Boolean) -> Unit) {
            view.text = label; view.minHeight = context.dp(48); view.setTextColor(palette.ink)
            view.buttonTintList = ColorStateList.valueOf(palette.accent)
            view.setOnCheckedChangeListener { _, value -> if (!binding && isCurrentBinding()) { action(value); changed(bound, true) } }
            flags.addView(view, LayoutParams(0, -2, 1f))
        }
        flag(mute, "静音") { bound.mute = it }
        flag(solo, "独听") { bound.solo = it }
        solo.contentDescription = "只播放勾选独听的音源，可同时勾选多台"
        addView(flags)
        addView(soundButton, LayoutParams(-1, -2).apply { topMargin = context.dp(6) })
        addView(equalizer, LayoutParams(-1, -2).apply { topMargin = context.dp(10) })
        equalizer.visibility = GONE
        addView(sourceStatus, LayoutParams(-1, -2).apply { topMargin = context.dp(12) })
        addView(MixerChip(context, "＋ 连接设备", palette, connect), LayoutParams(-1, -2).apply { topMargin = context.dp(6) })
        rebuildSelectors(); bind()
    }

    fun restoreSelection(key: String?, expanded: Boolean) {
        restoredKey = key
        setExpanded(expanded, false)
    }

    fun setSources(next: List<MixerSource>) {
        val valid = next.filter { it.slot in 0..15 && mix.trackIdentity(it.slot) == it.key }.distinctBy { it.key }
        val old = sources
        sources = valid
        restoredKey?.let { key -> if (valid.isNotEmpty()) { selectedKey = valid.find { it.key == key }?.key; restoredKey = null } }
        if (selectedKey != null && valid.none { it.key == selectedKey }) selectedKey = null
        if (old != valid) rebuildSelectors()
        if (bound !== currentControl()) bind()
        updateDescription()
    }

    fun clearSources() {
        sources = emptyList(); selectedKey = null; restoredKey = null
        rebuildSelectors(); bind()
    }

    fun markOffline() { setSources(sources.map { it.copy(online = false) }) }

    private fun currentSource() = sources.find { it.key == selectedKey }
    private fun currentControl() = currentSource()?.let { mix.tracks[it.slot] } ?: mix.master
    private fun isCurrentBinding(): Boolean {
        val source = currentSource()
        return bound === currentControl() && (source == null || mix.trackIdentity(source.slot) == source.key)
    }

    private fun changed(origin: MixControl, commit: Boolean) {
        if (binding || origin !== bound) return
        // A recycled slot cannot send a stale editor's settings to another computer.
        if (!isCurrentBinding()) { clearSources(); return }
        apply(currentSource()?.slot)
        removeCallbacks(delayedSave)
        if (commit) save() else postDelayed(delayedSave, 180)
    }

    private fun rebuildSelectors() {
        selectors.removeAllViews(); chips.clear()
        fun add(key: String?, label: String) {
            val chip = MixerChip(context, label, palette) {
                if (selectedKey != key) {
                    selectedKey = key; restoredKey = null
                    bind(); chips.forEach { (candidate, view) -> view.selected(candidate == key) }
                    volumeValue.pulse()
                }
            }
            chip.contentDescription = if (key == null) "调节整体声音" else "调节 $label，$key"
            chip.selected(selectedKey == key)
            chips[key] = chip; selectors.addView(chip)
        }
        add(null, "整体")
        sources.forEach { add(it.key, it.name + if (it.online) "" else " · 离线") }
    }

    private fun bind() {
        binding = true
        bound = currentControl()
        volume.progress = ((bound.gainDb + 60f) * 2).toInt()
        volumeValue.text = decibels(bound.gainDb)
        flags.visibility = if (currentSource() == null) GONE else VISIBLE
        mute.isChecked = bound.mute; solo.isChecked = bound.solo
        equalizer.bind(bound, currentSource()?.name ?: "整体")
        binding = false
        updateDescription()
    }

    private fun updateDescription() {
        equalizer.setTargetName(currentSource()?.name ?: "整体")
        description.text = currentSource()?.let { "只调整 ${it.name}${if (it.online) "" else " · 离线设置保留"}" } ?: "所有音源一起调整"
        sourceStatus.text = when {
            sources.isEmpty() -> "还没有音源，在「设备」中连接电脑。"
            sources.none { it.online } -> "音源已离线 · 调音设置已保留"
            else -> "${sources.count { it.online }} 个音源在线 · 轻点上方设备，单独调音"
        }
    }

    private fun setExpanded(value: Boolean, animate: Boolean) {
        equalizerExpanded = value
        equalizer.visibility = if (value) VISIBLE else GONE
        soundButton.text = soundButton.text.toString().dropLast(1) + if (value) "⌃" else "⌄"
        if (value && animate && animateUi()) {
            equalizer.alpha = 0f; equalizer.translationY = -context.px(6f)
            equalizer.animate().alpha(1f).translationY(0f).setDuration(180).start()
        } else { equalizer.animate().cancel(); equalizer.alpha = 1f; equalizer.translationY = 0f }
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(delayedSave); save()
        super.onDetachedFromWindow()
    }
}
