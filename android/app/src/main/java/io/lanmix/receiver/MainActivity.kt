package io.lanmix.receiver

import android.Manifest
import android.app.Activity
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
    private var updating = false
    private var requested = false
    private var running = false
    private var visible = false
    private val trackViews = mutableMapOf<Int, Pair<TextView, TextView>>()
    private val preferences by lazy { getSharedPreferences("lanmix", MODE_PRIVATE) }
    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) = updateDevices()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) = updateDevices()
    }
    private val poll = object : Runnable {
        override fun run() {
            if (!visible) return
            if (NativeBridge.loadError == null && running) {
                runCatching { updateTracks(JSONObject(NativeBridge.nativeTracks())) }
                    .onFailure { networkStats.text = "Diagnostics unavailable: ${it.message}" }
            }
            handler.postDelayed(this, 750)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        manager = getSystemService(AudioManager::class.java)
        mix = MixState(preferences)
        engine = PlaybackEngine(manager,
            onStatus = { text, active ->
                running = active
                status.text = text
                if (!active) { requested = false; window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
                startButton.text = if (requested) "Stop receiver" else "Start receiver"
            },
            onActual = { actual.text = it },
            onReady = {
                mix.resetBindings()
                trackViews.clear()
                sources.removeAllViews()
                emptySources.visibility = View.VISIBLE
                applyControl { mix.applyMaster() }
            },
            onFocusLost = { stopReceiver("Audio focus changed · tap Start to resume") }
        )
        createUi()
        manager.registerAudioDeviceCallback(deviceCallback, handler)
        updateDevices()
        NativeBridge.loadError?.let {
            status.text = "Native library unavailable. Build and package the Rust library for this device ABI.\n$it"
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
        stopReceiver("Stopped · Activity is in the background")
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
        root.addView(text("One output.\nEvery source.", 30, ink, true))
        root.addView(text("Native stereo receiver · up to 16 independent sources", 14, subtle))

        val connection = card(root, "Receiver")
        status = text("Ready · start to open the UDP receiver", 14, accent)
        connection.addView(status)
        bufferHealth = text("Source buffer diagnostics appear here while receiving", 12, subtle)
        connection.addView(bufferHealth)
        connection.addView(text("Send unicast UDP to this device:\n${localAddresses()}", 13, subtle).apply { setTextIsSelectable(true) })
        val inputs = LinearLayout(this)
        port = numberInput("UDP port", preferences.getInt("port", 46210).toString())
        buffer = numberInput("Initial jitter buffer / ms", preferences.getInt("buffer", 150).toString())
        inputs.addView(field("UDP port", port), LinearLayout.LayoutParams(0, -2, 1f))
        inputs.addView(field("Buffer / ms · 20–200", buffer), LinearLayout.LayoutParams(0, -2, 1f))
        connection.addView(inputs)
        connection.addView(text("Initial target; a starving source can adapt up to 400 ms to keep playback continuous. Try 150 ms for Wi-Fi.", 12, subtle))
        startButton = Button(this).apply {
            text = "Start receiver"; isAllCaps = false
            setOnClickListener {
                if (requested) stopReceiver("Stopping…") else {
                    if (Build.VERSION.SDK_INT >= 37 && checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED) {
                        requestPermissions(arrayOf(Manifest.permission.ACCESS_LOCAL_NETWORK), 37)
                    } else startReceiver()
                }
            }
        }
        connection.addView(startButton)
        connection.addView(text("Activity only: playback stops when this screen enters the background. Keep the screen open while receiving.", 12, subtle))

        val output = card(root, "Output device")
        deviceSpinner = Spinner(this)
        output.addView(deviceSpinner)
        output.addView(text("Requested AudioTrack rate", 12, subtle))
        rateSpinner = Spinner(this)
        output.addView(rateSpinner)
        deviceSpinner.onItemSelectedListener = selection { position ->
            if (!updating) {
                selectedId = devices.getOrNull(position - 1)?.id
                updateRates()
                updateCapabilities()
                if (requested) startReceiver()
            }
        }
        rateSpinner.onItemSelectedListener = selection { position ->
            val next = rateOptions.getOrElse(position) { 0 }
            if (!updating && next != selectedRate) {
                selectedRate = next
                if (requested) startReceiver()
            }
        }
        actual = text("No active AudioTrack", 13, ink)
        output.addView(actual)

        val reports = card(root, "Reported capabilities")
        reports.addView(text("Device declarations describe possible formats. They do not establish the current stream or physical DAC format. Empty lists mean unspecified.", 12, subtle))
        capabilities = text("", 12, subtle).apply { setTextIsSelectable(true) }
        val reportToggle = Button(this).apply { text = "Show all output reports"; isAllCaps = false }
        capabilities.visibility = View.GONE
        reportToggle.setOnClickListener {
            capabilities.visibility = if (capabilities.visibility == View.GONE) View.VISIBLE else View.GONE
            reportToggle.text = if (capabilities.visibility == View.VISIBLE) "Hide output reports" else "Show all output reports"
        }
        reports.addView(reportToggle)
        reports.addView(capabilities)

        val master = card(root, "Master mix")
        master.addView(text("Reserve headroom when combining sources. Master starts at −6 dB; lower preamp before boosting EQ.", 12, subtle))
        masterMeter = text("Peak — · clipped frames —", 12, accent)
        master.addView(masterMeter)
        addControls(master, mix.master, null)

        val sourceCard = card(root, "Sources")
        sourceCard.addView(text("Each source has its own gain, mute, solo, preamp and ten-band EQ. Settings follow its IP address and stream ID.", 12, subtle))
        emptySources = text("Waiting for source packets…", 14, subtle)
        sourceCard.addView(emptySources)
        sources = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        sourceCard.addView(sources)
        val diagnostics = card(root, "Network / diagnostics")
        networkStats = text("No packets received in this session", 12, subtle).apply { setTextIsSelectable(true) }
        diagnostics.addView(networkStats)
    }

    private fun startReceiver() {
        if (!visible || NativeBridge.loadError != null) return
        val portValue = port.text.toString().toIntOrNull()
        val bufferValue = buffer.text.toString().toIntOrNull()
        if (portValue == null || portValue !in 1..65535 || bufferValue == null || bufferValue !in 20..200) {
            status.text = "Use a port from 1–65535 and a jitter buffer from 20–200 ms"
            return
        }
        preferences.edit().putInt("port", portValue).putInt("buffer", bufferValue).apply()
        requested = true
        running = false
        startButton.text = "Stop receiver"
        status.text = "Starting audio and UDP…"
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        engine.start(PlaybackEngine.Config(portValue, bufferValue, devices.find { it.id == selectedId }, selectedRate))
    }

    private fun stopReceiver(message: String) {
        requested = false
        running = false
        if (::startButton.isInitialized) startButton.text = "Start receiver"
        if (::status.isInitialized) status.text = message
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        engine.stop()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 37) {
            if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED && visible) startReceiver()
            else status.text = "Local network permission is needed to receive LAN audio"
        }
    }

    private fun updateDevices() {
        if (!::deviceSpinner.isInitialized) return
        val previousIds = devices.map { it.id }
        devices = manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).toList()
        val lostSelection = selectedId != null && devices.none { it.id == selectedId }
        if (lostSelection) selectedId = null
        updating = true
        deviceSpinner.adapter = adapter(listOf("System default output") + devices.map(OutputDevices::label))
        deviceSpinner.setSelection(devices.indexOfFirst { it.id == selectedId } + 1)
        updateRates()
        updateCapabilities()
        updating = false
        if (requested && (lostSelection || previousIds != devices.map { it.id })) startReceiver()
    }

    private fun updateRates() {
        val wasUpdating = updating
        updating = true
        val reported = devices.find { it.id == selectedId }?.sampleRates?.toList().orEmpty()
        rateOptions = listOf(0) + (reported + listOf(44100, 48000, 96000, 192000))
            .filter { it in 8000..192000 }.distinct().sorted()
        if (selectedRate !in rateOptions) selectedRate = 0
        rateSpinner.adapter = adapter(rateOptions.map { if (it == 0) "Auto · Android/device native rate" else "$it Hz · requested" })
        rateSpinner.setSelection(rateOptions.indexOf(selectedRate))
        updating = wasUpdating
    }

    private fun updateCapabilities() {
        capabilities.text = devices.joinToString("\n\n", transform = OutputDevices::report).ifBlank { "Android reports no output devices" }
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
                val card = card(sources, "Source ${slot + 1}")
                val name = text("", 14, ink, true)
                val details = text("", 12, subtle)
                card.addView(name); card.addView(details)
                addControls(card, mix.tracks[slot], slot)
                name to details
            }
            views.first.text = "${slot + 1} · ${item.optString("source")} / stream ${item.optLong("stream_id")} · ${if (item.optBoolean("online")) "online" else "offline"}"
            val meter = json.optJSONArray("meters")?.optJSONObject(slot)
            views.second.text = "Source ${item.optInt("sample_rate")} Hz · stereo PCM float\n" +
                "Packets ${item.optLong("packets")} · lost frames ${item.optLong("lost_frames")} · late ${item.optLong("late_packets")}\n" +
                "Queue overflow ${item.optLong("queue_overflows")} · underruns ${meter?.optLong("underruns") ?: 0} · peak ${peak(meter?.optDouble("peak") ?: 0.0)}\n" +
                "Buffer ${String.format(Locale.US, "%.1f", meter?.optDouble("buffered_ms") ?: 0.0)} / target ${meter?.optInt("target_buffer_ms") ?: 0} ms · ${if (meter?.optBoolean("primed") == true) "playing" else "buffering"}"
        }
        trackViews.filterKeys { it !in seen }.forEach { (_, views) -> views.first.text = "No source in this receiver slot"; views.second.text = "Waiting for this session" }
        emptySources.visibility = if (seen.isEmpty()) View.VISIBLE else View.GONE
        val stats = json.optJSONObject("stats") ?: JSONObject()
        val meters = json.optJSONArray("meters")
        bufferHealth.text = if (seen.isEmpty()) "Waiting for source audio…" else seen.sorted().joinToString("\n") { slot ->
            val meter = meters?.optJSONObject(slot)
            "Source ${slot + 1}: ${String.format(Locale.US, "%.0f", meter?.optDouble("buffered_ms") ?: 0.0)} / ${meter?.optInt("target_buffer_ms") ?: 0} ms · underruns ${meter?.optLong("underruns") ?: 0} · ${if (meter?.optBoolean("primed") == true) "playing" else "buffering"}"
        }
        networkStats.text = "Received ${stats.optLong("received_packets")} packets / ${stats.optLong("received_bytes")} bytes\n" +
            "Invalid ${stats.optLong("invalid_packets")} · rejected sources ${stats.optLong("rejected_sources")}"
        val master = json.optJSONObject("master")
        masterMeter.text = "Peak ${peak(master?.optDouble("peak") ?: 0.0)} · clipped frames ${master?.optLong("clipped_frames") ?: 0}"
    }

    private fun addControls(parent: LinearLayout, control: MixControl, slot: Int?) {
        fun changed() {
            mix.save()
            if (running) applyControl { if (slot == null) mix.applyMaster() else mix.applyTrack(slot) }
        }
        slider(parent, "Gain", control.gainDb, -60f, 12f) { control.gainDb = it; changed() }
        if (slot != null) {
            val toggles = LinearLayout(this)
            toggles.addView(check("Mute", control.mute) { control.mute = it; changed() })
            toggles.addView(check("Solo", control.solo) { control.solo = it; changed() })
            parent.addView(toggles)
        }
        val eq = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = View.GONE }
        val expand = Button(this).apply { text = "Ten-band EQ / preamp"; isAllCaps = false }
        expand.setOnClickListener { eq.visibility = if (eq.visibility == View.GONE) View.VISIBLE else View.GONE }
        parent.addView(expand)
        parent.addView(eq)
        eq.addView(check("Enable EQ", control.eqEnabled) { control.eqEnabled = it; changed() })
        slider(eq, "Preamp", control.preampDb, -24f, 12f) { control.preampDb = it; changed() }
        val bands = listOf("31.25 Hz", "62.5 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz")
        bands.forEachIndexed { index, label ->
            slider(eq, label, control.bands[index], -12f, 12f) { control.bands[index] = it; changed() }
        }
    }

    private fun applyControl(action: () -> Unit) {
        runCatching(action).onFailure { status.text = "Control update failed: ${it.message}" }
    }

    private fun slider(parent: LinearLayout, name: String, value: Float, minimum: Float, maximum: Float, changed: (Float) -> Unit) {
        val label = text("$name  ${db(value)}", 13, ink)
        parent.addView(label)
        parent.addView(SeekBar(this).apply {
            max = ((maximum - minimum) * 2).toInt()
            progress = ((value - minimum) * 2).toInt()
            contentDescription = "$name in decibels"
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
            .joinToString("\n") { it.hostAddress ?: "" }.ifBlank { "Connect to a local network" }
    }.getOrDefault("Local address unavailable")
    private fun peak(value: Double) = if (value <= 0) "−∞ dBFS" else String.format(Locale.US, "%.1f dBFS", 20 * kotlin.math.log10(value))
    private fun db(value: Float) = String.format(Locale.US, "%+.1f dB", value)
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
}
