package io.lanmix.receiver

/** Runs in the service even when no Activity is attached. New sources start muted
 * in Rust and must receive their own saved settings before becoming audible. */
class SourceControlBinder(
    private val resetBindings: () -> Unit,
    private val applyMaster: () -> Unit,
    private val bind: (Int, String, Long) -> Boolean,
    private val applyTrack: (Int) -> Unit
) {
    private val pending = mutableSetOf<Int>()
    fun ready() { pending.clear(); resetBindings(); applyMaster() }
    fun source(slot: Int, address: String, stream: Long) {
        if (slot !in 0..15) return
        if (bind(slot, address, stream)) pending.add(slot)
        if (slot in pending) {
            // Binding updates model identity first; if JNI is temporarily
            // unavailable, retry next poll instead of leaving this source muted.
            applyTrack(slot)
            pending.remove(slot)
        }
    }
}
