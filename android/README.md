# LanMix native Android receiver

Current Android package: **1.0.1 / version code 11**, compatible with the 1.0.0 Windows
sender. The published GitHub Release remains 1.0.0. Official APKs use Jia060117's dedicated release certificate; old debug
builds require uninstalling before migration. Release signing, dependencies and
build instructions: [BUILDING.md](../docs/BUILDING.md). The historical UI notes
below describe the preceding iterations. Default 20 ms byte-identical backups
and the v1 protocol are unchanged.

1.0.1 replaces repeated master/source controls with one shared tuning card. Select
Overall or a source, then use the same volume slider and expandable EQ editor.
Source mute/solo remains independent. The ten vertical faders run horizontally
from low to high, with scrolling for narrow screens or enlarged text. Six presets
(Flat, Bass, Vocal, Pop, Rock, Classical) retain volume/mute/solo and include
preamp headroom; manual changes remain custom. Preamp is under Advanced. Preset
transitions, fader touch halos and expansion feedback respect disabled animations
and never send per-animation-frame DSP updates. Meters and detailed diagnostics
live in Settings; the receive address lives in Devices. Existing JSON settings
remain compatible. Audio/native libraries are byte-identical to official 1.0.0.
Debug/Release each pass 57 tests; both themes have rendered layout previews.
Physical installation, GPU appearance and listening validation remain pending.

Retain a 150 ms network buffer during investigation. New source diagnostics
classify sender timeline gaps versus unreceived sequences (including send errors),
count gap fillers/duplicates, and show the latest loss age. Cumulative evidence
survives source resets. Clipboard output puts source/marker/history before device
capabilities and omits unused meter slots. See
[diagnostic interpretation and recovery limits](../docs/PLAYBACK-DROPOUTS.md).

Native Kotlin Activity and playback foreground service, no external UI libraries,
minimum Android 8 / API 26.
0.2.2 groups each navigation icon and label with a 3 dp gap and centers the group,
caps the floating bar at 336 dp, and accommodates enlarged text. The glass uses
a clear tint, light blur, edge refraction and a lighter selection highlight.
Page headings and card spacing are more compact. Both themes were rendered for
layout review; native Skia verifies shader compilation. A physical GPU appearance
check is still pending because no USB device was connected this iteration.
The packaged ARM audio libraries are byte-identical to 0.2.1.
0.2.1 fixes a startup NPE in 0.2.0: system-bar appearance is now set only after
the complete content view creates the window decor. Confirmed by the Android 17
device crash stack and an in-place update/cold launch. Audio JNI libraries are
byte-identical to 0.2.0.
Build and target API 37, the newest stable platform installed on this host (its
`source.properties` reports Android 17, preview SDK 0, no beta codename). Android
Gradle plugin 9.3.2 includes Kotlin support; cached Gradle 9.6.1 and Android
JDK 21 are used. The installed NDK is 30.0.14904198 (r30-beta1).

## Build

From PowerShell, run `./android/build.ps1` from the repository root. It builds
`lanmix-android` for arm64-v8a and armeabi-v7a with the installed API-26 NDK
linkers, stages the libraries, and runs `testDebugUnitTest assembleDebug lintDebug`. No root
manifest changes or Gradle installation are required. `-Offline` uses cached
dependencies; `-SkipNative` reuses previously staged libraries. Gradle's preBuild
fails if either native library is absent, preventing UI-only APKs.

Activity startup tests use Robolectric 4.17 with Android resources. The first test
run also downloads its Android runtime jars into the local Maven cache; Gradle's
`--offline` alone does not prevent Robolectric's separate runtime downloads.

The output is `app/build/outputs/apk/debug/app-debug.apk`. The project also has a
standard Gradle wrapper for machines without a cached distribution. `ANDROID_HOME`
and, optionally, `JAVA_HOME` select local installations. Required Rust targets
must already be installed. The script does not install or launch devices.

## Interface and device discovery

