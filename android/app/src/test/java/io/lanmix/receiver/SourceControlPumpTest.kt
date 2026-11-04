package io.lanmix.receiver

import android.os.Handler
import android.os.Looper
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.time.Duration

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 35])
class SourceControlPumpTest {
    private val source = """[{"slot":0,"source":"192.168.1.20","stream_id":1,"sample_rate":48000,"online":true}]"""
    private fun advance(ms: Long) = shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(ms))

    @Test fun aNewSourceRestoresSettingsAndPublishesWithoutWaitingForDiagnostics() {
        val events = mutableListOf<String>()
        val binder = SourceControlBinder({}, {}, { _, _, _ -> true }, { events.add("applied") })
        var snapshot = "[]"
        val publications = mutableListOf<List<ReceiverSource>>()
        val pump = SourceControlPump(Handler(Looper.getMainLooper()), { snapshot }, binder::source,
            { publications.add(it); if (it.isNotEmpty()) events.add("shown") }, { throw it })
        pump.start(); advance(0)
        advance(51); snapshot = source
        advance(49)
        assertEquals(listOf("applied", "shown"), events)
        assertEquals("192.168.1.20/1", publications.last().single().identity)
        assertEquals(48000, publications.last().single().sampleRate)
        assertTrue(publications.last().single().online)
        pump.stop()
    }

    @Test fun stableTopologyDoesNotReapplyControlsOrRebuildTheEditor() {
        var applications = 0
        var identity: String? = null
        var publications = 0
        val binder = SourceControlBinder({}, {}, { _, address, stream ->
            val next = "$address/$stream"; val changed = next != identity; identity = next; changed
        }, { applications++ })
        val pump = SourceControlPump(Handler(Looper.getMainLooper()), { source }, binder::source,
            { publications++ }, { throw it })
        pump.start(); advance(1000)
        assertEquals(1, applications)
        assertEquals(1, publications)
        pump.stop()
    }

    @Test fun aTemporaryApplyFailureRetriesBeforePublishingAndStopCancelsFurtherReads() {
        var reads = 0
        var attempts = 0
        var bound = false
        var publications = 0
        val errors = mutableListOf<String>()
        val binder = SourceControlBinder({}, {}, { _, _, _ -> (!bound).also { bound = true } }, {
            attempts++; if (attempts == 1) error("session changing")
        })
        val pump = SourceControlPump(Handler(Looper.getMainLooper()), { reads++; source }, binder::source,
            { publications++ }, { errors.add(it.message ?: "") })
        pump.start(); advance(0)
        assertEquals(0, publications)
        advance(50)
        assertEquals(2, attempts); assertEquals(1, publications)
        assertEquals(listOf("session changing"), errors)
        pump.stop()
        val stoppedReads = reads
        advance(1000)
        assertEquals(stoppedReads, reads)
    }

    @Test fun restartPublishesAnEmptyNewSessionAndOfflineChangesDoNotReapplySettings() {
        var snapshot = source
        var applications = 0
        var bound = false
        val publications = mutableListOf<List<ReceiverSource>>()
        val binder = SourceControlBinder({ bound = false }, {}, { _, _, _ -> (!bound).also { bound = true } }, { applications++ })
        val pump = SourceControlPump(Handler(Looper.getMainLooper()), { snapshot }, binder::source,
            { publications.add(it) }, { throw it })
        binder.ready(); pump.start(); advance(0)
        snapshot = source.replace("true", "false"); advance(50)
        assertFalse(publications.last().single().online)
        assertEquals(1, applications)
        pump.stop(); snapshot = "[]"; binder.ready(); pump.start(); advance(0)
        assertTrue(publications.last().isEmpty())
        snapshot = source; advance(50)
        assertEquals(2, applications)
        pump.stop()
    }
}
