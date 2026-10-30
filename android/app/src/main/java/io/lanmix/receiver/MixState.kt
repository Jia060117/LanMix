package io.lanmix.receiver

import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject

class MixControl {
    var gainDb = 0f
    var mute = false
    var solo = false
    var eqEnabled = false
    var preampDb = 0f
    val bands = FloatArray(10)

    fun json() = JSONObject().put("gain", gainDb).put("mute", mute).put("solo", solo)
        .put("eq", eqEnabled).put("preamp", preampDb).put("bands", JSONArray(bands.toList()))

    fun load(json: JSONObject?) {
        if (json == null) return
        gainDb = json.optDouble("gain", 0.0).toFloat().coerceIn(-60f, 12f)
        mute = json.optBoolean("mute")
        solo = json.optBoolean("solo")
        eqEnabled = json.optBoolean("eq")
        preampDb = json.optDouble("preamp", 0.0).toFloat().coerceIn(-24f, 12f)
        repeat(10) { bands[it] = (json.optJSONArray("bands")?.optDouble(it, 0.0)
            ?: 0.0).toFloat().coerceIn(-12f, 12f) }
    }
}

class MixState(private val preferences: SharedPreferences) {
    val master = MixControl().apply { gainDb = -6f }
    val tracks = Array(16) { MixControl() }
    private val identities = arrayOfNulls<String>(16)
    private val sources = mutableMapOf<String, MixControl>()
    init {
        runCatching {
            val json = JSONObject(preferences.getString("mix", "{}") ?: "{}")
            master.load(json.optJSONObject("master"))
            val saved = json.optJSONObject("sources")
            saved?.keys()?.forEach { identity ->
                sources[identity] = MixControl().apply { load(saved.optJSONObject(identity)) }
            }
        }
    }
    fun save() {
        preferences.edit().putString("mix", JSONObject().put("master", master.json())
            .put("sources", JSONObject().apply { sources.forEach { (key, control) -> put(key, control.json()) } }).toString()).apply()
    }
    fun applyMaster() = master.let {
        NativeBridge.nativeSetMaster(it.gainDb, it.eqEnabled, it.preampDb, it.bands)
    }
    fun applyTrack(index: Int) = tracks[index].let {
        NativeBridge.nativeSetTrack(index, it.gainDb, it.mute, it.solo, it.eqEnabled, it.preampDb, it.bands)
    }
    fun resetBindings() { repeat(16) { identities[it] = null; tracks[it] = MixControl() } }
    fun bindTrack(index: Int, source: String, streamId: Long): Boolean {
        val identity = "$source/$streamId"
        if (identities[index] == identity) return false
        tracks[index] = sources.getOrPut(identity) { MixControl() }
        identities[index] = identity
        return true
    }
}
