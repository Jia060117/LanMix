use crate::{EQ_FREQUENCIES, StereoFrame, controls::ToneSnapshot};

pub(crate) fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// A finite, fixed-length ramp; repeated unchanged targets do not restart it.
pub(crate) struct Ramp {
    value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Ramp {
    pub(crate) fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }

    pub(crate) fn set(&mut self, target: f32, samples: u32) {
        if target != self.target {
            self.target = target;
            self.remaining = samples.max(1);
            self.step = (target - self.value) / self.remaining as f32;
        }
    }

    pub(crate) fn next(&mut self) -> f32 {
        if self.remaining != 0 {
            self.remaining -= 1;
            self.value += self.step;
            if self.remaining == 0 {
                self.value = self.target;
            }
        }
        self.value
    }
}

#[derive(Clone, Copy)]
struct Coefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefficients {
    const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn peaking(rate: u32, frequency: f32, db: f32) -> Self {
        if db == 0.0 || frequency >= rate as f32 * 0.5 {
            return Self::IDENTITY;
        }
        // RBJ peaking EQ; octave-spaced bands with Q = sqrt(2).
        let a = 10.0_f64.powf(db as f64 / 40.0);
        let w = std::f64::consts::TAU * frequency as f64 / rate as f64;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * std::f64::consts::SQRT_2);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

#[derive(Clone, Copy)]
struct Biquad {
    coefficients: Coefficients,
    // Transposed direct form II, with independent double-precision channels.
    z1: [f64; 2],
    z2: [f64; 2],
}

impl Biquad {
    const IDENTITY: Self = Self {
        coefficients: Coefficients::IDENTITY,
        z1: [0.0; 2],
        z2: [0.0; 2],
    };

    fn process(&mut self, frame: &mut [f64; 2]) {
        let c = self.coefficients;
        for (ch, sample) in frame.iter_mut().enumerate() {
            let y = c.b0 * *sample + self.z1[ch];
            self.z1[ch] = flush_tiny(c.b1 * *sample - c.a1 * y + self.z2[ch]);
            self.z2[ch] = flush_tiny(c.b2 * *sample - c.a2 * y);
            *sample = y;
        }
    }
}

fn flush_tiny(value: f64) -> f64 {
    if value.abs() < 1.0e-24 { 0.0 } else { value }
}

#[derive(Clone, Copy, PartialEq)]
struct EqSettings {
    enabled: bool,
    bands: [f32; 10],
}

impl EqSettings {
    const FLAT: Self = Self {
        enabled: false,
        bands: [0.0; 10],
    };
}

struct Bank {
    bands: [Biquad; 10],
    settings: EqSettings,
    audible: bool,
}

impl Bank {
    fn new() -> Self {
        Self {
            bands: [Biquad::IDENTITY; 10],
            settings: EqSettings::FLAT,
            audible: false,
        }
    }

    fn configure(&mut self, settings: EqSettings, rate: u32) {
        self.settings = settings;
        self.audible = settings.enabled && settings.bands.iter().any(|&x| x != 0.0);
        for (i, band) in self.bands.iter_mut().enumerate() {
            *band = Biquad::IDENTITY;
            if settings.enabled {
                band.coefficients =
                    Coefficients::peaking(rate, EQ_FREQUENCIES[i], settings.bands[i]);
            }
        }
    }

    fn clear(&mut self) {
        for b in &mut self.bands {
            b.z1 = [0.0; 2];
            b.z2 = [0.0; 2];
        }
    }

    fn process(&mut self, input: StereoFrame) -> StereoFrame {
        if !self.audible {
            return input;
        }
        let mut frame = [input[0] as f64, input[1] as f64];
        for band in &mut self.bands {
            band.process(&mut frame);
        }
        [frame[0] as f32, frame[1] as f32]
    }
}

/// Two stable filter banks crossfade over 10 ms. If controls change while a
/// fade runs, the latest settings are picked up at the first callback after
/// that fade completes. Coefficient calculation happens only in update().
pub(crate) struct Equalizer {
    banks: [Bank; 2],
    current: usize,
    remaining: u32,
    fade_samples: u32,
    rate: u32,
}

impl Equalizer {
    pub(crate) fn new(rate: u32) -> Self {
        Self {
            banks: [Bank::new(), Bank::new()],
            current: 0,
            remaining: 0,
            fade_samples: (rate / 100).max(1),
            rate,
        }
    }

    pub(crate) fn update(&mut self, snapshot: ToneSnapshot) {
        let settings = EqSettings {
            enabled: snapshot.enabled,
            bands: snapshot.bands,
        };
        if self.remaining == 0 && self.banks[self.current].settings != settings {
            self.banks[1 - self.current].configure(settings, self.rate);
            self.remaining = self.fade_samples;
        }
    }

    pub(crate) fn clear(&mut self) {
        for bank in &mut self.banks {
            bank.clear();
        }
    }

    pub(crate) fn process(&mut self, input: StereoFrame) -> StereoFrame {
        let a = self.banks[self.current].process(input);
        if self.remaining == 0 {
            return a;
        }
        let b = self.banks[1 - self.current].process(input);
        let blend = (self.fade_samples - self.remaining + 1) as f32 / self.fade_samples as f32;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.current = 1 - self.current;
        }
        [a[0] + (b[0] - a[0]) * blend, a[1] + (b[1] - a[1]) * blend]
    }
}
