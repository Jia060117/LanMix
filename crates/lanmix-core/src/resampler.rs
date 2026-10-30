use rtrb::Consumer;

use crate::StereoFrame;

const TAPS: usize = 64;
const PHASES: usize = 512;
const HALF_TAPS: usize = 63;
const MAX_STAGES: usize = 4;
pub(crate) const MAX_DRIFT: f64 = 0.005;

/// Precomputed fractional Blackman windows, shared during construction only.
pub(crate) struct Design {
    windows: Box<[[f64; TAPS]]>,
    halfband: [f64; HALF_TAPS],
}

impl Design {
    pub(crate) fn new() -> Self {
        let mut windows = vec![[0.0; TAPS]; PHASES + 1].into_boxed_slice();
        for (phase, row) in windows.iter_mut().enumerate() {
            for (tap, value) in row.iter_mut().enumerate() {
                let x = (tap as f64 - 31.0 - phase as f64 / PHASES as f64) / 32.0;
                *value = 0.42
                    + 0.5 * (std::f64::consts::PI * x).cos()
                    + 0.08 * (std::f64::consts::TAU * x).cos();
            }
        }
        let mut halfband = [0.0; HALF_TAPS];
        for (i, c) in halfband.iter_mut().enumerate() {
            let x = i as i32 - 31;
            let window = 0.42 - 0.5 * (std::f64::consts::TAU * i as f64 / 62.0).cos()
                + 0.08 * (2.0 * std::f64::consts::TAU * i as f64 / 62.0).cos();
            *c = if x == 0 {
                0.5
            } else if x % 2 == 0 {
                0.0
            } else {
                (std::f64::consts::FRAC_PI_2 * x as f64).sin() / (std::f64::consts::PI * x as f64)
                    * window
            };
        }
        let sum: f64 = halfband.iter().sum();
        for c in &mut halfband {
            *c /= sum;
        }
        Self { windows, halfband }
    }
}

struct Halfband {
    history: [StereoFrame; HALF_TAPS],
    cursor: usize,
    initialized: bool,
}

impl Halfband {
    fn new() -> Self {
        Self {
            history: [[0.0; 2]; HALF_TAPS],
            cursor: 0,
            initialized: false,
        }
    }

    fn push(&mut self, frame: StereoFrame) {
        if !self.initialized {
            self.history.fill(frame);
            self.initialized = true;
        }
        self.history[self.cursor] = frame;
        self.cursor = (self.cursor + 1) % HALF_TAPS;
    }

    fn output(&self, coefficients: &[f64; HALF_TAPS]) -> StereoFrame {
        // Odd taps and the center only: the other halfband taps are zero.
        let center = self.history[(self.cursor + 31) % HALF_TAPS];
        let mut result = [
            center[0] as f64 * coefficients[31],
            center[1] as f64 * coefficients[31],
        ];
        for i in (0..HALF_TAPS).step_by(2) {
            let frame = self.history[(self.cursor + i) % HALF_TAPS];
            result[0] += frame[0] as f64 * coefficients[i];
            result[1] += frame[1] as f64 * coefficients[i];
        }
        [result[0] as f32, result[1] as f32]
    }

    fn clear(&mut self) {
        self.initialized = false;
        self.cursor = 0;
    }
}

/// A 64-tap, 512-phase sinc converter with interpolated phase rows. Large
/// downsampling ratios first pass through up to four 2x halfband decimators,
/// keeping the final sinc ratio <= 2 and avoiding a narrow, ineffective kernel.
pub(crate) struct Resampler {
    table: Box<[[f32; TAPS]]>,
    window: [StereoFrame; TAPS],
    head: usize,
    phase: f64,
    initialized: bool,
    stages: [Halfband; MAX_STAGES],
    stage_count: usize,
    source_rate: u32,
    output_rate: u32,
    built_rows: usize,
    cutoff: f64,
}

impl Resampler {
    pub(crate) fn new(source_rate: u32, output_rate: u32, design: &Design) -> Self {
        let mut result = Self {
            table: vec![[0.0; TAPS]; PHASES + 1].into_boxed_slice(),
            window: [[0.0; 2]; TAPS],
            head: 0,
            phase: 0.0,
            initialized: false,
            stages: std::array::from_fn(|_| Halfband::new()),
            stage_count: 0,
            source_rate,
            output_rate,
            built_rows: 0,
            cutoff: 1.0,
        };
        result.configure(source_rate);
        result.build_rows(design, PHASES + 1);
        result
    }

