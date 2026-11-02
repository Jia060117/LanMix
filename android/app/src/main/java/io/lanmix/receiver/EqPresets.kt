package io.lanmix.receiver

/** Starting points for taste, with headroom; never change the listening volume. */
enum class EqPreset(val label: String, val preamp: Float, vararg val gains: Float) {
    FLAT("平直", 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f),
    BASS("低音增强", -8f, 6f, 5f, 4f, 2f, 0f, -1f, -1f, 0f, 0f, 0f),
    VOCAL("人声", -5f, -3f, -2f, -1f, 0f, 2f, 3f, 3f, 2f, 0f, -1f),
    POP("流行", -5f, 2f, 3f, 2f, 0f, -1f, 1f, 2f, 3f, 2f, 1f),
    ROCK("摇滚", -6f, 4f, 3f, 1f, -1f, -2f, 1f, 3f, 4f, 4f, 3f),
    CLASSICAL("古典", -5f, 2f, 1f, 0f, 0f, -1f, -1f, 0f, 2f, 3f, 3f);

    fun applyTo(control: MixControl) {
        gains.copyInto(control.bands)
        control.preampDb = preamp
        control.eqEnabled = true
    }

    companion object {
        fun matching(control: MixControl): EqPreset? = entries.find {
            it.gains.contentEquals(control.bands) && it.preamp == control.preampDb
        }
    }
}
