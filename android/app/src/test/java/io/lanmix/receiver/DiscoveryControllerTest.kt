package io.lanmix.receiver

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
class DiscoveryControllerTest {
    private class Backend : DiscoveryBackend {
        override val loadError: String? = null
        val connections = mutableListOf<String>()
        val identities = mutableListOf<String>()
        var snapshots = 0
        var sessions = "[]"
        override fun update(identity: String) { identities.add(identity) }
        override fun snapshot(): String {
            snapshots++
            return """{"peers":[{"id":"desktop","name":"台式机","role":"sender","address":"192.168.1.20:53000","audio_port":46210,"ready":true}],"sessions":$sessions,"error":null}"""
        }
        override fun connect(id: String) { connections.add(id) }
        override fun media(id: String, revision: Long, action: String) {}
        override fun probe(address: String) {}
        override fun stop() {}
    }
    private fun fixture(): Triple<ReceiverState, Backend, DiscoveryController> {
        val context: Context = RuntimeEnvironment.getApplication()
        val state = ReceiverState(MixState(context.getSharedPreferences("discovery-test", Context.MODE_PRIVATE)))
        val backend = Backend()
        return Triple(state, backend, DiscoveryController(context, state, backend))
    }
    private fun advance(ms: Long = 0) = shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(ms))

    @Test fun receiverReadinessSendsPendingConnectionBeforeTheNextHeartbeat() {
        val (state, backend, discovery) = fixture()
        discovery.screen(true); advance()
        discovery.connect(discovery.devices.single()); advance()
        assertTrue(backend.connections.isEmpty())
        state.running = true
        discovery.receiverReady(); advance()
        assertEquals(listOf("desktop"), backend.connections)
        assertTrue(backend.identities.last().contains("\"ready\":true"))
        advance(1000)
        assertEquals(listOf("desktop"), backend.connections)
        discovery.screen(false)
    }

    @Test fun readinessAfterOwnershipEndsCannotRestartDiscovery() {
        val (_, backend, discovery) = fixture()
        discovery.screen(true); advance()
        discovery.screen(false)
        val before = backend.snapshots
        discovery.receiverReady(); advance(1000)
        assertEquals(before, backend.snapshots)
        assertTrue(backend.connections.isEmpty())
    }

    @Test fun namesResolveFromPcmIpAndDiscoveryControlEndpoint() {
        val (_, _, discovery) = fixture()
        discovery.screen(true); advance()
        assertEquals("台式机", discovery.nameFor("192.168.1.20"))
        assertEquals("台式机", discovery.nameFor("192.168.1.20:12345"))
        assertNull(discovery.nameFor("192.168.1.99"))
        discovery.screen(false)
    }

    @Test fun requestAppearsImmediatelyAndConfirmationDoesNotNeedPcm() {
        val (state, backend, discovery) = fixture()
        discovery.screen(true); advance()
        discovery.connect(discovery.devices.single())
        assertEquals("台式机", discovery.connections.single().name)
        assertFalse(discovery.connections.single().connected)
        state.running = true; discovery.receiverReady(); advance()
        backend.sessions = """[{"id":"desktop","name":"台式机","address":"192.168.1.20:53000","stream_id":1,"state":"connected"}]"""
        advance(200)
        assertTrue(discovery.connections.single().connected)
        assertTrue(state.sources.isEmpty())
        discovery.connect(discovery.devices.single()); advance(200)
        assertEquals(1, backend.connections.size)
        backend.sessions = "[]"; advance(200)
        assertTrue(discovery.connections.isEmpty())
        discovery.screen(false)
    }

    @Test fun unconfirmedRequestExpiresAndServiceStopClearsSilentConnections() {
        val (state, _, discovery) = fixture()
        discovery.screen(true); advance()
        state.running = true
        discovery.connect(discovery.devices.single()); advance()
        assertEquals(1, discovery.connections.size)
        advance(12_000)
        assertTrue(discovery.connections.isEmpty())
        discovery.connect(discovery.devices.single()); advance()
        discovery.service(false)
        assertTrue(discovery.connections.isEmpty())
        discovery.screen(false)
    }
}