    pub(crate) fn configure(&mut self, source_rate: u32) {
        self.source_rate = source_rate;
        self.stage_count = 0;
        while source_rate as f64 / ((1 << self.stage_count) as f64 * self.output_rate as f64) > 2.0
            && self.stage_count < MAX_STAGES
        {
            self.stage_count += 1;
        }
        // Reserve cutoff for the maximum positive clock correction. Blackman
        // rolloff trades the last few percent of bandwidth for alias rejection.
        // Retain the high end, with a modest transition band and a drift guard.
        // As with every finite sinc, rejection immediately at Nyquist is less
        // than stopband rejection: see measured passband/alias tests.
        self.cutoff = 0.96 / (self.ratio() * (1.0 + MAX_DRIFT)).max(1.0);
        self.built_rows = 0;
        self.clear();
    }

    /// Partial tables are never rendered. Rate changes rebuild at most 64 rows
    /// per callback while the track is priming (4096 coefficients per track).
    pub(crate) fn prepare(&mut self, design: &Design) -> bool {
        self.build_rows(design, 64);
        self.built_rows == PHASES + 1
    }

    fn build_rows(&mut self, design: &Design, count: usize) {
        if self.built_rows == PHASES + 1 {
            return;
        }
        let cutoff = self.cutoff;
        let angle = std::f64::consts::PI * cutoff;
        let (step_sin, step_cos) = angle.sin_cos();
        let end = (self.built_rows + count).min(PHASES + 1);
        for phase in self.built_rows..end {
            let row = &mut self.table[phase];
            let fraction = phase as f64 / PHASES as f64;
            let (mut sin, mut cos) = (angle * (-31.0 - fraction)).sin_cos();
            let mut sum = 0.0;
            for (tap, coefficient) in row.iter_mut().enumerate() {
                let x = tap as f64 - 31.0 - fraction;
                let sinc = if x.abs() < 1.0e-12 {
                    cutoff
                } else {
                    sin / (std::f64::consts::PI * x)
                };
                let value = sinc * design.windows[phase][tap];
                *coefficient = value as f32;
                sum += value;
                (sin, cos) = (
                    sin * step_cos + cos * step_sin,
                    cos * step_cos - sin * step_sin,
                );
            }
            for coefficient in row {
                *coefficient = (*coefficient as f64 / sum) as f32;
            }
        }
        self.built_rows = end;
    }

    pub(crate) fn source_rate(&self) -> u32 {
        self.source_rate
    }

    pub(crate) fn ratio(&self) -> f64 {
        self.source_rate as f64 / ((1 << self.stage_count) as f64 * self.output_rate as f64)
    }

    pub(crate) fn minimum_prefill(&self) -> usize {
        34 * (1 << self.stage_count)
    }

    pub(crate) fn future_source_frames(&self) -> f64 {
        if self.initialized {
            (32.0 - self.phase) * (1 << self.stage_count) as f64
        } else {
            0.0
        }
    }

    pub(crate) fn clear(&mut self) {
        self.initialized = false;
        self.head = 0;
        self.phase = 0.0;
        for stage in &mut self.stages {
            stage.clear();
        }
    }

    fn read(
        &mut self,
        consumer: &mut Consumer<StereoFrame>,
        coefficients: &[f64; HALF_TAPS],
        level: usize,
    ) -> Option<StereoFrame> {
        if level == 0 {
            return consumer.pop().ok().map(|frame| {
                frame.map(|x| {
                    if x.is_finite() {
                        x.clamp(-16.0, 16.0)
                    } else {
                        0.0
                    }
                })
            });
        }
        for _ in 0..2 {
            let frame = self.read(consumer, coefficients, level - 1)?;
            self.stages[level - 1].push(frame);
        }
        Some(self.stages[level - 1].output(coefficients))
    }

    pub(crate) fn next(
        &mut self,
        consumer: &mut Consumer<StereoFrame>,
        design: &Design,
        ratio: f64,
    ) -> Option<StereoFrame> {
        if !self.initialized {
            let first = self.read(consumer, &design.halfband, self.stage_count)?;
            self.window[..32].fill(first);
            for tap in 32..TAPS {
                self.window[tap] = self.read(consumer, &design.halfband, self.stage_count)?;
            }
            self.initialized = true;
        }
        // Advance before producing the next frame: a failed read never leaves
        // a partially advanced window eligible for reuse after starvation.
        while self.phase >= 1.0 {
            let frame = self.read(consumer, &design.halfband, self.stage_count)?;
            self.window[self.head] = frame;
            self.head = (self.head + 1) % TAPS;
            self.phase -= 1.0;
        }
        let position = self.phase * PHASES as f64;
        let phase = (position as usize).min(PHASES - 1);
        let blend = (position - phase as f64) as f32;
        let a = &self.table[phase];
        let b = &self.table[phase + 1];
        let mut sum = [0.0_f64; 2];
        for tap in 0..TAPS {
            let coefficient = (a[tap] + (b[tap] - a[tap]) * blend) as f64;
            let frame = self.window[(self.head + tap) % TAPS];
            sum[0] += frame[0] as f64 * coefficient;
            sum[1] += frame[1] as f64 * coefficient;
        }
        self.phase += ratio;
        Some([sum[0] as f32, sum[1] as f32])
    }
}
