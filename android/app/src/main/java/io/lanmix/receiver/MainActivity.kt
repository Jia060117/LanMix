package io.lanmix.receiver

import android.Manifest
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.res.Configuration
import android.content.res.ColorStateList
import android.view.Gravity
import android.graphics.drawable.RippleDrawable
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
import android.widget.*
import org.json.JSONObject
import java.net.NetworkInterface
import java.util.Locale

/** Controls the process state; hiding/recreating this screen never stops the service. */
class MainActivity : Activity() {
    private val dark get() = resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
    private val background get() = if (dark) Color.rgb(15, 20, 28) else Color.rgb(244, 246, 250)
    private val surface get() = if (dark) Color.rgb(27, 35, 47) else Color.WHITE
    private val ink get() = if (dark) Color.rgb(239, 243, 250) else Color.rgb(29, 38, 55)
    private val subtle get() = if (dark) Color.rgb(156, 169, 190) else Color.rgb(115, 126, 146)
    private val accent get() = if (dark) Color.rgb(135, 193, 255) else Color.rgb(62, 103, 222)
    private val discovery get() = (application as LanMixApplication).discovery
    private lateinit var navigation: GlassNavigation
    private lateinit var pageHost: FrameLayout
    private val pages = mutableListOf<ScrollView>()
    private var currentPage = 0
    private lateinit var nearby: LinearLayout
    private lateinit var nearbyStatus: TextView
    private var nearbySignature = ""
    private val discoveryObserver: () -> Unit = { updateNearby() }
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var manager: AudioManager
    private val receiver get() = (application as LanMixApplication).receiver
    private val mix get() = receiver.mix
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
    private val requested get() = receiver.requested
    private val running get() = receiver.running
    private var visible = false
    private val trackViews = mutableMapOf<Int, Pair<TextView, TextView>>()
    private val preferences by lazy { getSharedPreferences("lanmix", MODE_PRIVATE) }
    private val diagnosticStart get() = receiver.startedAt
    private val history get() = receiver.history
    private val latestNativeDiagnostics get() = receiver.diagnostics
    private val latestNativeAt get() = receiver.sampledAt
    private var markedDiagnostic: String
        get() = receiver.marked
        set(value) { receiver.marked = value }
    private var displayedEpoch = -1L
    private val viewIdentities = mutableMapOf<Int, String>()
    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) = updateDevices()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) = updateDevices()
    }
    private val receiverObserver: () -> Unit = { syncReceiver() }

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
        selectedId = receiver.config?.deviceId
        currentPage = savedInstanceState?.getInt("page") ?: 0
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
        if (requested) selectedId = receiver.config?.deviceId
        updateDevices()
        receiver.observe(receiverObserver)
        discovery.observe(discoveryObserver)
        discovery.screen(true)
        if (Build.VERSION.SDK_INT >= 37 && checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED &&
            !preferences.getBoolean("discovery_permission_asked", false)) {
            preferences.edit().putBoolean("discovery_permission_asked", true).apply()
            requestPermissions(arrayOf(Manifest.permission.ACCESS_LOCAL_NETWORK), 38)
        }
    }

    override fun onStop() {
        visible = false
        discovery.removeObserver(discoveryObserver)
        discovery.screen(false)
        receiver.removeObserver(receiverObserver)
        mix.save()
        super.onStop()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putInt("page", currentPage)
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        manager.unregisterAudioDeviceCallback(deviceCallback)
        handler.removeCallbacksAndMessages(null)
        receiver.removeObserver(receiverObserver)
        super.onDestroy()
    }

    private fun createUi() {
        if (Build.VERSION.SDK_INT >= 30) window.setDecorFitsSystemWindows(false)
        @Suppress("DEPRECATION")
        run { window.statusBarColor = Color.TRANSPARENT; window.navigationBarColor = Color.TRANSPARENT }
        val shell = FrameLayout(this).apply { setBackgroundColor(this@MainActivity.background) }
        pageHost = FrameLayout(this).apply { setBackgroundColor(this@MainActivity.background) }
        shell.addView(pageHost, FrameLayout.LayoutParams(-1, -1))
        fun page(title: String, subtitle: String): LinearLayout {
            val column = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(22), dp(16), dp(22), dp(104)) }
            val scroll = ScrollView(this).apply {
                isFillViewport = true; clipToPadding = false; isVerticalScrollBarEnabled = false
                addView(column); visibility = View.GONE
                setOnScrollChangeListener { _, _, _, _, _ -> if (::navigation.isInitialized) navigation.invalidate() }
            }
            pages.add(scroll); pageHost.addView(scroll)
            column.addView(text("LANMIX", 11, accent, true).apply { letterSpacing = .18f })
            column.addView(text(title, 28, ink, true).apply { setPadding(0, dp(6), 0, dp(4)) })
            column.addView(text(subtitle, 13, subtle)); return column
        }
        val home = page("让声音汇合", "多个设备，一个声音出口。")
        val devicesPage = page("附近的设备", "连接同一局域网，轻点即可汇入声音。")
        val settings = page("按你的习惯", "音质、输出与连接，一处管理。")
        navigation = GlassNavigation(this, pageHost, dark, accent) { showPage(it) }
        val barHeight = dp(64 + ((resources.configuration.fontScale - 1f).coerceAtLeast(0f) * 18).toInt())
        shell.addView(navigation, FrameLayout.LayoutParams(-2, barHeight, Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL).apply {
            leftMargin = dp(24); rightMargin = dp(24); bottomMargin = dp(12)
        })
        shell.setOnApplyWindowInsetsListener { _, insets ->
            val left: Int; val right: Int; val top: Int; val bottom: Int
            if (Build.VERSION.SDK_INT >= 30) {
                val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
                left = bars.left; right = bars.right; top = bars.top; bottom = bars.bottom
            } else {
                @Suppress("DEPRECATION")
                run { left = insets.systemWindowInsetLeft; right = insets.systemWindowInsetRight
                    top = insets.systemWindowInsetTop; bottom = insets.systemWindowInsetBottom }
            }
            pageHost.setPadding(left, top, right, 0)
            pages.forEach { (it.getChildAt(0) as LinearLayout).setPadding(dp(22), dp(16), dp(22), barHeight + dp(40) + bottom) }
            (navigation.layoutParams as FrameLayout.LayoutParams).apply {
                leftMargin = dp(24) + left; rightMargin = dp(24) + right; bottomMargin = dp(12) + bottom
                navigation.layoutParams = this
            }; insets
        }
        val connection = card(home, "声音入口")
        status = text("准备就绪", 15, accent, true); connection.addView(status)
        connection.addView(text("${localAddresses().replace("\n", "  ·  ")}\nUDP ${preferences.getInt("port", 46210)}", 12, subtle).apply { setTextIsSelectable(true) })
        startButton = button("开始接收", true) { if (requested) stopReceiver("用户点击停止") else requestReceiverStart() }
        connection.addView(startButton)
        connection.addView(text("可在后台持续播放，与手机上的其他声音一起聆听。", 12, subtle))
        val master = card(home, "总音量")
        masterMeter = text("等待声音汇入", 12, accent); master.addView(masterMeter); addControls(master, mix.master, null)
        val sourceCard = card(home, "我的音源")
        emptySources = text("还没有声音汇入\n在「设备」中连接电脑，或从电脑选择这台手机。", 14, subtle)
        sourceCard.addView(emptySources)
        sources = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; sourceCard.addView(sources)
        val discoverCard = card(devicesPage, "局域网设备")
        nearbyStatus = text("正在发现…", 12, subtle); discoverCard.addView(nearbyStatus)
        nearby = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; discoverCard.addView(nearby)
        discoverCard.addView(button("刷新设备") {
            if (Build.VERSION.SDK_INT >= 37 && checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED)
                requestPermissions(arrayOf(Manifest.permission.ACCESS_LOCAL_NETWORK), 38)
            else discovery.screen(true)
        })
        val manual = card(devicesPage, "手动查找")
        manual.addView(text("自动发现受网络限制时，输入电脑的局域网 IPv4 地址。", 12, subtle))
        val host = EditText(this).apply { hint = "192.168.1.20"; setSingleLine(); setTextColor(ink); setHintTextColor(subtle)
            inputType = InputType.TYPE_CLASS_PHONE; contentDescription = "电脑 IPv4 地址" }
        manual.addView(host); manual.addView(button("查找电脑") { discovery.probe(host.text.toString()) })
        manual.addView(text("也支持原来的 send --to IP:端口。请在电脑运行新版 LanMix 发送窗口。", 12, subtle))
        val output = card(settings, "声音输出")
        output.addView(text("跟随系统或指定蓝牙、耳机与 USB 声卡。", 12, subtle))
        deviceSpinner = Spinner(this); output.addView(deviceSpinner)
        output.addView(text("应用输出采样率", 12, subtle)); rateSpinner = Spinner(this); output.addView(rateSpinner)
        deviceSpinner.onItemSelectedListener = selection { position ->
            if (position != deviceSpinner.selectedItemPosition) return@selection
            val nextId = devices.getOrNull(position - 1)?.id
            if (!updating && nextId != selectedId) { selectedId = nextId; updateRates(); updateCapabilities(); if (requested) startReceiver() }
        }
        rateSpinner.onItemSelectedListener = selection { position ->
            val next = rateOptions.getOrElse(position) { 0 }
            if (!updating && next != selectedRate) {
                selectedRate = next; preferences.edit().putInt("output_rate", next).apply(); if (requested) startReceiver()
            }
        }
        actual = text("等待启动音频输出", 12, subtle).apply { setTextIsSelectable(true) }; output.addView(actual)
        val capabilitiesPanel = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = View.GONE }
        output.addView(button("查看设备音频能力") { toggle(capabilitiesPanel) }); output.addView(capabilitiesPanel)
        capabilitiesPanel.addView(text("系统报告值与当前播放格式分别展示；硬件与蓝牙链路可能执行转换。", 12, subtle))
        capabilities = text("", 12, subtle).apply { setTextIsSelectable(true) }; capabilitiesPanel.addView(capabilities)
        val network = card(settings, "播放与连接")
        network.addView(text("播放模式", 12, subtle)); val modes = PlaybackMode.entries
        network.addView(Spinner(this).apply {
            adapter = adapter(modes.map { it.label }); setSelection(modes.indexOf(playbackMode))
            onItemSelectedListener = selection { position ->
                val next = modes[position]
                if (next != playbackMode) { playbackMode = next; preferences.edit().putString("playback_mode", next.key).apply(); if (requested) startReceiver() }
            }
        })
        network.addView(text("稳定优先适合日常聆听。均衡与响应模式仍为试验选项。", 12, subtle))
        val inputs = LinearLayout(this)
        port = numberInput("UDP 端口", preferences.getInt("port", 46210).toString())
        buffer = numberInput("初始缓冲 / ms", preferences.getInt("buffer", 150).toString())
        inputs.addView(field("接收端口", port), LinearLayout.LayoutParams(0, -2, 1f))
        inputs.addView(field("初始缓冲 · 20–200 ms", buffer), LinearLayout.LayoutParams(0, -2, 1f)); network.addView(inputs)
        network.addView(button("保存连接设置") {
            val p = port.text.toString().toIntOrNull(); val b = buffer.text.toString().toIntOrNull()
            if (p == null || p !in 1..65535 || b == null || b !in 20..200) Toast.makeText(this, "端口 1–65535，缓冲 20–200 ms", Toast.LENGTH_SHORT).show()
            else { preferences.edit().putInt("port", p).putInt("buffer", b).apply()
                if (requested) startReceiver() else Toast.makeText(this, "设置已保存", Toast.LENGTH_SHORT).show() }
        })
        network.addView(text("建议保留 150 ms。调整运行中的输出或连接设置会重启接收。", 12, subtle))
        val diagnostics = card(settings, "诊断与帮助")
        val details = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = View.GONE }
        diagnostics.addView(button("查看接收状态") { toggle(details) }); diagnostics.addView(details)
        bufferHealth = text("接收后显示各音源缓冲", 12, subtle); details.addView(bufferHealth)
        networkStats = text("本次尚未接收数据", 12, subtle).apply { setTextIsSelectable(true) }; details.addView(networkStats)
        eventHistory = text("暂无事件", 12, subtle).apply { setTextIsSelectable(true) }; details.addView(eventHistory)
        diagnostics.addView(button("标记刚才的闪断") {
            val now = SystemClock.elapsedRealtime() - diagnosticStart
            markedDiagnostic = "+${now}ms 用户标记；音源采样时间 +${latestNativeAt}ms\n${actual.text}\n${compactDiagnostics()}"
            recordEvent("用户标记闪断；最近音源采样 +${latestNativeAt}ms")
            Toast.makeText(this, "已保留此刻状态，可复制诊断信息", Toast.LENGTH_SHORT).show()
        })
        diagnostics.addView(button("复制诊断信息") {
            val report = listOf("LanMix ${packageManager.getPackageInfo(packageName, 0).versionName}", status.text, actual.text,
                bufferHealth.text, networkStats.text, masterMeter.text, compactDiagnostics(), "最近闪断标记：\n$markedDiagnostic", history.text(), capabilities.text).joinToString("\n\n")
            getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("LanMix", report))
            Toast.makeText(this, "诊断信息已复制", Toast.LENGTH_SHORT).show()
        })
        settings.addView(text("LanMix ${BuildConfig.VERSION_NAME}  ·  正在施工中\n为每个设备，留一个声音的位置。", 12, subtle).apply { gravity = Gravity.CENTER; setPadding(0, dp(26), 0, dp(8)) })
        // PhoneWindow.getInsetsController() dereferences its decor, even when the
        // Kotlin call uses ?. Install the complete content before accessing it.
        setContentView(shell)
        if (Build.VERSION.SDK_INT >= 30) window.insetsController?.setSystemBarsAppearance(
            if (dark) 0 else android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
            android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS)
        showPage(currentPage, false)
    }
    private fun showPage(index: Int, animate: Boolean = true) {
        currentPage = index.coerceIn(0, 2)
        pages.forEachIndexed { i, view -> view.visibility = if (i == currentPage) View.VISIBLE else View.GONE }
        navigation.select(currentPage, animate)
    }
    private fun toggle(view: View) { view.visibility = if (view.visibility == View.GONE) View.VISIBLE else View.GONE }
    private fun updateNearby() {
        if (!::nearby.isInitialized) return
        nearbyStatus.text = discovery.status
        val signature = discovery.devices.joinToString { "${it.id}/${it.name}/${it.ready}/${it.address}" }
        if (signature == nearbySignature && nearby.childCount != 0) return
        nearbySignature = signature; nearby.removeAllViews()
        if (discovery.devices.isEmpty()) nearby.addView(text("还没有发现设备\n在电脑打开 LanMix，稍等片刻即可出现。", 14, subtle))
        discovery.devices.forEach { device ->
            val row = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(0, dp(12), 0, dp(10)) }
            row.addView(text(device.name, 17, ink, true))
            row.addView(text("${if (device.role == "sender") "电脑 · 发送端" else "接收端"} · ${device.address.substringBeforeLast(':')} · ${if (device.ready) "可连接" else "待命或使用中"}", 12, subtle))
            if (device.role == "sender") row.addView(button(if (device.ready) "连接并接收" else "暂不可连接", device.ready) {
                discovery.connect(device); if (!requested) requestReceiverStart(); showPage(0)
            }.apply { isEnabled = device.ready })
            nearby.addView(row)
        }
        navigation.invalidate()
    }
    private fun button(label: String, primary: Boolean = false, action: () -> Unit) = Button(this).apply {
        text = label; isAllCaps = false; textSize = 14f; minHeight = dp(48); setTextColor(if (primary) (if (dark) Color.rgb(16, 29, 49) else Color.WHITE) else accent)
        background = RippleDrawable(ColorStateList.valueOf(if (dark) 0x22FFFFFF else 0x18000000), GradientDrawable().apply {
            setColor(if (primary) accent else if (dark) 0xFF26354B.toInt() else 0xFFEDF2FF.toInt()); cornerRadius = dp(16).toFloat()
        }, null)
        layoutParams = LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(10); bottomMargin = dp(4) }; setOnClickListener { action() }
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
        runCatching {
            startForegroundService(ReceiverService.startIntent(this,
                ReceiverSettings(portValue, bufferValue, selectedId, selectedRate, playbackMode), reason))
        }.onFailure { status.text = "后台服务启动失败：${it.message}"; recordEvent(status.text.toString()) }
    }

    private fun stopReceiver(message: String) {
        discovery.cancelPending()
        if (requested || running) recordEvent("停止请求：$message")
        runCatching { startService(Intent(this, ReceiverService::class.java).setAction(ReceiverService.STOP)) }
            .onFailure { status.text = "停止请求失败：${it.message}" }
    }

    private fun requestReceiverStart() {
        if (Build.VERSION.SDK_INT >= 37 && checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.ACCESS_LOCAL_NETWORK), 37)
        } else if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED &&
            !preferences.getBoolean("notification_permission_asked", false)) {
            preferences.edit().putBoolean("notification_permission_asked", true).apply()
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 33)
        } else startReceiver()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 38) {
            if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED && visible) discovery.screen(true)
        } else if (requestCode == 37) {
            if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED && visible) requestReceiverStart()
            else { discovery.cancelPending(); status.text = "请允许本地网络访问，以接收局域网音频" }
        } else if (requestCode == 33 && visible) {
            if (grantResults.firstOrNull() != PackageManager.PERMISSION_GRANTED)
                Toast.makeText(this, "通知未获允许，仍可后台接收；回到应用可停止", Toast.LENGTH_LONG).show()
            startReceiver()
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
        // The service handles preferred-device removal even while this screen is hidden.
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
            val viewIdentity = "${item.optString("source")}/${item.optLong("stream_id")}"
            if (viewIdentities.put(slot, viewIdentity) != viewIdentity) {
                trackViews.remove(slot)?.let { sources.removeView(it.first.parent as View) }
            }
            val views = trackViews.getOrPut(slot) {
                val card = card(sources, "音源 ${slot + 1}")
                val name = text("", 14, ink, true)
                val details = text("", 12, subtle)
                card.addView(name)
                addControls(card, mix.tracks[slot], slot)
                details.visibility = View.GONE
                card.addView(button("音源状态") { toggle(details) }); card.addView(details)
                name to details
            }
            views.first.text = "${discovery.nameFor(item.optString("source")) ?: "音源 ${slot + 1}"} · ${item.optString("source")} / 流 ${item.optLong("stream_id")} · ${if (item.optBoolean("online")) "在线" else "离线"}"
            val meter = json.optJSONArray("meters")?.optJSONObject(slot)
            views.second.text = "来源 ${item.optInt("sample_rate")} Hz · 双声道 PCM float\n" +
                "数据包 ${item.optLong("packets")} · 丢失帧 ${item.optLong("lost_frames")} · 迟到包 ${item.optLong("late_packets")}\n" +
                "补回包 ${item.optLong("gap_filled_packets")}（含乱序） · 去重 ${item.optLong("duplicate_packets")}\n" +
                "队列溢出 ${item.optLong("queue_overflows")} · 音源缺粮 ${meter?.optLong("underruns") ?: 0} · 峰值 ${peak(meter?.optDouble("peak") ?: 0.0)}\n" +
                "网络缓冲 ${String.format(Locale.US, "%.1f", meter?.optDouble("buffered_ms") ?: 0.0)} / 目标 ${meter?.optInt("target_buffer_ms") ?: 0} ms · ${if (meter?.optBoolean("primed") == true) "播放中" else "缓冲中"}"
        }
        trackViews.filterKeys { it !in seen }.forEach { (_, views) -> views.first.text = "此通道暂无音源"; views.second.text = "等待音源连接" }
        emptySources.visibility = if (seen.isEmpty()) View.VISIBLE else View.GONE
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

    private fun syncReceiver() {
        if (!visible || !::status.isInitialized || NativeBridge.loadError != null) return
        status.text = receiver.status
        actual.text = receiver.actual
        startButton.text = if (requested) "停止接收" else "开始接收"
        eventHistory.text = history.text()
        navigation.invalidate()
        if (displayedEpoch != receiver.epoch) {
            displayedEpoch = receiver.epoch
            trackViews.clear()
            viewIdentities.clear()
            sources.removeAllViews()
            emptySources.visibility = View.VISIBLE
        }
        if (running) runCatching { updateTracks(JSONObject(latestNativeDiagnostics)) }
            .onFailure { networkStats.text = "诊断暂不可用：${it.message}" }
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
        val expand = button("均衡器与预放大") { toggle(eq) }
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
            progressTintList = ColorStateList.valueOf(accent); thumbTintList = ColorStateList.valueOf(accent); minimumHeight = dp(48)
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
        text = label; setTextColor(ink); isChecked = value; buttonTintList = ColorStateList.valueOf(accent); minHeight = dp(48)
        setOnCheckedChangeListener { _, checked -> changed(checked) }
    }

    private fun card(parent: LinearLayout, title: String): LinearLayout {
        val card = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; setPadding(dp(18), dp(16), dp(18), dp(16))
            background = GradientDrawable().apply { setColor(surface); cornerRadius = dp(22).toFloat() }
        }
        parent.addView(card, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
        card.addView(text(title, 16, ink, true).apply { setPadding(0, 0, 0, dp(8)) })
        return card
    }

    private fun text(value: String, size: Int, color: Int, bold: Boolean = false) = TextView(this).apply {
        text = value; textSize = size.toFloat(); setTextColor(color); setPadding(0, dp(2), 0, dp(4))
        if (bold) setTypeface(typeface, Typeface.BOLD)
    }
    private fun numberInput(hintText: String, value: String) = EditText(this).apply {
        inputType = InputType.TYPE_CLASS_NUMBER; hint = hintText; setText(value)
        setTextColor(ink); setHintTextColor(subtle); setSingleLine()
    }
    private fun field(label: String, input: EditText) = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL; addView(text(label, 12, subtle)); addView(input)
    }
    private fun adapter(items: List<String>) = object : ArrayAdapter<String>(this, android.R.layout.simple_spinner_dropdown_item, items) {
        override fun getView(position: Int, convertView: View?, parent: android.view.ViewGroup): View =
            (super.getView(position, convertView, parent) as TextView).apply { setTextColor(ink); textSize = 14f; minHeight = dp(48) }
        override fun getDropDownView(position: Int, convertView: View?, parent: android.view.ViewGroup): View =
            (super.getDropDownView(position, convertView, parent) as TextView).apply { setTextColor(ink); setBackgroundColor(surface); minHeight = dp(48) }
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
