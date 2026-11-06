package io.lanmix.receiver

enum class RemoteMediaAction(val wire: String, val label: String, val icon: Int) {
    PREVIOUS("previous", "上一首", R.drawable.ic_media_previous),
    PLAY_PAUSE("play_pause", "播放/暂停", R.drawable.ic_media_play_pause),
    NEXT("next", "下一首", R.drawable.ic_media_next);
    companion object { fun fromWire(value: String?) = entries.find { it.wire == value } }
}

data class RemoteMediaTarget(val id: String, val name: String, val address: String,
    val streamId: Long, val revision: Long)

/** Silence/paused music does not disconnect a session. Extra pending/PCM sources disable control. */
internal fun remoteMediaTarget(connections: List<DeviceConnection>, sources: List<ReceiverSource>): RemoteMediaTarget? {
    val device = connections.singleOrNull() ?: return null
    if (!device.connected || !device.mediaControls || device.revision <= 0) return null
    val host = device.address.substringBeforeLast(':')
    if (sources.any { it.online && (it.address.substringBeforeLast(':', it.address) != host || it.streamId != device.streamId) }) return null
    return RemoteMediaTarget(device.id, device.name, device.address, device.streamId, device.revision)
}