The Activity has Mixer / Devices / Settings pages, follows system light/dark mode,
and uses an independently implemented native floating glass bar. Android 13+ tries
an AGSL edge lens over background blur, Android 12 uses blur, and older devices
use a translucent gradient. Shader creation failure falls back to blur.
Per-source volume/EQ, actual output, capability reports, service ownership and
copyable diagnostics remain available. See [UI and connection guide](../docs/UI-DISCOVERY.md).

Open `lanmix.exe` on Windows, then choose that PC on the Android Devices page.
The receiver starts locally and requests streaming only after its PCM output is
ready. Alternatively start reception first, then choose the phone from Windows.
Discovery uses its own Rust worker and UDP 46211 plus a dynamic control port;
PCM still uses UDP 46210 by default. Devices expire after seven seconds without
announcements. The discovery worker remains alive with the service or visible
Activity, then stops when neither owns it. No multicast lock is required.

## Receive audio

Open the Activity, allow local network access on Android 17+, and press 开始接收.
Send LanMix unicast UDP to the displayed IPv4 address, default port **46210** and
initial network jitter buffer **150 ms** for new installs (adjustable 20–200 ms).
Existing saved values are retained; after upgrading, start with 150 ms for
Wi-Fi if a smaller setting was previously used. Each starving source increases
its own target up to 400 ms until a session/rate reset. Source buffer occupancy,
adaptive target, and underrun count appear in Settings diagnostics and each
source card’s expandable status. These source underruns are separate from AudioTrack underruns.
INTERNET is declared;
target API 37 also uses ACCESS_LOCAL_NETWORK and requests its runtime grant.
There is no microphone, AudioRecord, multicast permission, or multicast lock.

The user starts a `mediaPlayback` foreground service from the visible Activity.
The service owns UDP/audio, source-control binding and diagnostics. Activity
`onStop`/`onDestroy`, screen locking and removing the Activity task no longer
request a stop. A low-importance, silent service notification opens controls or
stops reception; there is no MediaSession or media-button takeover. The display
may sleep normally. A renewable partial CPU wake-lock lease expires after two
minutes unless renewed each minute, and is released on stop/error/destruction.
Android 13+ notification permission is requested once on user start; denial does
not prevent a foreground service, but can hide its notification-drawer controls.
The application can always stop it from its own UI. `START_NOT_STICKY` avoids
unexpected playback after system process termination; no boot receiver is added.

Playback intentionally does not request audio focus or change system music
volume/communication mode. Raw PCM continues through the shared AudioTrack
mixer alongside other applications when device/app policy permits. This is an
explicit user-requested concurrent-audio tool, not a claim that every OEM or
exclusive player supports simultaneous playback. The backend remains AudioTrack,
not MediaPlayer/ExoPlayer or an exclusive AAudio stream. Background/lockscreen,
calls and multi-app behavior still need physical-device tests; see
[scope and test procedure](../docs/ANDROID-BACKGROUND.md).

Output selection calls `AudioTrack.setPreferredDevice`. Android may choose a
different route. The UI separately shows all `AudioManager.getDevices(OUTPUTS)`
declarations, API-31+ audio profiles, encodings, sample rates, channel counts and
masks, and the **actual** AudioTrack routed device, sample rate, buffer size, and
underrun count. Empty capability arrays mean unspecified. A requested format or
advertised capability does not prove a physical DAC format, bit-perfect playback,
or measured latency. Bluetooth/USB/HDMI downstream conversion is outside these
observations. Rate options are explicit requests and may fail on a device.

Auto rate uses Android's native output rate if compatible with the selected
device's report, otherwise 48 kHz or the nearest reported supported rate. In
0.1.3, routing is confirmed after 200 ms; duplicate or transient-null notifications
do not rebuild reception. Same-rate route changes retain AudioTrack and the native
receiver. A changed automatic rate still stops/joins/releases the old stream and
builds a new mixer. Removal of an explicitly selected output also rebuilds once.
Adding unrelated outputs no longer restarts UDP. If the worker cannot stop within
three seconds, replacement is refused. Hardware routing can still interrupt playback.

## Audio/JNI design

