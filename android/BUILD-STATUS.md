# Verified Android build

## 1.0.1 tuning UI · 2026-11-02

- Package `io.lanmix.receiver`, **1.0.1 / version code 11**, minimum API 26 / target 37, non-debuggable release.
- Local artifact: `dist/android/LanMix-1.0.1.apk`, **2,165,709 bytes**. This build does not replace the published v1.0.0 Release.
- SHA-256: `05020a20629ef5d0af299bd085a7d5ef2c3d9fbf72e4d120f7ef92e13a4db430`.
- `android/build.ps1 -SkipNative -Release -Offline`: Debug and Release each passed **57 Kotlin tests**, assembly and lint. Lint: debug **38 warnings**, release **37 warnings**, both **0 errors**.
- Single volume/EQ editor with master/source selection; six presets, horizontal ten-band faders, gain-curve and touch animations, and advanced preamp. Tests cover preset isolation/persistence, bypass retention, offline/reconnect settings, recycled source slots and stale gestures, animation callbacks, accessibility, and three screen widths/font scales.
- Deep/light home and two-source EQ previews rendered and inspected with native Skia. Existing real AGSL compilation assertion remains in the preview test; software previews do not establish physical GPU appearance.
- APK v2 signature and 16 KiB ZIP alignment verified. Publisher certificate is unchanged: `CN=Jia060117, OU=LanMix, O=Jia060117`, SHA-256 `d84eb2c373f642cc1716b92d55c5fb65a903932dd4a264c1fd91a954105e44ed`. Supports upgrading the official 1.0.0 installation in place.
- Both packaged ARM64/ARMv7 JNI libraries are byte-identical to official 1.0.0. All six packaged license/notice assets are unchanged. No DSP, transport, native playback or Windows change.
- No USB phone is connected. This iteration does not claim physical installation, GPU appearance, performance or listening acceptance.

## Official 1.0.0 · 2026-11-01

- Package `io.lanmix.receiver`, **1.0.0 / version code 10**, minimum API 26 / target 37, non-debuggable release.
- Artifact: `LanMix-1.0.0-android.apk`, **2,149,325 bytes**.
- SHA-256: `f6cb72bf2da6e36ea3ad5a88a89b4b409930fc830f628efcf13cc9ae9dbc0292`.
- Full dual-ABI Rust rebuild; Debug and Release each passed **36 Kotlin tests**, assembly and lint. Lint: debug 37 warnings, release 36 warnings, both 0 errors.
- Publisher certificate: `CN=Jia060117, OU=LanMix, O=Jia060117`, RSA 3072. SHA-256: `d84eb2c373f642cc1716b92d55c5fb65a903932dd4a264c1fd91a954105e44ed`. Dedicated private key stays outside the repository; old debug installs need uninstalling before migration.
- APK v2 signature verification passed. ZIP 16 KiB alignment and all ARM64 / ARMv7 ELF LOAD alignments passed. Packaged libraries match Gradle's stripped outputs, and the inputs match the fresh Rust build.
- Project and third-party licenses are included in APK assets and checked against the source files.
- Host JVM `-Xcheck:jni` passed all eleven exports, actual UDP discovery/control, mixed-rate fills and loss/repair experiments. No USB device is connected; this release round does not add physical install, GPU or listening validation.

## Previous 0.2.2 build · 2026-11-01

- Package `io.lanmix.receiver`, **0.2.2 / version code 9**, minimum API 26 / target 37.
- Artifact: `dist/android/LanMix-0.2.2-debug.apk`, **2,402,709 bytes**.
- SHA-256: `90DEACFDF9C943870D455E74C34F7818B0F4E4B29C1383FDF7EA4A1FCEB2F548`.
- `android/build.ps1 -SkipNative -Offline`: **36 Kotlin tests**, APK assembly and
  lint passed (**0 errors / 37 warnings**). Both packaged ARM libraries are
  byte-identical to 0.2.1. Existing debug certificate matches; APK signature and
  16 KiB ZIP alignment passed. Windows remains 0.2.0.
