package io.lanmix.receiver

import org.junit.Assert.*
import org.junit.Test

class DiscoveryLeaseTest {
    @Test fun hiddenScreenRetainsExplicitConnectionDuringServiceHandoff() {
        val lease = DiscoveryLease()
        lease.screen = true; lease.request("pc", 100)
        lease.screen = false
        assertTrue(lease.needed)
        assertFalse(lease.expire(12_099))
        lease.service = true; lease.cancel()
        assertTrue(lease.needed)
        lease.service = false
        assertFalse(lease.needed)
    }
    @Test fun failedStartupExpiresEvenWithoutVisibleScreenOrService() {
        val lease = DiscoveryLease()
        lease.request("pc", 100)
        assertTrue(lease.expire(12_100))
        assertNull(lease.target)
        assertFalse(lease.needed)
        assertFalse(lease.expire(24_100))
    }
    @Test fun stopCancelsPendingConnectionWithoutStoppingVisibleDiscovery() {
        val lease = DiscoveryLease()
        lease.screen = true; lease.request("pc", 0); lease.cancel()
        assertNull(lease.target)
        assertTrue(lease.needed)
        lease.screen = false
        assertFalse(lease.needed)
    }
    @Test fun newerSelectionGetsItsOwnDeadlineAndCannotConnectOldTarget() {
        val lease = DiscoveryLease()
        lease.request("old", 0); lease.request("new", 10_000)
        assertFalse(lease.expire(12_000))
        assertEquals("new", lease.target)
        assertTrue(lease.expire(22_000))
        assertFalse(lease.needed)
    }
}
