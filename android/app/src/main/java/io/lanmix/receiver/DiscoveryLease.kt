package io.lanmix.receiver

/** Screen/service ownership plus a bounded handoff for an explicitly requested connection. */
class DiscoveryLease {
    var screen = false
    var service = false
    var target: String? = null; private set
    private var requestedAt = 0L
    val needed get() = screen || service || target != null
    fun request(id: String, now: Long) { target = id; requestedAt = now }
    fun cancel() { target = null }
    fun expire(now: Long): Boolean {
        if (target == null || now - requestedAt < 12_000) return false
        cancel(); return true
    }
}