- Navigation uses explicit centered icon/label groups with a 3 dp gap, a 336 dp
  width cap and a 64 dp default height that grows with font size. Layout tests
  cover three widths and 1.0 / 1.3 / 2.0 font scale on API 28 / 32 / 35, plus
  clicks, accessibility selection and the existing startup/recreation regression.
- Glass tint and selection are lighter; 1.5 dp blur preserves background detail.
  The capsule lens has a narrow highlight and padded backdrop sampling. Native
  Skia API 35 compiles the actual runtime shader and asserts successful creation.
- Light/dark idle-fixture layout previews were rendered and inspected. Their
  software canvas uses the translucent fallback, so they do not establish GPU
  refraction appearance. No USB device was connected; no install, physical GPU,
  listening or performance acceptance is claimed this iteration.

## 0.2.1 startup repair · 2026-11-01

- Package `io.lanmix.receiver`, **0.2.1 / version code 8**, minimum API 26 / target 37.
- Artifact: `dist/android/LanMix-0.2.1-debug.apk`, **2,303,565 bytes**.
- SHA-256: `943AB30D3D0FA738F812C4467071C29EF6AE33B87D1CC3EA6D21AAFFE109DF95`.
- User's Android 17 / API 37 crash log identifies a `NullPointerException` inside
  `PhoneWindow.getInsetsController`, called by `MainActivity.createUi` before
  content/decor creation. The nullable Kotlin call did not protect the getter's
  internal dereference. Complete the controls and install content first, then set
  system-bar appearance and initial page selection.
- Installed with `adb install -r` on the connected Android 17 phone: **Success**.
  `am start -W` reported **Status: ok / COLD**, total time 700 ms. The same app
  process remained alive and resumed several minutes later, with no crash entries
  for that process. No root, data clearing, or permission changes used.
- ARM64 and ARMv7 native libraries are byte-identical to 0.2.0. APK signature
  matches the existing debug certificate; 16 KiB ZIP alignment passed.
- **29 Kotlin tests** passed, including actual Activity creation, software-canvas
  drawing, all three tabs and recreation under Robolectric API 28 / 32 / 35.
  A negative check using the original 0.2.0 Activity fails on API 32 and 35 with
  the same `PhoneWindow.getInsetsController` NPE, and passes after restoring the
  fix. API 28 never enters the affected API-30+ system-bar branch.
- APK assembly and lint passed: **0 errors / 37 warnings**. These runtime tests
  use test-only Robolectric dependencies; the APK adds no UI/runtime dependency.

This establishes startup on the connected phone, not a new audio-quality,
background, Wi-Fi or Bluetooth acceptance test.

## 0.2.0 UI/discovery build · 2026-11-01

- Package `io.lanmix.receiver`, **0.2.0 / version code 7**, minimum API 26 / target 37.
- Artifact: `dist/android/LanMix-0.2.0-debug.apk`, **2,826,894 bytes**.
- SHA-256: `9D92921B25EA2F767973AD37B03F166A76F030C75FC5BCEAE0AF78FD9678871D`.
- Both ARM JNI libraries rebuilt offline; Kotlin tests, APK assembly and lint passed.
  **26 Kotlin tests**, no failures/errors; lint **0 errors, 37 warnings**, kept visible.
- APK v2 signature, 16 KiB ZIP alignment, version, permissions and launcher icon
  checked. Debug signing certificate SHA-256 remains
  `d3d92a78d02feadacf7bccbae4bd2e783422c809213d4cd36d6880a5c08657e4`.
- Rust workspace tests, formatting and strict Clippy passed, including four discovery
  tests. No DSP, PCM transport or AudioTrack policy change in this iteration.
