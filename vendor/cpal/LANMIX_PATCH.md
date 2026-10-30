# LanMix CPAL WASAPI capture patch

This directory vendors CPAL **0.16.0** from the local crates.io registry source.
Its upstream version, public API, copyright notices, and **Apache-2.0** license
are retained; see `LICENSE`, `Cargo.toml`, and the upstream `README.md`.
Registry cache marker files and the upstream lockfile are omitted. The global
Cargo registry cache is not modified.

Upstream issue: https://github.com/RustAudio/cpal/issues/1376

Microsoft flag contract:
https://learn.microsoft.com/en-us/windows/win32/api/audioclient/ne-audioclient-_audclnt_bufferflags

Only WASAPI input processing is patched. `run_input` preallocates a `Vec<u64>`
before priority boosting and before its event loop, sized with checked arithmetic
for `max_frames_in_buffer * bytes_per_frame`, rounded up to whole aligned words.
Allocation failure reports a stream error before capture starts processing.

For `AUDCLNT_BUFFERFLAGS_SILENT`, the callback receives the owned aligned silence
storage with the original packet sample count. The driver pointer is neither
read nor written, even if NULL, stale, or misaligned. Signed integer and float
silence use zero bits; unsigned PCM uses its midpoint (including U8 = 128).
Unflagged packets retain their original pointer and contents.

Selection performs no allocation or resizing. Oversized, NULL nonsilent, or
misaligned nonsilent packets are released and rejected using a preconstructed
error. Successful empty reads do not create a slice from NULL. Timestamp-error
exits also attempt to release an acquired packet. Ordinary callback completion
keeps the upstream ReleaseBuffer/error path. Existing upstream and application
error callbacks may have their own allocations; this patch does not claim to
make every error callback real-time safe.

Hardware-independent Windows unit tests cover SILENT with NULL/stale/invalid
driver pointers, aligned zero storage, combined flags, unchanged real data,
sample counts, oversized/misaligned rejection, and unsigned PCM silence.
These tests and compilation do not establish physical loopback runtime behavior.

The workspace owner enables this copy through `[patch.crates-io]` with
`cpal = { path = "vendor/cpal" }` in the workspace root manifest.

Validation completed on Windows:

- `cargo test --manifest-path vendor/cpal/Cargo.toml --lib --offline --target-dir <temporary-test-target>`:
  six tests passed, including all four patch tests; no device was opened.
- `cargo check -p lanmix-audio -p lanmix-sender`: passed using the workspace patch.
- `cargo test -p cpal --lib` from the workspace root is rejected by Cargo because
  CPAL is an excluded dependency with dev-dependencies. Use the explicit
  manifest command above instead. No workspace membership changes are needed.

Both successful commands retain two pre-existing upstream compiler warnings:
`NotSendSyncAcrossAllPlatforms` dead code and an elided `MutexGuard` lifetime.
