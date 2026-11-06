package io.lanmix.receiver

data class ConnectionRow(val key: String, val name: String, val detail: String)

/** Discovery identifies a session even when silent; PCM identifies actual received audio. */
internal fun connectionRows(connections: List<DeviceConnection>, sources: List<ReceiverSource>,
    nameFor: (String) -> String?): List<ConnectionRow> {
    fun host(address: String) = address.substringBeforeLast(':', address)
    val covered = mutableSetOf<String>()
    val rows = connections.map { connection ->
        val audio = sources.filter { host(it.address) == host(connection.address) && it.streamId == connection.streamId }
        audio.forEach { covered.add(it.identity) }
        ConnectionRow(connection.id, connection.name, when {
            audio.any { it.online } -> "接收中"
            connection.connected -> "已连接 · 等待电脑声音"
            else -> "正在连接…"
        })
    }
    return rows + sources.filter { it.online && it.identity !in covered }.map { source ->
        ConnectionRow(source.identity, nameFor(source.address) ?: source.address, "接收中")
    }
}
