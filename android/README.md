# LanMix native Android receiver

Native Kotlin Activity, no external UI libraries, minimum Android 8 / API 26.
Build and target API 37, the newest stable platform installed on this host (its
`source.properties` reports Android 17, preview SDK 0, no beta codename). Android
Gradle plugin 9.3.2 includes Kotlin support; cached Gradle 9.6.1 and Android
Studio's bundled JDK are used. The installed NDK is 30.0.14904198 (r30-beta1).

## Build

From PowerShell, run `./android/build.ps1` from the repository root. It builds
`lanmix-android` for arm64-v8a and armeabi-v7a with the installed API-26 NDK
linkers, stages the libraries, and runs `assembleDebug lintDebug`. No root
manifest changes or Gradle installation are required. `-Offline` uses cached
dependencies; `-SkipNative` reuses previously staged libraries. Gradle's preBuild
fails if either native library is absent, preventing UI-only APKs.

The output is `app/build/outputs/apk/debug/app-debug.apk`. The project also has a
standard Gradle wrapper for machines without a cached distribution. `ANDROID_HOME`
and, optionally, `JAVA_HOME` select local installations. Required Rust targets
must already be installed. The script does not install or launch devices.

## Receive audio

Open the Activity, allow local network access on Android 17+, and press Start.
Send LanMix unicast UDP to the displayed IPv4 address, default port **46210** and
network jitter buffer **60 ms** (adjustable 20–200 ms). INTERNET is declared;
target API 37 also uses ACCESS_LOCAL_NETWORK and requests its runtime grant.
There is no microphone, AudioRecord, multicast permission, or multicast lock.

This version intentionally runs only while the Activity is visible. `onStop`
closes audio and UDP. The screen stays awake while receiving; there is no
foreground service or background audio support. Audio focus loss stops playback
and requires tapping Start again. Settings survive a restart; the receiver does
not automatically resume.

Output selection calls `AudioTrack.setPreferredDevice`. Android may choose a
different route. The UI separately shows all `AudioManager.getDevices(OUTPUTS)`
declarations, API-31+ audio profiles, encodings, sample rates, channel counts and
masks, and the **actual** AudioTrack routed device, sample rate, buffer size, and
underrun count. Empty capability arrays mean unspecified. A requested format or
advertised capability does not prove a physical DAC format, bit-perfect playback,
or measured latency. Bluetooth/USB/HDMI downstream conversion is outside these
observations. Rate options are explicit requests and may fail on a device.

Auto rate uses Android's native output rate if compatible with the selected
device's report, otherwise 48 kHz or the nearest reported supported rate. Route
changes stop/pause/flush the old stream, join its worker, release it and the
native receiver, then build a new stream and mixer for the new rate. If the
worker cannot stop within three seconds, replacement is refused. Routes can
briefly interrupt playback; this is not gapless handover.

## Audio/JNI design

The v0.1 backend is a dedicated Java/Kotlin **blocking AudioTrack writer**, not an
Oboe/AAudio real-time callback. It requests THREAD_PRIORITY_AUDIO, MODE_STREAM,
stereo ENCODING_PCM_FLOAT. It does not require PERFORMANCE_MODE_LOW_LATENCY.
The playback buffer is separate from the network jitter buffer. Every fill uses
one preallocated Java float[2048], handles partial writes, and renders 1024
frames. PCM uses SetFloatArrayRegion's deliberate array copy. There is no
GetPrimitiveArrayCritical pinning.

`NativeBridge` (`io.lanmix.receiver`) loads `liblanmix_android.so` and exposes:

* `nativeStart(port, bufferMs, sampleRate)` binds `0.0.0.0:port` using
  `lanmix_network::Receiver::bind_with_muted_sources`, throwing IllegalStateException on failure.
* `nativeFill(float[])` writes interleaved stereo. Arrays must have even length
  2–8192; larger/odd/empty arrays are rejected. Native frame and interleave
  scratch buffers are allocated once at start (4096 frames maximum).
* `nativeStop()` drops the network receiver and joins its worker; it is idempotent.
* `nativeTracks()` returns JSON `{running, tracks, stats, meters, master}`. `tracks`
  and `stats` are the network's serialized snapshots. Track identity uses `slot`,
  `source`, `stream_id`, and `session_id`. Meters add peak/underrun and master
  clipping counters. Diagnostics allocate on the UI polling path only.
* `nativeSetTrack(index, gainDb, mute, solo, eqEnabled, preampDb, float[10])`.
* `nativeSetMaster(gainDb, eqEnabled, preampDb, float[10])`. Gain is decibels.

The first fill transfers the prepared mixer into that worker's thread-local
storage. Subsequent fills take **no global mutex**. UI diagnostics and atomic
control setters use an independent receiver lifecycle mutex and never lock the
render path. Start/stop use a one-time startup handoff; epochs invalidate cached
mixer state across restarts. Kotlin joins the old worker before exposing a new
session. All fills for a session must occur on
the same worker. Panic/argument failures become JVM exceptions, never unwind
across JNI. The same Rust code and exports compile on Windows for host checking
and JNI smoke tests.

Mixer construction runs on a short-lived Rust initialization thread with an
explicit 8 MiB stack, then transfers a boxed mixer. This avoids overflowing the
JVM's smaller native stack during construction, including unoptimized builds.
`./android/verify-jni.ps1` tests the actual host DLL under `-Xcheck:jni`: real UDP,
new-source mute gating, audible float output, controls/JSON concurrent with
rendering, invalid arrays, single-worker ownership, and a 48-to-96-kHz restart.

The UI provides 16 source slots, each with gain, mute, solo, independent EQ enable,
preamp and ten bands (31.25, 62.5, 125, 250, 500 Hz, 1, 2, 4, 8, 16 kHz), plus master
gain/preamp/EQ. Controls are persisted by source IP and stream ID and rebound
when a slot changes identity. New identities remain muted in the receiver until
the first snapshot loads their own saved controls; only then is the saved mute
applied. Master alone is applied at startup. Rebuilds clear slot bindings so all
identified controls are reapplied; same-identity reconnects keep their controls.
Master defaults to −6 dB. Source rate, packet/loss/late/overflow/underrun diagnostics remain separate
from the AudioTrack output rate.
