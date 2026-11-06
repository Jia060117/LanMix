package io.lanmix.receiver

import android.app.Notification
import android.content.Context
import android.os.Looper
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.time.Duration

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 35])
class RemoteMediaTest {
    private val connection = DeviceConnection("desktop", "台式机", "192.168.1.20:53000", 1, true, true, 2)
    private val source = ReceiverSource(0, "192.168.1.20", 1, 48000, true)
    private val target get() = remoteMediaTarget(listOf(connection), emptyList())!!

    @Test fun onlyOneConfirmedCapableDeviceCanBeControlledEvenWhenItIsSilent() {
        assertEquals(target, remoteMediaTarget(listOf(connection), listOf(source)))
        assertEquals(target, remoteMediaTarget(listOf(connection), listOf(source.copy(online = false))))
        assertNull(remoteMediaTarget(emptyList(), listOf(source)))
        assertNull(remoteMediaTarget(listOf(connection.copy(connected = false)), emptyList()))
        assertNull(remoteMediaTarget(listOf(connection.copy(mediaControls = false)), listOf(source)))
        assertNull(remoteMediaTarget(listOf(connection.copy(revision = 0)), emptyList()))
        assertNull(remoteMediaTarget(listOf(connection, connection.copy(id = "laptop", connected = false)), emptyList()))
        assertNull(remoteMediaTarget(listOf(connection), listOf(source, source.copy(address = "192.168.1.30"))))
        assertNull(remoteMediaTarget(listOf(connection), listOf(source.copy(streamId = 2))))
    }

    @Test fun notificationShowsThreeCompactCommandsAndKeepsStopInExpandedView() {
        val context = RuntimeEnvironment.getApplication()
        val notification = receiverNotification(context, "test", "ready", target)
        assertEquals(Notification.CATEGORY_TRANSPORT, notification.category)
        assertEquals(listOf("上一首", "播放/暂停", "下一首", "停止接收"), notification.actions.map { it.title.toString() })
        assertArrayEquals(intArrayOf(0, 1, 2), notification.extras.getIntArray("android.compactActions"))
        assertFalse(notification.extras.containsKey(Notification.EXTRA_MEDIA_SESSION))
        assertTrue(notification.flags and Notification.FLAG_ONGOING_EVENT != 0)
        val idle = receiverNotification(context, "test", "ready", null)
        assertEquals(listOf("停止接收"), idle.actions.map { it.title.toString() })
        assertEquals(Notification.CATEGORY_SERVICE, idle.category)
    }

    @Test fun oldNotificationCannotSwitchTargetsOrControlANewSession() {
        val context = RuntimeEnvironment.getApplication()
        val old = receiverNotification(context, "test", "ready", target)
        val next = target.copy(revision = 3)
        val fresh = receiverNotification(context, "test", "ready", next)
        assertNotEquals(old.actions[1].actionIntent, fresh.actions[1].actionIntent)
        val click = shadowOf(old.actions[1].actionIntent).savedIntent
        val sent = mutableListOf<RemoteMediaAction>()
        fun dispatch(current: RemoteMediaTarget?) = dispatchMediaIntent(click, current) { _, action -> sent.add(action); true }
        assertFalse(dispatch(null))
        assertFalse(dispatch(next))
        assertFalse(dispatch(target.copy(id = "another-device")))
        assertFalse(dispatch(target.copy(streamId = 2)))
        assertTrue(dispatch(target))
        assertEquals(listOf(RemoteMediaAction.PLAY_PAUSE), sent)
        click.putExtra("command", "unknown")
        assertFalse(dispatch(target))
        assertEquals(1, sent.size)
    }

    @Test fun backgroundDiscoveryRoutesClicksAndRevalidatesChangesBeforeSending() {
        val context = RuntimeEnvironment.getApplication()
        val state = ReceiverState(MixState(context.getSharedPreferences("media-test", Context.MODE_PRIVATE)))
        state.requested = true; state.running = true
        val sent = mutableListOf<String>()
        var revision = 2
        val backend = object : DiscoveryBackend {
            override val loadError: String? = null
            override fun update(identity: String) {}
            override fun snapshot() = """{"peers":[],"sessions":[{"id":"desktop","name":"台式机","address":"192.168.1.20:53000","stream_id":1,"state":"connected","media_controls":true,"revision":$revision}],"error":null}"""
            override fun connect(id: String) {}
            override fun media(id: String, revision: Long, action: String) { sent.add("$id/$revision/$action") }
            override fun probe(address: String) {}
            override fun stop() {}
        }
        val discovery = DiscoveryController(context, state, backend)
        discovery.service(true)
        shadowOf(Looper.getMainLooper()).idle()
        val current = discovery.mediaTarget!!
        assertTrue(discovery.sendMedia(current, RemoteMediaAction.NEXT))
        assertEquals(listOf("desktop/2/next"), sent)
        state.sources = listOf(source.copy(address = "192.168.1.30"))
        assertNull(discovery.mediaTarget)
        assertFalse(discovery.sendMedia(current, RemoteMediaAction.NEXT))
        state.sources = emptyList()
        revision = 3
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
        assertFalse(discovery.sendMedia(current, RemoteMediaAction.PLAY_PAUSE))
        assertTrue(discovery.sendMedia(discovery.mediaTarget!!, RemoteMediaAction.PREVIOUS))
        discovery.service(false)
        assertNull(discovery.mediaTarget)
        assertFalse(discovery.sendMedia(current, RemoteMediaAction.NEXT))
        assertEquals(listOf("desktop/2/next", "desktop/3/previous"), sent)
    }
}
