# lanmix-core

Pure Rust stereo DSP. The only dependency is `rtrb = "0.3"`.

`new_mixer(rate, buffer_ms)` returns a mixer, 16 producers, 16 shared track
controls, and shared master controls. Construct and drop these outside the
audio callback; call `Mixer::render` inside it. All queues, tables, windows and
filter banks are allocated at construction. Each queue has 97,024 stereo slots,
covering more than 500 ms at the maximum 192 kHz source rate. Tracks start
inactive. Initial targets clamp to 20–200 ms; the GUI defaults to 150 ms.
Each starvation event grows that source's target by 50% (at least 20 ms), up
to 400 ms. It stays at that target until a session/rate reset. Start also waits
for at least a complete output block plus resampler lookahead, so large output
requests cannot immediately exhaust an otherwise valid prefill.

Track and master gain/preamp range from -60 to +12 dB, EQ bands from -12 to
+12 dB, and stereo balance from -1 to +1. NaN becomes zero; infinities and
out-of-range values saturate. Invalid EQ indices are ignored/read as zero.
Preamp precedes EQ and works while EQ is disabled. Muted/solo-excluded tracks
continue consuming input. Only active solo tracks affect solo selection.

Gain, balance, mute and solo use 10 ms ramps. Priming and starvation recovery
use 5 ms fades. On starvation the last filtered stereo sample fades out;
the track then waits for its enlarged prefill and fades back in. This conceals queue
starvation, but does not detect gaps represented by explicit input zero frames.
An underrun counts a running-to-starved transition, not every silent callback.

Controls expose `buffered_frames()`, `target_buffer_ms()`, `primed()`, and
`drift_ppm()` as allocation-free atomic diagnostics at render boundaries.

For discontinuities, call `set_active(false)`, `reset()`, `set_sample_rate(rate)`,
then `set_active(true)`. Reset and deactivation increment an atomic generation,
so toggles between callbacks cannot hide a session change. The next callback
discards the queue snapshot and all track DSP history and re-primes. New frames
racing this discard can also be sacrificed. Coordinate metadata changes before
refilling. A running sample-rate change also discards the old stream. No replay
of buffered samples takes place while a track is inactive.

The resampler uses a 64-tap Blackman-windowed sinc with 512 interpolated phases,
normalization for DC preservation, and a cutoff at 96% of the smaller Nyquist
frequency with a further 0.5% clock-correction guard. Up to four 63-tap halfband
2x stages precede it for large downsampling ratios. New rate tables rebuild in
64-row batches at callback boundaries while the track primes; a new table is
usable after nine nonempty callbacks. This bounds reconfiguration work to
4,096 coefficients per affected track per callback, without allocation. Initial
tables are built fully in the constructor. No trigonometry runs per sample.

Clock correction measures queue occupancy plus unplayed sinc lookahead,
averages callback start/end occupancy, and smooths it with a 750 ms time
constant. A proportional controller targets the requested buffer depth, with
correction bounded to +/-5,000 ppm and an additional 100 ms ratio smoother.
The occupancy target follows the adaptive buffer, and starvation resets clock
correction rather than carrying a stale rate adjustment into recovery.
There is no frame dropping or duplication for ordinary clock correction.

EQ is ten RBJ peaking biquads with Q=sqrt(2), independent f64 stereo states,
and output-rate coefficients. Bands at/above output Nyquist bypass. Controls
are latched once per callback; changed EQ settings crossfade between two stable
banks over 10 ms. Changes during an existing fade are coalesced to the latest
settings at the next callback after the fade ends. Tiny filter states are
flushed explicitly. Independent atomic setters are not a multi-field transaction.

Master gain defaults to -6 dB. A final hard ceiling clamps to +/-0.98; this is
sample-domain clipping protection, not a lookahead or true-peak limiter. Peak
meters show the last nonempty callback's maximum absolute post-fader level;
master peak is after clamping. `clipped_frames()` counts stereo frames once
when either channel exceeds the ceiling. Diagnostic counters are cumulative.

Validated by package tests (including an allocation-audited render path):

| Conversion | Measured response |
| --- | --- |
| 48 → 48 kHz, 20 kHz tone | +0.002 dB |
| 48 → 44.1 kHz, 20 kHz tone | -0.542 dB |
| 44.1 → 48 kHz, 20 kHz tone | -0.258 dB |
| 48 → 44.1 kHz, 23 kHz out-of-band tone | -64.51 dB |
| 192 → 8 kHz, 5 kHz out-of-band tone | -86.86 dB |
| 192 → 8 kHz, 10 kHz out-of-band tone | -101.77 dB |

These are steady-state tone measurements, not a uniform stopband guarantee;
attenuation immediately around Nyquist follows the finite filter transition.
The long drift test simulates 60 seconds in each +/-2,000 ppm clock direction
through the actual queue and renderer, with no underruns and bounded occupancy.
