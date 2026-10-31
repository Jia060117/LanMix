use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Supported sample rates are clamped to this inclusive range.
pub(crate) fn sample_rate(value: u32) -> u32 {
    value.clamp(8_000, 192_000)
}

fn sanitized(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(min, max)
    }
}

#[derive(Debug)]
pub(crate) struct AtomicFloat(AtomicU32);

impl AtomicFloat {
    pub(crate) fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    pub(crate) fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub(crate) fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(crate) struct ToneControls {
    gain: AtomicFloat,
    enabled: AtomicBool,
    bands: [AtomicFloat; 10],
    preamp: AtomicFloat,
    pub(crate) peak: AtomicFloat,
}

impl ToneControls {
    fn new(gain: f32) -> Self {
        Self {
            gain: AtomicFloat::new(gain),
            enabled: AtomicBool::new(false),
            bands: std::array::from_fn(|_| AtomicFloat::new(0.0)),
            preamp: AtomicFloat::new(0.0),
            peak: AtomicFloat::new(0.0),
        }
    }

    pub(crate) fn snapshot(&self) -> ToneSnapshot {
        ToneSnapshot {
            gain_db: self.gain.get(),
            preamp_db: self.preamp.get(),
            enabled: self.enabled.load(Ordering::Relaxed),
            bands: std::array::from_fn(|i| self.bands[i].get()),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ToneSnapshot {
    pub(crate) gain_db: f32,
    pub(crate) preamp_db: f32,
    pub(crate) enabled: bool,
    pub(crate) bands: [f32; 10],
}

/// Thread-safe controls for one permanently assigned producer/track pair.
///
/// Setters are independent atomics, not a multi-field transaction. Values are
/// sampled once per render call. NaN becomes zero; infinities saturate.
/// Gain/preamp: -60..=12 dB; EQ: -12..=12 dB; balance: -1..=1.
#[derive(Debug)]
pub struct TrackControls {
    // Bit zero is active. The remaining bits are an invalidation generation.
    lifecycle: AtomicU64,
    rate: AtomicU32,
    muted: AtomicBool,
    solo: AtomicBool,
    balance: AtomicFloat,
    pub(crate) tone: ToneControls,
    pub(crate) underrun_count: AtomicU64,
    pub(crate) buffered_frames: AtomicU32,
    pub(crate) target_buffer_ms: AtomicU32,
    pub(crate) primed: AtomicBool,
    pub(crate) drift_ppm: AtomicFloat,
}

impl TrackControls {
    pub(crate) fn new(rate: u32, buffer_ms: u32) -> Self {
        Self {
            lifecycle: AtomicU64::new(0),
            rate: AtomicU32::new(rate),
            muted: AtomicBool::new(false),
            solo: AtomicBool::new(false),
            balance: AtomicFloat::new(0.0),
            tone: ToneControls::new(0.0),
            underrun_count: AtomicU64::new(0),
            buffered_frames: AtomicU32::new(0),
            target_buffer_ms: AtomicU32::new(buffer_ms),
            primed: AtomicBool::new(false),
            drift_ppm: AtomicFloat::new(0.0),
        }
    }

    /// Deactivation invalidates buffered audio, including a false/true toggle
    /// that takes place entirely between render calls. Initial activation
    /// preserves the initial prefill.
    pub fn set_active(&self, active: bool) {
        // Keep the update atomic with reset(), without depending on the newer
        // try_update API or the fetch_update name deprecated by newer Rust.
        let mut state = self.lifecycle.load(Ordering::Acquire);
        loop {
            let next = if active {
                state | 1
            } else if state & 1 != 0 {
                (state & !1).wrapping_add(2)
            } else {
                state
            };
            match self.lifecycle.compare_exchange_weak(
                state,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => state = current,
            }
        }
    }

    pub fn active(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) & 1 != 0
    }

    /// Invalidate queued frames and all resampler/filter history on the next
    /// callback. The callback discards a bounded snapshot of the queue, so some
    /// new frames racing with reset may also be discarded. Refill to re-prime.
    /// This does not clear controls or lifetime diagnostic counters.
    pub fn reset(&self) {
        self.lifecycle.fetch_add(2, Ordering::AcqRel);
    }

    pub fn set_sample_rate(&self, rate: u32) {
        self.rate.store(sample_rate(rate), Ordering::Relaxed);
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_solo(&self, solo: bool) {
        self.solo.store(solo, Ordering::Relaxed);
    }

    pub fn solo(&self) -> bool {
        self.solo.load(Ordering::Relaxed)
    }

    /// Stereo balance: center preserves both channels, endpoints silence one
    /// channel. This is not a mono equal-power pan.
    pub fn set_balance(&self, balance: f32) {
        self.balance.set(sanitized(balance, -1.0, 1.0));
    }

    pub fn balance(&self) -> f32 {
        self.balance.get()
    }

    /// Number of running-to-starved transitions. Initial priming is not an
    /// underrun, and consecutive silent callbacks count as a single event.
    pub fn underruns(&self) -> u64 {
        self.underrun_count.load(Ordering::Relaxed)
    }

    /// Unplayed source frames at the last render boundary, including lookahead.
    pub fn buffered_frames(&self) -> u32 {
        self.buffered_frames.load(Ordering::Relaxed)
    }

    /// Current adaptive target. The configured value is a lower bound;
    /// starvation can increase it to 400 ms for this source's current session.
    pub fn target_buffer_ms(&self) -> u32 {
        self.target_buffer_ms.load(Ordering::Relaxed)
    }

    pub fn primed(&self) -> bool {
        self.primed.load(Ordering::Relaxed)
    }

    pub fn drift_ppm(&self) -> f32 {
        self.drift_ppm.get()
    }

    pub(crate) fn snapshot(&self) -> TrackSnapshot {
        let lifecycle = self.lifecycle.load(Ordering::Acquire);
        TrackSnapshot {
            active: lifecycle & 1 != 0,
            generation: lifecycle >> 1,
            rate: self.sample_rate(),
            muted: self.muted(),
            solo: self.solo(),
            balance: self.balance(),
            tone: self.tone.snapshot(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TrackSnapshot {
    pub(crate) active: bool,
    pub(crate) generation: u64,
    pub(crate) rate: u32,
    pub(crate) muted: bool,
    pub(crate) solo: bool,
    pub(crate) balance: f32,
    pub(crate) tone: ToneSnapshot,
}

/// Master tone and level controls. The default -6 dB gain reserves headroom.
#[derive(Debug)]
pub struct MasterControls {
    pub(crate) tone: ToneControls,
    pub(crate) clipped_count: AtomicU64,
}

impl MasterControls {
    pub(crate) fn new() -> Self {
        Self {
            tone: ToneControls::new(-6.0),
            clipped_count: AtomicU64::new(0),
        }
    }

    /// Stereo output frames with either channel exceeding the safety ceiling
    /// before clamping, counted once per frame (not per channel).
    pub fn clipped_frames(&self) -> u64 {
        self.clipped_count.load(Ordering::Relaxed)
    }
}

macro_rules! tone_api {
    ($ty:ty) => {
        impl $ty {
            pub fn set_gain_db(&self, db: f32) {
                self.tone.gain.set(sanitized(db, -60.0, 12.0));
            }
            pub fn gain_db(&self) -> f32 {
                self.tone.gain.get()
            }
            pub fn set_eq_enabled(&self, enabled: bool) {
                self.tone.enabled.store(enabled, Ordering::Relaxed);
            }
            pub fn eq_enabled(&self) -> bool {
                self.tone.enabled.load(Ordering::Relaxed)
            }
            /// Invalid band indices are ignored; getters return zero.
            pub fn set_eq_band(&self, band: usize, db: f32) {
                if let Some(value) = self.tone.bands.get(band) {
                    value.set(sanitized(db, -12.0, 12.0));
                }
            }
            pub fn eq_band(&self, band: usize) -> f32 {
                self.tone.bands.get(band).map_or(0.0, AtomicFloat::get)
            }
            /// Preamp is applied whether or not EQ is enabled.
            pub fn set_preamp_db(&self, db: f32) {
                self.tone.preamp.set(sanitized(db, -60.0, 12.0));
            }
            pub fn preamp_db(&self) -> f32 {
                self.tone.preamp.get()
            }
            /// Maximum absolute post-fader sample in the last nonempty render
            /// call. Master peak is measured after the hard ceiling.
            pub fn peak(&self) -> f32 {
                self.tone.peak.get()
            }
        }
    };
}

tone_api!(TrackControls);
tone_api!(MasterControls);
