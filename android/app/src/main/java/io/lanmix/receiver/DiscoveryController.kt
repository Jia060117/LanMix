package io.lanmix.receiver

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import org.json.JSONObject
import java.util.UUID

data class NearbyDevice(val id: String, val name: String, val role: String,
    val address: String, val port: Int, val ready: Boolean)

data class DeviceConnection(val id: String, val name: String, val address: String,
    val streamId: Long, val connected: Boolean)

interface DiscoveryBackend {
    val loadError: String?
    fun update(identity: String)
    fun snapshot(): String
    fun connect(id: String)
    fun probe(address: String)
    fun stop()
}

private object NativeDiscoveryBackend : DiscoveryBackend {
    override val loadError get() = NativeBridge.loadError
    override fun update(identity: String) = NativeBridge.nativeDiscoveryUpdate(identity)
    override fun snapshot() = NativeBridge.nativeDiscoverySnapshot()
    override fun connect(id: String) = NativeBridge.nativeDiscoveryConnect(id)
    override fun probe(address: String) = NativeBridge.nativeDiscoveryProbe(address)
    override fun stop() = NativeBridge.nativeDiscoveryStop()
}

/** Main-thread owner. Service keeps discovery alive while the screen is hidden. */
class DiscoveryController(private val context: Context, private val receiver: ReceiverState,
    private val backend: DiscoveryBackend = NativeDiscoveryBackend) {
    private val preferences = context.getSharedPreferences("lanmix", Context.MODE_PRIVATE)
    private val id = preferences.getString("device_id", null) ?: UUID.randomUUID().toString().replace("-", "").also {
        preferences.edit().putString("device_id", it).apply()
    }
    private val handler = Handler(Looper.getMainLooper())
    private val lease = DiscoveryLease()
    private var active = false
    private var identity = ""
    private val observers = mutableSetOf<() -> Unit>()
    var devices: List<NearbyDevice> = emptyList(); private set
    var connections: List<DeviceConnection> = emptyList(); private set
    private val requested = mutableMapOf<String, Pair<NearbyDevice, Long>>()
    var status = "正在发现局域网设备…"; private set
    private var noticeUntil = 0L
    private fun notice(message: String) {
        status = message; noticeUntil = SystemClock.elapsedRealtime() + 6000
    }
    private val poll = object : Runnable {
        override fun run() {
            if (!owned()) return
            refresh()
            if (owned()) handler.postDelayed(this, 200) else reconcile()
        }
    }
    // Keep an explicit in-flight connection across Activity recreation/start-service handoff.
    // The twelve-second deadline releases this temporary owner if startup fails.
    private fun owned() = lease.needed
    fun observe(observer: () -> Unit) { observers.add(observer); observer() }
    fun removeObserver(observer: () -> Unit) { observers.remove(observer) }
    fun screen(visible: Boolean) { lease.screen = visible; reconcile() }
    fun service(running: Boolean) {
        lease.service = running
        if (!running) { requested.clear(); connections = emptyList(); lease.cancel() }
        reconcile(); changed()
    }
    // Audio readiness is an event, not something a pending connection must
    // discover on the next one-second heartbeat.
    fun receiverReady() { if (owned()) { handler.removeCallbacks(poll); handler.post(poll) } }
    private fun reconcile() {
        handler.removeCallbacks(poll)
        if (owned()) handler.post(poll)
        else {
            if (active) runCatching { backend.stop() }
            active = false; identity = ""; devices = emptyList(); connections = emptyList(); requested.clear(); lease.cancel()
        }
    }
    private fun refresh() {
        val now = SystemClock.elapsedRealtime()
        val expired = requested.filterValues { now - it.second >= 12_000 }.keys.toList()
        expired.forEach { requested.remove(it) }
        if (expired.isNotEmpty()) {
            connections = connections.filterNot { it.id in expired }
            notice("连接未确认，请检查电脑上的 LanMix；旧版发送端会在收到音频后显示"); changed()
        }
        if (lease.expire(now)) { notice("连接超时，请确认电脑上的发送窗口仍在运行"); changed() }
        if (!owned()) return
        if (backend.loadError != null) { status = "音频核心不可用"; changed(); return }
        if (Build.VERSION.SDK_INT >= 37 && context.checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK)
            != PackageManager.PERMISSION_GRANTED) {
            status = "允许本地网络访问后，即可发现设备"; changed(); return
        }
        runCatching {
            val next = JSONObject().put("id", id).put("name", Build.MODEL.take(64))
                .put("role", "receiver").put("audio_port", receiver.config?.port ?: preferences.getInt("port", 46210))
                .put("ready", receiver.running).put("version", BuildConfig.VERSION_NAME).toString()
            if (!active || identity != next) {
                backend.update(next); active = true; identity = next
            }
            val snapshot = JSONObject(backend.snapshot())
            val peers = snapshot.optJSONArray("peers")
            val found = mutableListOf<NearbyDevice>()
            if (peers != null) repeat(peers.length()) { i ->
                val peer = peers.getJSONObject(i)
                found.add(NearbyDevice(peer.getString("id"), peer.getString("name"), peer.getString("role"),
                    peer.getString("address"), peer.getInt("audio_port"), peer.getBoolean("ready")))
            }
            devices = found
            val sessions = snapshot.optJSONArray("sessions")
            val live = mutableListOf<DeviceConnection>()
            if (receiver.running && sessions != null) repeat(sessions.length()) { index ->
                val session = sessions.getJSONObject(index)
                val state = session.optString("state")
                if (state == "connected" || state == "connecting") {
                    val connection = DeviceConnection(session.getString("id"), session.getString("name"),
                        session.getString("address"), session.optLong("stream_id", 1), state == "connected")
                    live.add(connection)
                    requested.remove(connection.id)
                }
            }
            // Old senders have no session notices; received PCM also confirms their request.
            requested.entries.removeAll { (_, request) -> receiver.sources.any { source ->
                source.online && source.address.substringBeforeLast(':', source.address) == request.first.address.substringBeforeLast(':')
            } }
            connections = live + requested.filterKeys { id -> live.none { it.id == id } }.map { (id, request) ->
                DeviceConnection(id, request.first.name, request.first.address, 1, false)
            }
            if (!snapshot.isNull("error")) status = "发现受限：${snapshot.optString("error")}"
            else if (SystemClock.elapsedRealtime() >= noticeUntil) status = "同一局域网 · 自动刷新"
            lease.target?.let { target ->
                val peer = devices.find { it.id == target }
                if (peer == null) {
                    requested.remove(target)
                    connections = connections.filterNot { it.id == target }
                    lease.cancel(); notice("连接未完成，请确认电脑上的发送窗口仍在运行")
                } else if (receiver.running) {
                    backend.connect(target)
                    lease.cancel(); notice("已请求连接 ${peer.name}，等待音源")
                }
            }
        }.onFailure { status = "自动发现暂不可用：${it.message}" }
        changed()
    }
    fun connect(device: NearbyDevice) {
        if (connections.any { it.id == device.id }) return
        requested[device.id] = device to SystemClock.elapsedRealtime()
        connections = connections + DeviceConnection(device.id, device.name, device.address, 1, false)
        lease.request(device.id, SystemClock.elapsedRealtime())
        notice("正在连接 ${device.name}…"); changed()
        handler.removeCallbacks(poll); handler.post(poll)
    }
    fun cancelPending() { lease.cancel(); requested.clear(); connections = emptyList(); reconcile(); changed() }
    fun probe(host: String): Boolean = runCatching {
        val parts = host.trim().split(".")
        require(parts.size == 4 && parts.all { p -> p.isNotEmpty() && p.all(Char::isDigit) && (p.toIntOrNull() ?: -1) in 0..255 }) {
            "请输入电脑的 IPv4 地址，例如 192.168.1.20"
        }
        check(active) { "请先允许本地网络访问" }
        backend.probe("${host.trim()}:46211")
        notice("正在查找 ${host.trim()}…"); changed()
    }.onFailure { status = it.message ?: "地址不可用"; changed() }.isSuccess
    fun nameFor(source: String): String? {
        val host = source.substringBeforeLast(':')
        return devices.firstOrNull { it.role == "sender" && it.address.substringBeforeLast(':') == host }?.name
    }
    private fun changed() { observers.toList().forEach { it() } }
}
