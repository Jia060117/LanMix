//! Real-time stereo mixer. Construct once, then hand each SPSC producer to its
//! network owner and the mixer to the audio callback. `render` never allocates,
//! waits, performs I/O, or takes a lock. All tracks continue consuming while
//! muted or excluded by solo, keeping them synchronized with live audio.
//!
//! Rates clamp to 8..=192 kHz; initial buffering clamps to 20..=200 ms. After
//! starvation, each track can adapt its target up to 400 ms. Every
//! queue holds at least 500 ms at 192 kHz. A track starts only after its target
//! prefill, and re-primes after starvation, reset, or a running rate change.
//! EQ runs at the output rate, so bands at/above its Nyquist are bypassed.
//! Preamp precedes EQ and remains effective with EQ disabled. The master starts
//! at -6 dB and applies a final, sample-domain hard ceiling at +/-0.98.
//!
//! Lifecycle changes are callback-boundary operations. On reset, queued frames
//! present at that boundary are discarded, including any new frames racing the
//! reset. Coordinate source discontinuities with `TrackControls::reset()`.

mod controls;
mod eq;
mod resampler;

use std::sync::{Arc, atomic::Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

pub use controls::{MasterControls, TrackControls};
use controls::{TrackSnapshot, sample_rate};
use eq::{Equalizer, Ramp, db_to_linear};
use resampler::{Design, MAX_DRIFT, Resampler};

pub type StereoFrame = [f32; 2];
pub const MAX_TRACKS: usize = 16;
pub const EQ_FREQUENCIES: [f32; 10] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
pub const SAFETY_CEILING: f32 = 0.98;

/// Owns all render-thread state. Keep it alive for the audio stream lifetime;
/// constructing or dropping it belongs outside the callback.
pub struct Mixer {
    // Keep construction and moves small on platform/JVM native stacks too.
    // Length is fixed at MAX_TRACKS and never changes after initialization.
    tracks: Box<[Track]>,
    master: Arc<MasterControls>,
    master_eq: Equalizer,
    master_gain: Ramp,
    master_preamp: Ramp,
    design: Design,
    output_rate: u32,
}

/// Preallocate all 16 tracks, their independent SPSC queues, filter banks, and
/// resampling tables. Tracks initially inactive; source rate initially equals
/// output rate. GUI receivers use `buffer_ms = 150` as an initial target.
#[allow(clippy::type_complexity)] // Keep the agreed tuple API explicit.
pub fn new_mixer(
    output_rate: u32,
    buffer_ms: u32,
) -> (
    Mixer,
    Vec<Producer<StereoFrame>>,
    Vec<Arc<TrackControls>>,
    Arc<MasterControls>,
) {
    let output_rate = sample_rate(output_rate);
    let buffer_ms = buffer_ms.clamp(20, 200);
    let design = Design::new();
    let mut producers = Vec::with_capacity(MAX_TRACKS);
    let mut controls = Vec::with_capacity(MAX_TRACKS);
    let tracks = (0..MAX_TRACKS)
        .map(|_| {
            // 500 ms at the largest supported source rate, with extra lookahead.
            let (producer, consumer) = RingBuffer::new(96_000 + 1024);
            let control = Arc::new(TrackControls::new(output_rate, buffer_ms));
            producers.push(producer);
            controls.push(Arc::clone(&control));
            Track::new(consumer, control, output_rate, buffer_ms, &design)
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let master = Arc::new(MasterControls::new());
    let mixer = Mixer {
        tracks,
        master: Arc::clone(&master),
        master_eq: Equalizer::new(output_rate),
        master_gain: Ramp::new(db_to_linear(-6.0)),
        master_preamp: Ramp::new(1.0),
        design,
        output_rate,
    };
    (mixer, producers, controls, master)
}

impl Mixer {
    /// Overwrite the entire output slice. Controls are latched once per call;
    /// an empty slice does nothing, including leaving meters untouched.
    pub fn render(&mut self, output: &mut [StereoFrame]) {
        if output.is_empty() {
            return;
        }
        output.fill([0.0; 2]);
        let snapshots: [TrackSnapshot; MAX_TRACKS] =
            std::array::from_fn(|i| self.tracks[i].controls.snapshot());
        let any_solo = snapshots.iter().any(|s| s.active && s.solo);
        for (track, snapshot) in self.tracks.iter_mut().zip(snapshots) {
            track.render(output, snapshot, any_solo, &self.design);
        }
        let snapshot = self.master.tone.snapshot();
        let ramp = (self.output_rate / 100).max(1);
        self.master_eq.update(snapshot);
        self.master_preamp
            .set(db_to_linear(snapshot.preamp_db), ramp);
        self.master_gain.set(db_to_linear(snapshot.gain_db), ramp);
        let mut peak = 0.0_f32;
        let mut clipped = 0_u64;
        for frame in output {
            let preamp = self.master_preamp.next();
            let filtered = self
                .master_eq
                .process([frame[0] * preamp, frame[1] * preamp]);
            let gain = self.master_gain.next();
            let mut frame_clipped = false;
            for ch in 0..2 {
                let value = filtered[ch] * gain;
                frame_clipped |= !value.is_finite() || value.abs() > SAFETY_CEILING;
                frame[ch] = if value.is_nan() {
                    0.0
                } else {
                    value.clamp(-SAFETY_CEILING, SAFETY_CEILING)
                };
                peak = peak.max(frame[ch].abs());
            }
            clipped += u64::from(frame_clipped);
        }
        self.master.tone.peak.set(peak);
        self.master
            .clipped_count
            .fetch_add(clipped, Ordering::Relaxed);
    }
}

struct Track {
    consumer: Consumer<StereoFrame>,
    controls: Arc<TrackControls>,
    resampler: Resampler,
    equalizer: Equalizer,
    preamp: Ramp,
    left: Ramp,
    right: Ramp,
    gate: Ramp,
    last_filtered: StereoFrame,
    generation: u64,
    active: bool,
    primed: bool,
    output_rate: u32,
    buffer_ms: u32,
    target_buffer_ms: u32,
    smoothed_occupancy: f64,
    correction: f64,
    occupancy_initialized: bool,
}

impl Track {
    fn new(
        consumer: Consumer<StereoFrame>,
        controls: Arc<TrackControls>,
        output_rate: u32,
        buffer_ms: u32,
        design: &Design,
    ) -> Self {
        Self {
            consumer,
            controls,
            resampler: Resampler::new(output_rate, output_rate, design),
            equalizer: Equalizer::new(output_rate),
            preamp: Ramp::new(1.0),
            left: Ramp::new(1.0),
            right: Ramp::new(1.0),
            gate: Ramp::new(0.0),
            last_filtered: [0.0; 2],
            generation: 0,
            active: false,
            primed: false,
            output_rate,
            buffer_ms,
            target_buffer_ms: buffer_ms,
            smoothed_occupancy: 0.0,
            correction: 0.0,
            occupancy_initialized: false,
        }
    }

    fn discard_queue(&mut self) {
        // Only discard the queue snapshot, never chase a concurrent producer.
        let available = self.consumer.slots();
        if available != 0
            && let Ok(chunk) = self.consumer.read_chunk(available)
        {
            chunk.commit_all();
        }
    }

    fn clear(&mut self) {
        self.resampler.clear();
        self.equalizer.clear();
        self.primed = false;
        self.gate = Ramp::new(0.0);
        self.last_filtered = [0.0; 2];
        self.correction = 0.0;
        self.occupancy_initialized = false;
        self.target_buffer_ms = self.buffer_ms;
        self.publish_buffer();
    }

    fn publish_buffer(&self) {
        let frames = self.consumer.slots() as f64 + self.resampler.future_source_frames();
        self.controls
            .buffered_frames
            .store(frames as u32, Ordering::Relaxed);
        self.controls
            .target_buffer_ms
            .store(self.target_buffer_ms, Ordering::Relaxed);
        self.controls.primed.store(self.primed, Ordering::Relaxed);
        self.controls
            .drift_ppm
            .set((self.correction * 1_000_000.0) as f32);
    }

    fn render(
        &mut self,
        output: &mut [StereoFrame],
        snapshot: TrackSnapshot,
        any_solo: bool,
        design: &Design,
    ) {
        let invalidated = snapshot.generation != self.generation;
        let rate_changed = snapshot.rate != self.resampler.source_rate();
        if invalidated || (rate_changed && self.active) {
            self.discard_queue();
            self.clear();
        }
        self.generation = snapshot.generation;
        if rate_changed {
            self.resampler.configure(snapshot.rate);
            self.clear();
        }
        let table_ready = self.resampler.prepare(design);
        if !snapshot.active {
            self.discard_queue();
            if self.active {
                self.clear();
            }
            self.active = false;
            self.controls.tone.peak.set(0.0);
            self.publish_buffer();
            return;
        }
        self.active = true;
        self.equalizer.update(snapshot.tone);
        let ramp = (self.output_rate / 100).max(1);
        let audible = !snapshot.muted && (!any_solo || snapshot.solo);
        let gain = if audible {
            db_to_linear(snapshot.tone.gain_db)
        } else {
            0.0
        };
        self.left
            .set(gain * (1.0 - snapshot.balance.max(0.0)), ramp);
        self.right
            .set(gain * (1.0 + snapshot.balance.min(0.0)), ramp);
        self.preamp.set(db_to_linear(snapshot.tone.preamp_db), ramp);
        // Starting with less than one complete render block plus sinc
        // lookahead causes an immediate underrun even on a perfect network.
        let start_guard = (output.len() as f64 * snapshot.rate as f64 / self.output_rate as f64
            * (1.0 + MAX_DRIFT))
            .ceil() as usize
            + self.resampler.minimum_prefill();
        let guard_ms = (start_guard as u64 * 1000).div_ceil(snapshot.rate as u64) as u32;
        self.target_buffer_ms = self.target_buffer_ms.max(guard_ms.min(400));
        let target = snapshot.rate as f64 * self.target_buffer_ms as f64 / 1000.0;
        let available = self.consumer.slots();
        if !self.primed && table_ready && available >= (target.ceil() as usize).max(start_guard) {
            self.resampler.clear();
            self.equalizer.clear();
            self.primed = true;
            self.gate.set(1.0, ramp / 2);
            self.smoothed_occupancy = target;
            self.occupancy_initialized = true;
        }
        let was_primed = self.primed;
        let occupancy_before = available as f64 + self.resampler.future_source_frames();
        let ratio = self.resampler.ratio();
        let mut peak = 0.0_f32;
        for frame in output.iter_mut() {
            let preamp = self.preamp.next();
            if self.primed {
                if let Some(input) =
                    self.resampler
                        .next(&mut self.consumer, design, ratio * (1.0 + self.correction))
                {
                    self.last_filtered = self
                        .equalizer
                        .process([input[0] * preamp, input[1] * preamp]);
                } else {
                    self.primed = false;
                    self.resampler.clear();
                    self.occupancy_initialized = false;
                    self.correction = 0.0;
                    // Repeating the same too-small prefill creates recurring
                    // dropouts on lossless but batched Wi-Fi/driver delivery.
                    // Grow once per running->starved event and retain the new
                    // target until an explicit session/rate reset.
                    self.target_buffer_ms =
                        (self.target_buffer_ms + (self.target_buffer_ms / 2).max(20)).min(400);
                    self.controls.underrun_count.fetch_add(1, Ordering::Relaxed);
                    self.gate.set(0.0, (ramp / 2).max(1));
                }
            } else {
                // Let an in-flight EQ transition finish even while unprimed.
                let _ = self.equalizer.process([0.0; 2]);
            }
            let gate = self.gate.next();
            let left = self.last_filtered[0] * gate * self.left.next();
            let right = self.last_filtered[1] * gate * self.right.next();
            frame[0] += left;
            frame[1] += right;
            peak = peak.max(left.abs()).max(right.abs());
        }
        if was_primed && self.primed && self.occupancy_initialized {
            // Include the sinc's unplayed lookahead, not its already-played
            // history. Average callback start/end to remove callback sawtooth.
            let occupancy_after =
                self.consumer.slots() as f64 + self.resampler.future_source_frames();
            let measured = 0.5 * (occupancy_before + occupancy_after);
            let dt = output.len() as f64 / self.output_rate as f64;
            // Rational one-pole smoothing: no trig/exp in the sample loop.
            let alpha = dt / (0.75 + dt);
            self.smoothed_occupancy += alpha * (measured - self.smoothed_occupancy);
            let desired =
                (0.025 * (self.smoothed_occupancy - target) / target).clamp(-MAX_DRIFT, MAX_DRIFT);
            self.correction += (dt / (0.1 + dt)) * (desired - self.correction);
        }
        self.controls.tone.peak.set(peak);
        self.publish_buffer();
    }
}

#[cfg(test)]
mod tests;
