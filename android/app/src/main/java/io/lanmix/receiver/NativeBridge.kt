package io.lanmix.receiver

/** Stereo, interleaved PCM float. JNI methods throw on invalid input or I/O failure. */
object NativeBridge {
    val loadError: String? = try {
        System.loadLibrary("lanmix_android")
        null
    } catch (e: UnsatisfiedLinkError) {
        e.message ?: "Native library unavailable"
    }

    external fun nativeStart(port: Int, bufferMs: Int, sampleRate: Int)
    external fun nativeStop()
    external fun nativeFill(array: FloatArray)
    /** JSON object: {tracks: TrackSnapshot[], stats: NetworkStats}. */
    external fun nativeTracks(): String
    external fun nativeSetTrack(index: Int, gainDb: Float, mute: Boolean, solo: Boolean,
        eqEnabled: Boolean, preampDb: Float, bands: FloatArray)
    external fun nativeSetMaster(gainDb: Float, eqEnabled: Boolean, preampDb: Float, bands: FloatArray)
}
