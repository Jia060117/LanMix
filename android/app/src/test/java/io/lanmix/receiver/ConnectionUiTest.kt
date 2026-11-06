package io.lanmix.receiver

import org.junit.Assert.*
import org.junit.Test

class ConnectionUiTest {
    private val connection = DeviceConnection("desktop", "台式机", "192.168.1.20:53000", 1, true)

    @Test fun silentConnectedDeviceIsVisibleBeforeAnAudioSourceExists() {
        val row = connectionRows(listOf(connection), emptyList()) { null }.single()
        assertEquals("台式机", row.name)
        assertEquals("已连接 · 等待电脑声音", row.detail)
        assertEquals("正在连接…", connectionRows(listOf(connection.copy(connected = false)), emptyList()) { null }.single().detail)
    }
    @Test fun pcmReplacesWaitingStateWithoutDuplicatingDevice() {
        val rows = connectionRows(listOf(connection), listOf(ReceiverSource(0, "192.168.1.20", 1, 48000, true))) { null }
        assertEquals(1, rows.size)
        assertEquals("接收中", rows.single().detail)
    }
    @Test fun legacySenderStillAppearsAndDifferentStreamsArePreserved() {
        val sources = listOf(ReceiverSource(0, "192.168.1.20", 2, 48000, true), ReceiverSource(1, "192.168.1.21", 1, 48000, true))
        val rows = connectionRows(listOf(connection), sources) { if (it == "192.168.1.21") "笔记本" else null }
        assertEquals(3, rows.size)
        assertEquals("笔记本", rows.last().name)
        assertEquals(1, connectionRows(emptyList(), sources.map { it.copy(online = it.slot == 1) }) { null }.size)
    }
}