- Actual host JVM `-Xcheck:jni` checked all eleven exports: discovery identity/update,
  actual UDP probe/reply, snapshot, connection request, shutdown, plus the existing
  audio lifetime, loss repair, controls and supported-rate checks.
- Real Windows process discovery and bidirectional probe replies passed independently
  of PCM. Two-source 44.1/96 kHz smoke and deliberate loss/backup tests also passed.

The Activity now has Mixer / Devices / Settings pages and a native floating glass
navigation bar, with light/dark colors and API-dependent blur/lens fallbacks.
A process controller keeps discovery alive with either the screen or playback
service and releases it when neither is active. A pending explicit connection holds
a bounded twelve-second handoff, covered by four new lifecycle tests. It does not change audio focus,
background playback ownership or jitter buffer defaults.

**No desktop/phone UI automation or physical Android/Wi-Fi/Bluetooth acceptance was
performed this iteration.** In particular shader appearance/performance, actual
button flows on phones, background/lockscreen and multi-app behavior need device
acceptance. See [UI/discovery design and instructions](../docs/UI-DISCOVERY.md).

## 0.1.5 background ownership and shared playback · 2026-10-31

- Package `io.lanmix.receiver`, **0.1.5 / version code 6**, minimum API 26 / target 37.
- Artifact: `dist/android/LanMix-0.1.5-debug.apk`, **1,968,028 bytes**.
- SHA-256: `0F608A8F3AF1920064665BF9290E77607A2AC9ECAE0642ABEDC63C000ED023D5`.
- `./android/build.ps1 -SkipNative -Offline` passed tests, assemble and lint.
  **22 Kotlin tests**, 0 failures/errors; lint **0 errors, 35 warnings** (31
  diagnostic/UI string localization plus existing icon, extraction, ChromeOS ABI
  and Gradle-version warnings). No warning baseline or new lint suppression.
- APK v2 signature, 16 KiB ZIP alignment, version and declared permissions checked.
  Signing certificate matches 0.1.4 for an in-place update.
- ARM64 and ARMv7 libraries extracted from the final APK are byte-identical to
  the 0.1.4 APK. Rust/PCM protocol/DSP and Windows tools were not changed.
- Actual host `-Xcheck:jni` passed again: real UDP source gating, float output,
  concurrent controls, lifetime loss diagnostics, missing-original repair,
  ownership and supported-rate fills. This does not exercise Android services.
- Four new control-binding tests cover joining without UI, saved per-source
  gain, preserving live controls across polling, slot reuse, reset, invalid slots,
  and retry after a temporary native-control failure.

The visible Activity now starts a non-exported `mediaPlayback` foreground service.
Home/lockscreen/Activity destruction no longer request playback stop. The service
owns rendering, UDP, source initialization, route removal and diagnostics; ordinary
silent notification actions return to controls or stop. Timed CPU wake-lock
leases renew while receiving and are released on cleanup. Audio focus requests
and MediaSession ownership are absent; the PCM backend remains shared AudioTrack.
Service engine cleanup/opening is serialized across instances, and an active
old writer prevents replacement. No boot start or sticky process restart.

**Physical Android background/lockscreen, notification interaction and multi-app
audio were not exercised by the agent.** OEM power/routing/exclusive-playback
policies may still interrupt audio. See [behavior and pending device acceptance
steps](../docs/ANDROID-BACKGROUND.md). Windows sender remains 0.1.4.

## 0.1.4 loss classification and delayed PCM backups · 2026-10-31

- Package `io.lanmix.receiver`, **0.1.4 / version code 5**, minimum API 26 / target 37.
- Artifact: `dist/android/LanMix-0.1.4-debug.apk`, **1,888,912 bytes**.
- SHA-256: `79562D5EB2412A943959DDBF8822CF24315E5E1147315CD46E10D4F49095DD6C`.
- Both ARM native libraries rebuilt with `./android/build.ps1 -Offline`.
  Kotlin tests, assemble and lint passed: **18 tests, 0 errors, 33 lint warnings**.
  Warnings remain visible, including diagnostic UI text localization.
