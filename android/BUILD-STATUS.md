# Verified Android build

## 0.1.2 playback scheduling update · 2026-10-31

The user confirmed 0.1.1 playback became stable without the reported crackles or
dropouts, while perceived latency remained high. Android 0.1.2 optimizes output
queueing and UI diagnostics; desktop and native Rust package versions remain
0.1.1, with no DSP or network-policy change in this iteration.

- APK: `dist/android/LanMix-0.1.2-debug.apk` from the repository root.
- Package: `io.lanmix.receiver`, **0.1.2 / version code 3**.
- Size: **1,919,979 bytes**.
- SHA-256: `3CF9F53379172C393105101CA4AE3CD649666B69CEE4D7440C74F7857D8A8D78`.
- Both native ARM release libraries rebuilt using `./android/build.ps1 -Offline`.
  Final Kotlin/UI changes checked with `-SkipNative -Offline`.
- **10 Kotlin/JUnit tests passed**, zero failures/errors/skips; `assembleDebug`
  and `lintDebug` passed. Lint XML: **0 errors, 25 warnings**, no baseline.
- APK v2 signature and `zipalign -c -P 16 4` passed. `aapt2 dump badging` confirmed
  the version, API 26 minimum and API 37 target. The signer certificate matches
  the 0.1.1 debug APK, allowing an in-place update.
- Actual JVM `-Xcheck:jni` smoke passed with real UDP, audible float output,
  concurrent controls/diagnostics and same-port lifecycle checks. New coverage
  exercises 10 ms fills at 8/44.1/48/96/192 kHz.
- `cargo test --locked --workspace`: **41 tests passed** again.

The 60/100/30 ms modes request smaller effective AudioTrack buffers while
retaining system capacity for output-underrun recovery. Compatibility mode leaves
the platform limit untouched. Device clamping, route changes, resize failures,
and queue-counter wraparound are covered in host tests. These tests do not
instantiate physical Android AudioTrack hardware. No phone was operated by the
agent, and no real-device latency reduction or new-version continuity was
measured. See [implementation and validation](../docs/PLAYBACK-LATENCY.md).

The following sections preserve the checks and limitations recorded at those
earlier builds; the user's later stability confirmation is recorded above.

## 0.1.1 buffer continuity update · 2026-10-31

`./android/build.ps1 -Offline` rebuilt both ARM libraries, `assembleDebug`, and
`lintDebug` successfully. Version **0.1.1 / code 2**, package unchanged.

- APK size: **1,852,540 bytes**.
- SHA-256: `B62367661578BF35B24C3BD16BCE571744ACEB1980B29F6D864BAF9ADD257470`.
- Lint XML: **0 errors, 53 warnings**; no baseline hides findings.
- APK v2 signature and `zipalign -c -P 16 4` passed.
- Host JVM `-Xcheck:jni` passed again, including the actual new buffer diagnostic
  JSON fields, real UDP, rendering, concurrent controls and lifecycle checks.
- Workspace checks: strict Clippy, formatting and **41 passing tests**.

The change adds adaptive per-source buffering, buffer-aware network reordering,
and visible source buffer diagnostics. See [the investigation](../docs/AUDIO-DROPOUTS.md).
The user's recording was inspected, but this APK has not been installed or
auditioned on their physical phone by the agent. Physical-device improvement
still needs confirmation. The following section records the earlier APK only.

## Historical 0.1.0 build

Final APK built **2026-10-31 00:49:45 Asia/Shanghai**, after the shared core's
boxed-track constructor change and latest network fixes. Changes are confined
to `android/` and `crates/lanmix-android/`; root manifests were not edited.

* APK: `app/build/outputs/apk/debug/app-debug.apk`
* Package: `io.lanmix.receiver`, version 0.1.0 / code 1
* Size: **2,119,048 bytes**
* SHA-256: `F9C0575EA4B9C5E0F9133A6EE80C4D54128583D071034834F8D7DD951F556ED5`
* Minimum SDK 26; compiled/target SDK 37, confirmed with `aapt2 dump badging`.
* Manifest permissions: INTERNET and ACCESS_LOCAL_NETWORK. No microphone,
  multicast, or foreground-service permission.

## Final checks

`./android/build.ps1 -Offline` rebuilt both native release libraries and finished
`assembleDebug lintDebug` successfully. Lint: **0 errors, 48 warnings**, with no
baseline hiding errors. See `app/build/reports/lint-results-debug.html`; warnings
include localization, absent launcher icon, backup extraction policy, missing
ChromeOS x86_64 ABI, and a newer Gradle being available. Cached Gradle 9.6.1 was
used successfully with AGP 9.3.2, the installed API 37 platform, build tools 36,
and NDK 30.0.14904198.

The following libraries were extracted from this exact APK and checked:

| APK entry | Bytes | ELF architecture | Every LOAD alignment |
| --- | ---: | --- | --- |
| `lib/arm64-v8a/liblanmix_android.so` | 542,824 | ELF64 / AArch64 | 0x4000 / 16 KiB |
| `lib/armeabi-v7a/liblanmix_android.so` | 374,188 | ELF32 / ARM | 0x4000 / 16 KiB |

`llvm-nm --dynamic --defined-only` confirms all **six** expected JNI exports in
both libraries: nativeStart, nativeStop, nativeFill, nativeTracks, nativeSetTrack,
nativeSetMaster. `javap -s` confirms the Kotlin native method descriptors match.

`apksigner verify --verbose` passes with one Android debug signer and APK v2
signing. `zipalign -c -P 16 4` passes. No device installation is implied by these
packaging checks.

## Actual JVM/JNI smoke

`./android/verify-jni.ps1` rebuilt the Windows DLL against the latest shared core
and network, then ran Java with `-Xcheck:jni` and the normal JVM stack limit. It
passed real loopback UDP reception and source identity JSON, unknown-source mute
gating, restore to audible stereo float output (peak **0.03990524262189865**),
concurrent UI diagnostics/control updates and rendering, invalid array and slot
handling, rejection of a second fill worker, idempotent stop, maximum-size fill,
and same-port 48-to-96-kHz restart. All six native methods were actually called.

The earlier `hs_err_pid48108.log` recorded an **unoptimized native constructor
stack overflow** on the JVM's 1 MiB stack. Native startup now constructs and boxes
the mixer on a dedicated 8 MiB Rust initialization thread. The shared core also
boxes tracks and constructs them individually. The smoke passed after both
changes without increasing the JVM stack. The historical crash report is kept
under ignored `.verification/` for diagnosis.

## Runtime boundary

Physical Android playback, Android local-network permission interaction, USB /
Bluetooth / HDMI route changes, UI rendering, and hardware latency have **not**
been exercised on a device. No emulator or phone was installed, launched, or
controlled. This is the documented Activity-only blocking AudioTrack PCM_FLOAT
backend; background playback is not implemented. Source persistence uses source
IP and stream ID, and muted startup gates remain closed until the identified
source's own settings are restored. `set_muted` is the final native track-control
write.
