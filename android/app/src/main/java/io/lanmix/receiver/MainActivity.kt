package io.lanmix.receiver

import android.Manifest
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.text.InputType
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.*
import org.json.JSONObject
import java.net.NetworkInterface
import java.util.Locale

/** Activity-only receiver: onStop always closes audio and UDP. No microphone. */
class MainActivity : Activity() {
    private val background = Color.rgb(16, 21, 29)
    private val surface = Color.rgb(25, 34, 46)
    private val ink = Color.rgb(234, 241, 250)
    private val subtle = Color.rgb(157, 175, 197)
    private val accent = Color.rgb(104, 224, 203)
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var manager: AudioManager
    private lateinit var engine: PlaybackEngine
    private lateinit var mix: MixState
    private lateinit var root: LinearLayout
    private lateinit var status: TextView
    private lateinit var bufferHealth: TextView
    private lateinit var actual: TextView
    private lateinit var capabilities: TextView
    private lateinit var networkStats: TextView
    private lateinit var eventHistory: TextView
    private lateinit var masterMeter: TextView
    private lateinit var deviceSpinner: Spinner
    private lateinit var rateSpinner: Spinner
    private lateinit var port: EditText
    private lateinit var buffer: EditText
    private lateinit var startButton: Button
    private lateinit var sources: LinearLayout
    private lateinit var emptySources: TextView
    private var devices = emptyList<AudioDeviceInfo>()
    private var selectedId: Int? = null
    private var rateOptions = listOf(0)
    private var selectedRate = 0
    private var playbackMode = PlaybackMode.STABLE
    private var updating = false
    private var requested = false
    private var running = false
    private var visible = false
    private val trackViews = mutableMapOf<Int, Pair<TextView, TextView>>()
    private val preferences by lazy { getSharedPreferences("lanmix", MODE_PRIVATE) }
    private val diagnosticStart = SystemClock.elapsedRealtime()
    private val history = DiagnosticHistory()
    private var latestNativeDiagnostics = "{}"
    private var latestNativeAt = 0L
    private var markedDiagnostic = "尚未标记闪断"
    private data class SourceDiagnostic(val session: Long, val lost: Long, val late: Long,
        val overflow: Long, val underruns: Long, val resets: Long, val primed: Boolean,
        val gapFilled: Long)
    private val sourceDiagnostics = mutableMapOf<String, SourceDiagnostic>()
    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) = updateDevices()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) = updateDevices()
    }
    private val poll = object : Runnable {
        override fun run() {
            if (!visible) return
            if (NativeBridge.loadError == null && running) {
                runCatching {
                    latestNativeDiagnostics = NativeBridge.nativeTracks()
                    latestNativeAt = SystemClock.elapsedRealtime() - diagnosticStart
                    updateTracks(JSONObject(latestNativeDiagnostics))
                }
                    .onFailure { networkStats.text = "诊断暂不可用：${it.message}" }
            }
            handler.postDelayed(this, 750)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        manager = getSystemService(AudioManager::class.java)
        // 0.1.2's reduced output buffer was not stable on the user's route.
        // Start this policy revision conservatively once, retaining other settings.
        val policyUpgrade = preferences.getInt("playback_policy_revision", 0) < 1
        if (policyUpgrade) preferences.edit().putString("playback_mode", "stable")
            .putInt("playback_policy_revision", 1).apply()
        playbackMode = PlaybackMode.fromKey(preferences.getString("playback_mode", null))
        selectedRate = preferences.getInt("output_rate", 0)
        mix = MixState(preferences)
        engine = PlaybackEngine(manager,
            onStatus = { text, active ->
                running = active
                status.text = text
                if (!active) { requested = false; window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
                startButton.text = if (requested) "停止接收" else "开始接收"
            },
            onActual = { actual.text = it },
            onReady = {
                mix.resetBindings()
                trackViews.clear()
                sourceDiagnostics.clear()
                sources.removeAllViews()
                emptySources.visibility = View.VISIBLE
                applyControl { mix.applyMaster() }
            },
            onFocusLost = { stopReceiver("音频焦点已改变 · 点击开始恢复") },
            onEvent = { recordEvent(it) }
        )
        createUi()
        if (policyUpgrade) recordEvent("已启用稳定优先：保留系统播放缓冲；均衡/响应为试验模式")
        manager.registerAudioDeviceCallback(deviceCallback, handler)
        updateDevices()
        NativeBridge.loadError?.let {
            status.text = "无法加载音频核心，请确认安装包支持当前设备架构。\n$it"
            startButton.isEnabled = false
        }
    }

    override fun onStart() {
        super.onStart()
        visible = true
        handler.post(poll)
    }

    override fun onStop() {
        visible = false
        handler.removeCallbacks(poll)
        stopReceiver("已停止 · 应用已进入后台")
        mix.save()
        super.onStop()
    }

    override fun onDestroy() {
        manager.unregisterAudioDeviceCallback(deviceCallback)
        handler.removeCallbacksAndMessages(null)
        engine.close()
        super.onDestroy()
    }

    private fun createUi() {
        val scroll = ScrollView(this).apply { setBackgroundColor(this@MainActivity.background); isFillViewport = true }
        root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(20), dp(16), dp(20), dp(28)) }
        scroll.addView(root)
        scroll.setOnApplyWindowInsetsListener { view, insets ->
            if (Build.VERSION.SDK_INT >= 30) {
                val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            } else {
                @Suppress("DEPRECATION")
                view.setPadding(insets.systemWindowInsetLeft, insets.systemWindowInsetTop,
                    insets.systemWindowInsetRight, insets.systemWindowInsetBottom)
            }
            insets
        }
        setContentView(scroll)
        root.addView(text("LANMIX / RECEIVER", 12, accent).apply { letterSpacing = .12f })
        root.addView(text("多个设备\n一个声音出口", 30, ink, true))
        root.addView(text("最多 16 路独立音源 · 音量与均衡器单独控制", 14, subtle))

        val connection = card(root, "接收与连接")
        status = text("准备就绪 · 点击开始接收", 14, accent)
        connection.addView(status)
        bufferHealth = text("接收后显示各音源的网络缓冲状态", 12, subtle)
        connection.addView(bufferHealth)
        connection.addView(text("将电脑音频发送到以下地址：\n${localAddresses()}", 13, subtle).apply { setTextIsSelectable(true) })
        val inputs = LinearLayout(this)
        port = numberInput("UDP 端口", preferences.getInt("port", 46210).toString())
        buffer = numberInput("网络初始缓冲 / ms", preferences.getInt("buffer", 150).toString())
        inputs.addView(field("UDP 端口", port), LinearLayout.LayoutParams(0, -2, 1f))
        inputs.addView(field("网络缓冲 / ms · 20–200", buffer), LinearLayout.LayoutParams(0, -2, 1f))
        connection.addView(inputs)
        connection.addView(text("Wi-Fi 建议先保留 150 ms。音源缺粮时自动增加，最高 400 ms，优先保持连续播放。", 12, subtle))
        startButton = Button(this).apply {
            text = "开始接收"; isAllCaps = false
            setOnClickListener {
                if (requested) stopReceiver("正在停止…") else {
                    if (Build.VERSION.SDK_INT >= 37 && checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED) {
                        requestPermissions(arrayOf(Manifest.permission.ACCESS_LOCAL_NETWORK), 37)
                    } else startReceiver()
                }
            }
        }
        connection.addView(startButton)
        connection.addView(text("接收期间请保持此页面在前台；进入后台或锁屏后停止播放。", 12, subtle))

        val output = card(root, "播放与输出")
        deviceSpinner = Spinner(this)
        output.addView(deviceSpinner)
        output.addView(text("应用输出采样率", 12, subtle))
        rateSpinner = Spinner(this)
        output.addView(rateSpinner)
        deviceSpinner.onItemSelectedListener = selection { position ->
            if (position != deviceSpinner.selectedItemPosition) return@selection
            val nextId = devices.getOrNull(position - 1)?.id
            if (!updating && nextId != selectedId) {
                selectedId = nextId
                updateRates()
                updateCapabilities()
                if (requested) startReceiver()
            }
        }
        rateSpinner.onItemSelectedListener = selection { position ->
            val next = rateOptions.getOrElse(position) { 0 }
            if (!updating && next != selectedRate) {
                selectedRate = next
                preferences.edit().putInt("output_rate", next).apply()
                if (requested) startReceiver()
            }
        }
        output.addView(text("播放模式", 12, subtle))
        val modes = PlaybackMode.entries
        val modeSpinner = Spinner(this).apply {
            adapter = adapter(modes.map { it.label })
            setSelection(modes.indexOf(playbackMode))
            onItemSelectedListener = selection { position ->
                val next = modes[position]
                if (next != playbackMode) {
                    playbackMode = next
                    preferences.edit().putString("playback_mode", next.key).apply()
                    if (requested) startReceiver()
                }
            }
        }
        output.addView(modeSpinner)
        output.addView(text("默认稳定优先，保留系统播放缓冲。兼容基线还原 0.1.1 的 1024 帧写入；均衡/响应为试验模式。切换会重启接收。", 12, subtle))
        actual = text("等待启动音频输出", 13, ink).apply { setTextIsSelectable(true) }
        output.addView(actual)

        val reports = card(root, "设备报告的音频能力")
        reports.addView(text("以下是系统报告的可用格式，不等同于当前播放格式或 DAC 的实际工作参数；空列表表示未报告。", 12, subtle))
        capabilities = text("", 12, subtle).apply { setTextIsSelectable(true) }
        val reportToggle = Button(this).apply { text = "展开设备能力"; isAllCaps = false }
        capabilities.visibility = View.GONE
        reportToggle.setOnClickListener {
            capabilities.visibility = if (capabilities.visibility == View.GONE) View.VISIBLE else View.GONE
            reportToggle.text = if (capabilities.visibility == View.VISIBLE) "收起设备能力" else "展开设备能力"
        }
        reports.addView(reportToggle)
        reports.addView(capabilities)

        val master = card(root, "总混音")
        master.addView(text("默认总音量 −6 dB，为多个音源叠加预留余量；提升 EQ 时建议降低预放大。", 12, subtle))
        masterMeter = text("峰值 — · 触发保护帧数 —", 12, accent)
        master.addView(masterMeter)
        addControls(master, mix.master, null)

        val sourceCard = card(root, "独立音源")
        sourceCard.addView(text("独立调节音量、静音、独听与 10 段 EQ；按音源 IP 和流标识保存设置。", 12, subtle))
        emptySources = text("等待音源连接…", 14, subtle)
        sourceCard.addView(emptySources)
        sources = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        sourceCard.addView(sources)
        val diagnostics = card(root, "网络与诊断")
        networkStats = text("本次尚未接收数据", 12, subtle).apply { setTextIsSelectable(true) }
        diagnostics.addView(networkStats)
        eventHistory = text("暂无事件", 12, subtle).apply {
            setTextIsSelectable(true); visibility = View.GONE
        }
        diagnostics.addView(Button(this).apply {
            text = "展开最近诊断事件"; isAllCaps = false
            setOnClickListener {
                eventHistory.visibility = if (eventHistory.visibility == View.GONE) View.VISIBLE else View.GONE
                text = if (eventHistory.visibility == View.VISIBLE) "收起诊断事件" else "展开最近诊断事件"
            }
        })
        diagnostics.addView(eventHistory)
        diagnostics.addView(Button(this).apply {
            text = "标记刚才的闪断"; isAllCaps = false
            setOnClickListener {
                val now = SystemClock.elapsedRealtime() - diagnosticStart
                markedDiagnostic = "+${now}ms 用户标记；音源采样时间 +${latestNativeAt}ms\n${actual.text}\n${compactDiagnostics()}"
                recordEvent("用户标记闪断；最近音源采样 +${latestNativeAt}ms")
                Toast.makeText(this@MainActivity, "已保留此刻状态，可复制诊断信息", Toast.LENGTH_SHORT).show()
            }
        })
        diagnostics.addView(Button(this).apply {
            text = "复制诊断信息"; isAllCaps = false
            setOnClickListener {
                val details = listOf("LanMix ${packageManager.getPackageInfo(packageName, 0).versionName}",
                    status.text, actual.text, bufferHealth.text, networkStats.text, masterMeter.text,
                    "音源诊断（+${latestNativeAt}ms 采样，仅省略未使用槽位的电平）：\n${compactDiagnostics()}",
                    "最近一次闪断标记：\n$markedDiagnostic",
                    "最近事件（本次打开应用，重建不清空）：\n${history.text()}", capabilities.text).joinToString("\n\n")
                getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("LanMix", details))
                Toast.makeText(this@MainActivity, "诊断信息已复制", Toast.LENGTH_SHORT).show()
            }
        })
    }

    private fun startReceiver(reason: String = "用户启动或设置变更") {
        if (!visible || NativeBridge.loadError != null) return
        val portValue = port.text.toString().toIntOrNull()
        val bufferValue = buffer.text.toString().toIntOrNull()
        if (portValue == null || portValue !in 1..65535 || bufferValue == null || bufferValue !in 20..200) {
            status.text = "端口范围 1–65535，网络初始缓冲范围 20–200 ms"
            return
        }
        preferences.edit().putInt("port", portValue).putInt("buffer", bufferValue).apply()
        requested = true
        running = false
        startButton.text = "停止接收"
        status.text = "正在启动音频与网络接收…"
        bufferHealth.text = "等待音源音频…"
        actual.text = "正在打开音频输出…"
        networkStats.text = "正在开启新的接收会话"
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        engine.start(PlaybackEngine.Config(portValue, bufferValue, devices.find { it.id == selectedId }, selectedRate, playbackMode), reason = reason)
    }

    private fun stopReceiver(message: String) {
        if (requested || running) recordEvent("停止请求：$message")
        requested = false
        running = false
        if (::startButton.isInitialized) startButton.text = "开始接收"
        if (::status.isInitialized) status.text = message
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        engine.stop()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 37) {
            if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED && visible) startReceiver()
            else status.text = "请允许本地网络访问，以接收局域网音频"
        }
    }

    private fun updateDevices() {
        if (!::deviceSpinner.isInitialized) return
        devices = manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).toList()
        val lostSelection = selectedId != null && devices.none { it.id == selectedId }
        if (lostSelection) selectedId = null
        updating = true
        deviceSpinner.adapter = adapter(listOf("跟随系统默认输出") + devices.map(OutputDevices::label))
        deviceSpinner.setSelection(devices.indexOfFirst { it.id == selectedId } + 1)
        updateRates()
        updateCapabilities()
        updating = false
        // AudioTrack follows actual same-rate routing. Adding an unrelated
        // output must not tear down UDP and discard all source buffers.
        if (requested && lostSelection) startReceiver("原首选输出已移除，回到系统默认")
    }

    private fun updateRates() {
        val wasUpdating = updating
        updating = true
        val reported = devices.find { it.id == selectedId }?.sampleRates?.toList().orEmpty()
        rateOptions = listOf(0) + (reported + listOf(44100, 48000, 96000, 192000))
            .filter { it in 8000..192000 }.distinct().sorted()
        if (selectedRate !in rateOptions) selectedRate = 0
        rateSpinner.adapter = adapter(rateOptions.map { if (it == 0) "自动 · 系统或设备原生采样率" else "$it Hz · 请求值" })
        rateSpinner.setSelection(rateOptions.indexOf(selectedRate))
        updating = wasUpdating
    }

    private fun updateCapabilities() {
        capabilities.text = devices.joinToString("\n\n", transform = OutputDevices::report).ifBlank { "系统未报告输出设备" }
    }

    private fun updateTracks(json: JSONObject) {
        val tracks = json.optJSONArray("tracks")
        val seen = mutableSetOf<Int>()
        if (tracks != null) repeat(tracks.length()) { i ->
            val item = tracks.getJSONObject(i)
            val slot = item.optInt("slot", -1)
            if (slot !in 0..15) return@repeat
            seen.add(slot)
            if (mix.bindTrack(slot, item.optString("source"), item.optLong("stream_id"))) {
                trackViews.remove(slot)?.let { sources.removeView(it.first.parent as View) }
                // The receiver gates a new identity with mute until its own
                // saved tone/solo settings are applied, then restores its mute.
                applyControl { mix.applyTrack(slot) }
            }
            val views = trackViews.getOrPut(slot) {
                val card = card(sources, "音源 ${slot + 1}")
                val name = text("", 14, ink, true)
                val details = text("", 12, subtle)
                card.addView(name); card.addView(details)
                addControls(card, mix.tracks[slot], slot)
                name to details
            }
            views.first.text = "${slot + 1} · ${item.optString("source")} / 流 ${item.optLong("stream_id")} · ${if (item.optBoolean("online")) "在线" else "离线"}"
            val meter = json.optJSONArray("meters")?.optJSONObject(slot)
            val identity = "${item.optString("source")}/${item.optLong("stream_id")}"
            val diagnostic = SourceDiagnostic(item.optLong("session_id"), item.optLong("concealed_frames"),
                item.optLong("late_packets"), item.optLong("queue_overflows"), meter?.optLong("underruns") ?: 0,
                item.optLong("reset_count"), meter?.optBoolean("primed") == true,
                item.optLong("gap_filled_packets"))
            val previous = sourceDiagnostics.put(identity, diagnostic)
            if (previous == null || previous != diagnostic) {
                recordEvent("音源 $identity：会话 ${diagnostic.session}，丢帧 ${diagnostic.lost}，迟到 ${diagnostic.late}，" +
                    "溢出 ${diagnostic.overflow}，缺粮 ${diagnostic.underruns}，重置 ${diagnostic.resets} " +
                    "(${item.optString("reset_reason")})，补回包 ${diagnostic.gapFilled}，" +
                    "最近缺口 ${item.optString("last_loss_kind")} / ${item.optLong("last_loss_frames")} 帧，" +
                    "${if (diagnostic.primed) "播放中" else "缓冲中"}")
            }
            views.second.text = "来源 ${item.optInt("sample_rate")} Hz · 双声道 PCM float\n" +
                "数据包 ${item.optLong("packets")} · 丢失帧 ${item.optLong("lost_frames")} · 迟到包 ${item.optLong("late_packets")}\n" +
                "补回包 ${item.optLong("gap_filled_packets")}（含乱序） · 去重 ${item.optLong("duplicate_packets")}\n" +
                "队列溢出 ${item.optLong("queue_overflows")} · 音源缺粮 ${meter?.optLong("underruns") ?: 0} · 峰值 ${peak(meter?.optDouble("peak") ?: 0.0)}\n" +
                "网络缓冲 ${String.format(Locale.US, "%.1f", meter?.optDouble("buffered_ms") ?: 0.0)} / 目标 ${meter?.optInt("target_buffer_ms") ?: 0} ms · ${if (meter?.optBoolean("primed") == true) "播放中" else "缓冲中"}"
        }
        trackViews.filterKeys { it !in seen }.forEach { (_, views) -> views.first.text = "此通道暂无音源"; views.second.text = "等待音源连接" }
        emptySources.visibility = if (seen.isEmpty()) View.VISIBLE else View.GONE
        val activeIdentities = (0 until (tracks?.length() ?: 0)).map { index ->
            val track = tracks!!.getJSONObject(index)
            "${track.optString("source")}/${track.optLong("stream_id")}"
        }.toSet()
        sourceDiagnostics.keys.retainAll(activeIdentities)
        val stats = json.optJSONObject("stats") ?: JSONObject()
        val meters = json.optJSONArray("meters")
        bufferHealth.text = if (seen.isEmpty()) "等待音源音频…" else seen.sorted().joinToString("\n") { slot ->
            val meter = meters?.optJSONObject(slot)
            "音源 ${slot + 1}：网络 ${String.format(Locale.US, "%.0f", meter?.optDouble("buffered_ms") ?: 0.0)} / 目标 ${meter?.optInt("target_buffer_ms") ?: 0} ms · 缺粮 ${meter?.optLong("underruns") ?: 0} · ${if (meter?.optBoolean("primed") == true) "播放中" else "缓冲中"}"
        }
        var concealed = 0L
        var lossEvents = 0L
        var resets = 0L
        var missing = 0L
        var senderGaps = 0L
        var recovered = 0L
        val recentLosses = mutableListOf<String>()
        if (tracks != null) repeat(tracks.length()) { index ->
            val track = tracks.getJSONObject(index)
            concealed += track.optLong("concealed_frames")
            lossEvents += track.optLong("loss_events")
            resets += track.optLong("reset_count")
            missing += track.optLong("missing_datagrams")
            senderGaps += track.optLong("sender_gap_frames")
            recovered += track.optLong("gap_filled_packets")
            if (!track.isNull("last_loss_age_ms")) {
                val kind = when (track.optString("last_loss_kind")) {
                    "sender_timeline" -> "发送时间线跳帧"
                    "unreceived_datagrams" -> "包序号缺失（网络或发送失败）"
                    else -> "序号无法分类"
                }
                recentLosses.add("音源 ${track.optInt("slot") + 1} 最近缺口：$kind · ${track.optLong("last_loss_frames")} 帧 · ${track.optLong("last_loss_age_ms")} ms 前")
            }
        }
        networkStats.text = "已接收 ${stats.optLong("received_packets")} 包 / ${stats.optLong("received_bytes")} 字节\n" +
            "无效包 ${stats.optLong("invalid_packets")} · 拒绝音源 ${stats.optLong("rejected_sources")}\n" +
            "本会话音源累计丢帧补偿 $concealed 帧 / $lossEvents 次 · 音源重置 $resets 次（含初次连接）\n" +
            "未收到序号 $missing 包 · 序号连续的发送缺口 $senderGaps 帧 · 补回 $recovered 包（含乱序）\n" +
            (if (recentLosses.isEmpty()) "" else recentLosses.joinToString("\n") + "\n") +
            "丢帧补偿和重置不会增加缺粮计数；写入耗时也不直接等同于断音。"
        val master = json.optJSONObject("master")
        masterMeter.text = "峰值 ${peak(master?.optDouble("peak") ?: 0.0)} · 触发保护帧数 ${master?.optLong("clipped_frames") ?: 0}"
    }

    private fun compactDiagnostics(): String = runCatching {
        val json = JSONObject(latestNativeDiagnostics)
        val tracks = json.optJSONArray("tracks")
        val slots = (0 until (tracks?.length() ?: 0)).map { tracks!!.getJSONObject(it).optInt("slot", -1) }.toSet()
        val meters = json.optJSONArray("meters")
        val activeMeters = org.json.JSONArray()
        if (meters != null) repeat(meters.length()) { index ->
            val meter = meters.getJSONObject(index)
            if (meter.optInt("slot", -1) in slots) activeMeters.put(meter)
        }
        json.put("meters", activeMeters).toString()
    }.getOrDefault(latestNativeDiagnostics)

    private fun recordEvent(event: String) {
        history.record(SystemClock.elapsedRealtime() - diagnosticStart, event)
        if (::eventHistory.isInitialized) eventHistory.text = history.text()
    }

    private fun addControls(parent: LinearLayout, control: MixControl, slot: Int?) {
        fun changed() {
            mix.save()
            if (running) applyControl { if (slot == null) mix.applyMaster() else mix.applyTrack(slot) }
        }
        slider(parent, "音量", control.gainDb, -60f, 12f) { control.gainDb = it; changed() }
        if (slot != null) {
            val toggles = LinearLayout(this)
            toggles.addView(check("静音", control.mute) { control.mute = it; changed() })
            toggles.addView(check("独听", control.solo) { control.solo = it; changed() })
            parent.addView(toggles)
        }
        val eq = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = View.GONE }
        val expand = Button(this).apply { text = "10 段均衡器 / 预放大"; isAllCaps = false }
        expand.setOnClickListener { eq.visibility = if (eq.visibility == View.GONE) View.VISIBLE else View.GONE }
        parent.addView(expand)
        parent.addView(eq)
        eq.addView(check("启用均衡器", control.eqEnabled) { control.eqEnabled = it; changed() })
        slider(eq, "预放大", control.preampDb, -24f, 12f) { control.preampDb = it; changed() }
        val bands = listOf("31.25 Hz", "62.5 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz")
        bands.forEachIndexed { index, label ->
            slider(eq, label, control.bands[index], -12f, 12f) { control.bands[index] = it; changed() }
        }
    }

    private fun applyControl(action: () -> Unit) {
        runCatching(action).onFailure { status.text = "调节失败：${it.message}" }
    }

    private fun slider(parent: LinearLayout, name: String, value: Float, minimum: Float, maximum: Float, changed: (Float) -> Unit) {
        val label = text("$name  ${db(value)}", 13, ink)
        parent.addView(label)
        parent.addView(SeekBar(this).apply {
            max = ((maximum - minimum) * 2).toInt()
            progress = ((value - minimum) * 2).toInt()
            contentDescription = "$name，单位分贝"
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(seekBar: SeekBar, progress: Int, fromUser: Boolean) {
                    if (!fromUser) return
                    val current = minimum + progress / 2f
                    label.text = "$name  ${db(current)}"
                    changed(current)
                }
                override fun onStartTrackingTouch(seekBar: SeekBar) {}
                override fun onStopTrackingTouch(seekBar: SeekBar) {}
            })
        })
    }

    private fun check(label: String, value: Boolean, changed: (Boolean) -> Unit) = CheckBox(this).apply {
        text = label; setTextColor(ink); isChecked = value
        setOnCheckedChangeListener { _, checked -> changed(checked) }
    }

    private fun card(parent: LinearLayout, title: String): LinearLayout {
        val card = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; setPadding(dp(16), dp(14), dp(16), dp(14))
            background = GradientDrawable().apply { setColor(surface); cornerRadius = dp(14).toFloat() }
        }
        parent.addView(card, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(16) })
        card.addView(text(title, 18, ink, true).apply { setPadding(0, 0, 0, dp(8)) })
        return card
    }

    private fun text(value: String, size: Int, color: Int, bold: Boolean = false) = TextView(this).apply {
        text = value; textSize = size.toFloat(); setTextColor(color); setPadding(0, dp(4), 0, dp(6))
        if (bold) setTypeface(typeface, Typeface.BOLD)
    }
    private fun numberInput(hintText: String, value: String) = EditText(this).apply {
        inputType = InputType.TYPE_CLASS_NUMBER; hint = hintText; setText(value)
        setTextColor(ink); setHintTextColor(subtle); setSingleLine()
    }
    private fun field(label: String, input: EditText) = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL; addView(text(label, 12, subtle)); addView(input)
    }
    private fun adapter(items: List<String>) = ArrayAdapter(this, android.R.layout.simple_spinner_item, items).apply {
        setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
    }
    private fun selection(action: (Int) -> Unit) = object : AdapterView.OnItemSelectedListener {
        override fun onNothingSelected(parent: AdapterView<*>?) {}
        override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) = action(position)
    }
    private fun localAddresses(): String = runCatching {
        NetworkInterface.getNetworkInterfaces().toList().filter { it.isUp && !it.isLoopback }
            .flatMap { it.inetAddresses.toList() }.filter { it is java.net.Inet4Address && !it.isLoopbackAddress }
            .joinToString("\n") { it.hostAddress ?: "" }.ifBlank { "请先连接局域网" }
    }.getOrDefault("暂时无法获取本地地址")
    private fun peak(value: Double) = if (value <= 0) "−∞ dBFS" else String.format(Locale.US, "%.1f dBFS", 20 * kotlin.math.log10(value))
    private fun db(value: Float) = String.format(Locale.US, "%+.1f dB", value)
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
}