- APK v2 signature, `zipalign -c -P 16 4`, and `aapt2` version checks passed.
  Certificate SHA-256 matches 0.1.3, allowing an in-place update.
- **49 Rust tests**, formatting and strict Clippy passed. No DSP/CPAL changes.
- Actual host JNI/UDP test repaired three missing originals with 126 duplicate
  datagrams discarded and zero consecutive near-silent frames in the tested PCM.
- Actual CLI sender -> UDP loss proxy -> headless receiver: two three-packet
  loss bursts gave **576 concealed frames with repair off, 0 with repair on**.
  1490 delayed copies were byte-identical; median observed delay was 21.63 ms.
- Two-source 44.1/96 kHz process smoke test passed with backups enabled:
  zero concealed/late/overflow frames or clipping; finite non-silent mix.
  Sources stop two seconds before receiver end; each records one terminal
  starvation, which is expected after the test senders intentionally exit.
- Windows sender and receiver report **0.1.4**; packaged in
  `dist/LanMix-0.1.4-windows-x64.zip` (3,057,514 bytes at this check).
  Native shared crate versions remain 0.1.1; protocol version remains v1.

The supplied 0.1.3 diagnostic proves source-time concealment occurred despite
zero underruns, not its exact cause or a physical-device cure. The new software
separates sender timeline gaps from unreceived sequence numbers and attempts
loss recovery without raising buffer targets. It doubles PCM traffic by default;
`--repair-ms 0` disables copies. Physical Android/Bluetooth playback is untested
by the agent. See [investigation and limits](../docs/PLAYBACK-DROPOUTS.md).

## 0.1.3 dropout investigation and safe playback default · 2026-10-31

- Package `io.lanmix.receiver`, **0.1.3 / version code 4**, API 26 minimum / 37 target.
- Artifact: `dist/android/LanMix-0.1.3-debug.apk` from the repository root.
- Size: **1,934,905 bytes**.
- SHA-256: `A3065D9D94957988E44E11F5C404BC4DA8B5605E365E980B9F6A070926151B88`.
- Both ARM release libraries rebuilt with `./android/build.ps1 -Offline`; final
  Kotlin changes checked with `-SkipNative -Offline`. Unit tests, assemble and lint passed.
- **18 Kotlin tests passed**, no failures/errors; lint **0 errors, 28 warnings**.
  Warnings concern string localization, icon, extraction policy, ChromeOS ABI and
  Gradle version; no baseline suppresses findings.
- APK v2 signature and `zipalign -c -P 16 4` passed; version verified by `aapt2`.
  The signing certificate matches the 0.1.1 APK for an in-place upgrade.
- **42 Rust tests**, formatting and strict workspace Clippy passed. Existing
  vendored CPAL compiler warnings remain; no CPAL modification in this iteration.
- Actual host JVM `-Xcheck:jni` passed. Intentional real UDP loss of **700 frames**
  produced **597 near-silent output frames** at 48 kHz while mixer underruns stayed
  zero. Cumulative loss diagnostics survived a sender-session reset. Also passed
  gated-to-audible output, concurrent controls, ownership, validation and rate restarts.

Stable mode now retains system buffer capacity; compatibility mode additionally
restores 1024-frame writes. Experimental tuning preserves device-accepted floors
and skips unchanged resizing. Same-rate route changes retain the receiver.
The UI copies raw source diagnostics, persistent-in-Activity history, timing and
near-silence observations, with a manual dropout marker. See
[findings and limitations](../docs/PLAYBACK-DROPOUTS.md).

The user's physical 0.1.2 issue has not been reproduced on Android hardware by
the agent. Passing host checks proves neither a final root cause nor that 0.1.3
is free of physical-device dropouts. Earlier build records follow.

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