The v0.1 backend is a dedicated Java/Kotlin **blocking AudioTrack writer**, not an
Oboe/AAudio real-time callback. It requests THREAD_PRIORITY_AUDIO, MODE_STREAM,
stereo ENCODING_PCM_FLOAT. It does not require PERFORMANCE_MODE_LOW_LATENCY.
The playback buffer is separate from the network jitter buffer. Every fill uses
one preallocated Java float array, handles partial writes, and renders approximately
10 ms (480 stereo frames at 48 kHz, 441 at 44.1 kHz), except the compatibility
baseline which uses the previous 1024-frame block. PCM uses
SetFloatArrayRegion's deliberate array copy. There is no
GetPrimitiveArrayCritical pinning.

### Playback scheduling in Android 0.1.3

Following the user's 0.1.2 dropout report, 稳定优先 is now the default and does
not reduce the platform buffer. 兼容基线 also retains that buffer and restores
0.1.1's 1024-frame render/write block. On the first launch of this policy revision,
the saved playback mode is reset once to stable; volume, EQ, rate and network
settings are retained. Later mode changes remain saved.

均衡 (60 ms requested) and 响应优先 (30 ms requested) remain experimental.
Their original allocated capacity is retained for recovery. Android may clamp
requests; the accepted safety floor is now retained after routing, and identical
limits do not trigger repeated resize calls. The UI distinguishes effective size
from capacity. These modes change neither source buffering nor PCM/EQ processing.

The writer observes cumulative output underruns about every 500 ms. A new
underrun grows the effective buffer by 50%, at least 10 ms; multiple new events
double it, capped at platform capacity. It never shrinks an adapted target
within a session. A newly confirmed route reapplies the learned limit on the writer.
Failed resizing
attempts to restore full capacity and disables further tuning until restart.

The displayed queue estimate uses accepted stereo samples minus Android's
unsigned playback-head counter, accounting for partial writes and wraparound.
Inconsistent counters display unavailable. This is diagnostic only and never
paces rendering; it does not measure downstream Bluetooth or end-to-end delay.
Mode and requested sample rate are saved. Changing mode restarts reception.
The UI has Chinese controls, separately identified network/output buffer health,
and user-operated mark-dropout and copy-diagnostics buttons. A bounded in-memory
history survives receiver and Activity rebuilds within the process. It includes
source session/reset changes, lifetime missing-frame concealment, output underruns,
and near-silence episodes of at least 2 ms observed before AudioTrack writes.
PCM silence also includes natural silence, mute and startup, so it is not an error
verdict. Numeric writer metrics include render/write/fill-gap maxima, zero writes,
and silence totals for the latest reporting window; string formatting runs on the
UI handler. Native JSON and its sample time are included when copying; 0.1.4
omits only unused meter slots and puts device capabilities last.
See [dropout investigation](../docs/PLAYBACK-DROPOUTS.md).

`NativeBridge` (`io.lanmix.receiver`) loads `liblanmix_android.so` and exposes:

* `nativeStart(port, bufferMs, sampleRate)` binds `0.0.0.0:port` using
  `lanmix_network::Receiver::bind_with_muted_sources`, throwing IllegalStateException on failure.
* `nativeFill(float[])` writes interleaved stereo. Arrays must have even length
  2–8192; larger/odd/empty arrays are rejected. Native frame and interleave
  scratch buffers are allocated once at start (4096 frames maximum).
* `nativeStop()` drops the network receiver and joins its worker; it is idempotent.
* `nativeTracks()` returns JSON `{running, tracks, stats, meters, master}`. `tracks`
  and `stats` are the network's serialized snapshots. Track identity uses `slot`,
  `source`, `stream_id`, and `session_id`. Tracks also report `reset_count`,
  `reset_reason`, `concealed_frames`, and `loss_events`; the latter totals survive
  timeline/session resets for the same source identity, until receiver restart or
  slot reassignment. Initial connection counts as one reset. Meters add peak/underrun and master
  clipping counters. Meters also report `buffered_ms`, `target_buffer_ms`,
  `primed`, and `drift_ppm`. Diagnostics allocate on the service's main-thread
  polling path, outside the PCM writer, even when the Activity is hidden.
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
