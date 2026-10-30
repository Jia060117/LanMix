use super::*;

#[test]
fn construction_and_moves_fit_a_small_platform_stack() {
    std::thread::Builder::new()
        .name("small-platform-stack".into())
        .stack_size(512 * 1024)
        .spawn(|| {
            let (mut mixer, _, _, _) = new_mixer(48000, 60);
            let mut output = [[0.0; 2]; 32];
            mixer.render(&mut output);
            assert!(output.iter().all(|frame| *frame == [0.0; 2]));
        })
        .unwrap()
        .join()
        .unwrap();
}

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct AuditedAllocator;

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

// SAFETY: Every operation delegates to System with exactly the original
// pointer/layout arguments; the thread-local counter does not allocate.
unsafe impl GlobalAlloc for AuditedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: GlobalAlloc's caller supplies a valid allocation layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: GlobalAlloc's caller supplies a valid allocation layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller provides an allocated pointer and valid layouts.
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The caller provides System's allocated pointer/layout pair.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static TEST_ALLOCATOR: AuditedAllocator = AuditedAllocator;

fn fill(producer: &mut Producer<StereoFrame>, count: usize, frame: StereoFrame) {
    for _ in 0..count {
        producer.push(frame).unwrap();
    }
}

fn settled(output: &[StereoFrame], expected: StereoFrame, tolerance: f32) {
    for frame in &output[output.len() - 64..] {
        for ch in 0..2 {
            assert!(
                (frame[ch] - expected[ch]).abs() < tolerance,
                "{frame:?}, expected {expected:?}"
            );
        }
    }
}

#[test]
fn controls_sanitize_and_queues_cover_maximum_rate() {
    let (_, producers, controls, master) = new_mixer(0, 0);
    assert_eq!(producers.len(), MAX_TRACKS);
    assert!(producers.iter().all(|p| p.slots() >= 96_000));
    let track = &controls[0];
    assert!(!track.active());
    assert_eq!(track.sample_rate(), 8000);
    track.set_sample_rate(u32::MAX);
    assert_eq!(track.sample_rate(), 192_000);
    track.set_gain_db(f32::INFINITY);
    track.set_balance(f32::NEG_INFINITY);
    track.set_preamp_db(f32::NAN);
    track.set_eq_band(0, f32::NEG_INFINITY);
    track.set_eq_band(usize::MAX, 1.0);
    assert_eq!(track.gain_db(), 12.0);
    assert_eq!(track.balance(), -1.0);
    assert_eq!(track.preamp_db(), 0.0);
    assert_eq!(track.eq_band(0), -12.0);
    assert_eq!(track.eq_band(usize::MAX), 0.0);
    master.set_gain_db(f32::NAN);
    master.set_preamp_db(100.0);
    master.set_eq_band(9, f32::INFINITY);
    assert_eq!(master.gain_db(), 0.0);
    assert_eq!(master.preamp_db(), 12.0);
    assert_eq!(master.eq_band(9), 12.0);
}

#[test]
fn mix_gain_preamp_balance_and_headroom() {
    let (mut mixer, mut producers, controls, master) = new_mixer(48_000, 20);
    for i in 0..2 {
        fill(&mut producers[i], 16_000, [0.1, 0.2]);
        controls[i].set_active(true);
    }
    let mut out = [[0.0; 2]; 1024];
    mixer.render(&mut out);
    settled(
        &out,
        [0.2 * db_to_linear(-6.0), 0.4 * db_to_linear(-6.0)],
        1e-5,
    );
    master.set_gain_db(0.0);
    controls[0].set_gain_db(-6.0);
    controls[0].set_preamp_db(6.0);
    controls[0].set_balance(1.0);
    mixer.render(&mut out);
    settled(&out, [0.1, 0.4], 1e-5);
    assert!((master.peak() - 0.4).abs() < 1e-5);
    assert_eq!(master.clipped_frames(), 0);
}

#[test]
fn mute_solo_and_muted_tracks_keep_consuming() {
    let (mut mixer, mut producers, controls, master) = new_mixer(48_000, 20);
    master.set_gain_db(0.0);
    for i in 0..2 {
        fill(&mut producers[i], 12_000, [0.1 * (i + 1) as f32; 2]);
        controls[i].set_active(true);
    }
    let mut out = [[0.0; 2]; 1024];
    mixer.render(&mut out);
    controls[0].set_solo(true);
    let before = producers[1].slots();
    mixer.render(&mut out);
    settled(&out, [0.1; 2], 1e-5);
    assert!(producers[1].slots() > before + 1000);
    controls[0].set_muted(true);
    mixer.render(&mut out);
    settled(&out, [0.0; 2], 1e-5);
    controls[0].set_active(false);
    mixer.render(&mut out);
    settled(&out, [0.2; 2], 1e-5);
}

#[test]
fn reset_and_unobserved_disconnect_never_replay_old_audio() {
    let (mut mixer, mut producers, controls, _) = new_mixer(48_000, 20);
    let mut out = [[0.0; 2]; 1024];
    controls[0].set_active(true);
    fill(&mut producers[0], 6000, [0.4; 2]);
    mixer.render(&mut out);
    assert!(out[1000][0] > 0.1);
    controls[0].reset();
    mixer.render(&mut out);
    assert!(out.iter().all(|&f| f == [0.0; 2]));
    fill(&mut producers[0], 6000, [-0.3; 2]);
    mixer.render(&mut out);
    assert!(out[1000][0] < -0.1);
    controls[0].set_active(false);
    controls[0].reset();
    controls[0].set_sample_rate(44_100);
    controls[0].set_active(true);
    mixer.render(&mut out);
    assert!(out.iter().all(|&f| f == [0.0; 2]));
    // Complete the bounded coefficient rebuild before prefill.
    for _ in 0..10 {
        mixer.render(&mut out);
    }
    fill(&mut producers[0], 6000, [0.2; 2]);
    mixer.render(&mut out);
    settled(&out, [0.2 * db_to_linear(-6.0); 2], 1e-5);
    controls[0].set_active(false);
    controls[0].set_active(true);
    mixer.render(&mut out);
    assert!(out.iter().all(|&f| f == [0.0; 2]));
}

#[test]
fn starvation_is_concealed_and_recovery_is_ramped() {
    let (mut mixer, mut producers, controls, master) = new_mixer(48_000, 20);
    master.set_gain_db(0.0);
    controls[0].set_active(true);
    let mut out = [[0.0; 2]; 2048];
    mixer.render(&mut out);
    assert_eq!(controls[0].underruns(), 0);
    fill(&mut producers[0], 1200, [0.25; 2]);
    mixer.render(&mut out);
    assert_eq!(controls[0].underruns(), 0);
    assert!(
        !controls[0].primed(),
        "must wait for a complete render block"
    );
    assert!(out.iter().all(|frame| *frame == [0.0; 2]));
    fill(&mut producers[0], 1800, [0.25; 2]);
    mixer.render(&mut out);
    assert!(controls[0].primed());
    mixer.render(&mut out);
    assert_eq!(controls[0].underruns(), 1);
    assert!(out.windows(2).all(|x| (x[1][0] - x[0][0]).abs() < 0.002));
    settled(&out, [0.0; 2], 1e-6);
    mixer.render(&mut out);
    assert_eq!(controls[0].underruns(), 1);
    fill(&mut producers[0], 6000, [-0.25; 2]);
    mixer.render(&mut out);
    assert!(out.windows(2).all(|x| (x[1][0] - x[0][0]).abs() < 0.002));
    settled(&out, [-0.25; 2], 1e-6);
}

#[test]
fn arbitrary_input_is_finite_and_ceiling_counts_stereo_frames_once() {
    let (mut mixer, mut producers, controls, master) = new_mixer(48_000, 20);
    master.set_gain_db(12.0);
    master.set_preamp_db(12.0);
    for i in 0..MAX_TRACKS {
        controls[i].set_active(true);
        controls[i].set_gain_db(12.0);
        controls[i].set_preamp_db(12.0);
        controls[i].set_eq_enabled(true);
        for band in 0..10 {
            controls[i].set_eq_band(band, 12.0);
        }
        for j in 0..3000 {
            producers[i]
                .push(match j % 5 {
                    0 => [f32::NAN, f32::INFINITY],
                    1 => [f32::MAX, -f32::MAX],
                    2 => [f32::NEG_INFINITY, f32::NAN],
                    _ => [1.0, -1.0],
                })
                .unwrap();
        }
    }
    let mut out = [[0.0; 2]; 2048];
    mixer.render(&mut out);
    assert!(
        out.iter()
            .flatten()
            .all(|x| x.is_finite() && x.abs() <= SAFETY_CEILING)
    );
    assert!(master.clipped_frames() > 0);
    assert!(master.clipped_frames() <= out.len() as u64);
    assert_eq!(master.peak(), SAFETY_CEILING);
}

fn tone_amplitude(source_rate: u32, output_rate: u32, frequency: f64) -> (f64, f64) {
    let design = Design::new();
    let mut converter = Resampler::new(source_rate, output_rate, &design);
    let count = output_rate as usize / 4;
    let skip = output_rate as usize / 20;
    let source_count =
        ((count + skip) as f64 * source_rate as f64 / output_rate as f64).ceil() as usize + 2048;
    let (mut producer, mut consumer) = RingBuffer::new(source_count);
    for i in 0..source_count {
        let sine = (std::f64::consts::TAU * frequency * i as f64 / source_rate as f64).sin() as f32;
        producer.push([sine * 0.25, 0.1]).unwrap();
    }
    let mut energy = 0.0;
    let mut dc_error = 0.0_f64;
    for i in 0..count + skip {
        let frame = converter
            .next(&mut consumer, &design, converter.ratio())
            .unwrap();
        if i >= skip {
            energy += (frame[0] as f64).powi(2);
            dc_error = dc_error.max((frame[1] as f64 - 0.1).abs());
        }
    }
    ((2.0 * energy / count as f64).sqrt(), dc_error)
}

#[test]
fn resampler_preserves_passband_and_independent_stereo_dc() {
    for (source, output) in [
        (8000, 192000),
        (192000, 8000),
        (44100, 48000),
        (48000, 44100),
        (48000, 48000),
        (96000, 48000),
    ] {
        let (amplitude, dc_error) = tone_amplitude(source, output, 1000.0);
        eprintln!("{source}->{output}: 1 kHz amplitude {amplitude:.8}, DC error {dc_error:.3e}");
        assert!(
            (amplitude - 0.25).abs() < 0.001,
            "{source}->{output}: {amplitude}"
        );
        assert!(dc_error < 1e-6);
    }
    for (source, output) in [(48000, 48000), (48000, 44100), (44100, 48000)] {
        let (amplitude, _) = tone_amplitude(source, output, 20000.0);
        let db = 20.0 * (amplitude / 0.25).log10();
        eprintln!("{source}->{output}: 20 kHz {db:.3} dB");
        assert!(db > -0.8 && db < 0.1, "{source}->{output}: {db} dB");
    }
}

#[test]
fn resampler_rejects_out_of_band_aliases_including_extreme_downsampling() {
    for (source, output, frequency) in [
        (192000, 8000, 5000.0),
        (192000, 8000, 10000.0),
        (192000, 8000, 65000.0),
        (48000, 16000, 10000.0),
        (48000, 8000, 10000.0),
        (48000, 44100, 23000.0),
    ] {
        let (amplitude, _) = tone_amplitude(source, output, frequency);
        let db = 20.0 * (amplitude / 0.25).log10();
        eprintln!("{source}->{output}: {frequency} Hz stopband {db:.2} dB");
        assert!(db < -50.0, "{source}->{output} {frequency} Hz: {db} dB");
    }
}

#[test]
fn peaking_eq_changes_band_and_bypasses_above_nyquist() {
    let mut equalizer = Equalizer::new(48000);
    let controls = MasterControls::new();
    controls.set_eq_enabled(true);
    controls.set_eq_band(5, 6.0);
    equalizer.update(controls.tone.snapshot());
    let mut input_energy = 0.0;
    let mut output_energy = 0.0;
    for i in 0..24000 {
        let x = (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin() * 0.1;
        let out = equalizer.process([x, 0.0]);
        assert_eq!(out[1], 0.0);
        if i > 4800 {
            input_energy += (x as f64).powi(2);
            output_energy += (out[0] as f64).powi(2);
        }
    }
    assert!((10.0 * (output_energy / input_energy).log10() - 6.0).abs() < 0.02);
    let mut equalizer = Equalizer::new(8000);
    controls.set_eq_band(5, 0.0);
    for band in 7..10 {
        controls.set_eq_band(band, 12.0);
    }
    equalizer.update(controls.tone.snapshot());
    for _ in 0..1000 {
        assert_eq!(equalizer.process([0.2, -0.1]), [0.2, -0.1]);
    }
}

#[test]
fn long_clock_drift_is_bounded_and_tracks_real_source_rate() {
    // Exercise the real queue, resampler and callback path for a full minute
    // in each clock direction, using a low supported rate to keep tests quick.
    for drift in [-0.002, 0.002] {
        let (mut mixer, mut producers, controls, _) = new_mixer(8000, 60);
        controls[0].set_active(true);
        fill(&mut producers[0], 480, [0.1; 2]);
        let mut out = [[0.0; 2]; 80];
        let mut produced = 0.0_f64;
        let mut minimum = usize::MAX;
        let mut maximum = 0;
        for callback in 0..6000 {
            produced += 80.0 * (1.0 + drift);
            let frames = produced.floor() as usize;
            produced -= frames as f64;
            fill(&mut producers[0], frames, [0.1; 2]);
            mixer.render(&mut out);
            if callback > 3000 {
                let occupancy = mixer.tracks[0].consumer.slots();
                minimum = minimum.min(occupancy);
                maximum = maximum.max(occupancy);
            }
        }
        let correction = mixer.tracks[0].correction;
        eprintln!("drift {drift}: correction {correction:.7}, queue {minimum}..{maximum}");
        assert_eq!(controls[0].underruns(), 0);
        assert!(minimum > 200 && maximum < 800, "{minimum}..{maximum}");
        assert!(
            (correction - drift).abs() < 0.0001,
            "{correction} vs {drift}"
        );
        assert!(correction.abs() <= MAX_DRIFT);
    }
}

#[test]
fn batched_delivery_stops_repeatedly_starving_after_buffer_adapts() {
    // Model lossless Wi-Fi/driver delivery in 120 ms bursts, while the output
    // requests 10 ms blocks. A fixed 70 ms target keeps re-priming too early.
    let (mut mixer, mut producers, controls, _) = new_mixer(8000, 70);
    controls[0].set_active(true);
    let mut output = [[0.0; 2]; 80];
    let mut warmup_underruns = 0;
    for tick in 0..1200 {
        if tick % 12 == 0 {
            fill(&mut producers[0], 960, [0.1; 2]);
        }
        mixer.render(&mut output);
        if tick == 199 {
            warmup_underruns = controls[0].underruns();
        }
    }
    assert!(warmup_underruns > 0, "fixture did not exercise starvation");
    eprintln!(
        "120 ms bursts: {warmup_underruns} warmup underruns, final {} ms target",
        controls[0].target_buffer_ms()
    );
    assert_eq!(
        controls[0].underruns(),
        warmup_underruns,
        "lossless batched delivery must not keep cutting out after warmup"
    );
    settled(&output, [0.1 * db_to_linear(-6.0); 2], 1e-5);
    assert!(controls[0].target_buffer_ms() > 70 && controls[0].target_buffer_ms() <= 400);
    assert!(controls[0].primed());
    assert!(controls[0].buffered_frames() > 0);
}

#[test]
fn adaptive_buffer_is_bounded_and_session_reset_restores_initial_target() {
    let (mut mixer, mut producers, controls, _) = new_mixer(8000, 20);
    controls[0].set_active(true);
    let mut output = [[0.0; 2]; 80];
    for _ in 0..12 {
        fill(&mut producers[0], 4000, [0.1; 2]);
        let before = controls[0].underruns();
        for _ in 0..60 {
            mixer.render(&mut output);
        }
        assert_eq!(controls[0].underruns(), before + 1);
        assert!(controls[0].target_buffer_ms() <= 400);
    }
    assert_eq!(controls[0].target_buffer_ms(), 400);
    controls[0].reset();
    mixer.render(&mut output);
    assert_eq!(controls[0].target_buffer_ms(), 20);
    assert_eq!(controls[0].buffered_frames(), 0);
    assert!(!controls[0].primed());
    assert_eq!(controls[0].drift_ppm(), 0.0);
}

#[test]
fn render_allocates_nothing_during_audio_reconfiguration_or_reset() {
    let (mut mixer, mut producers, controls, master) = new_mixer(8000, 20);
    for i in 0..MAX_TRACKS {
        controls[i].set_sample_rate(192000);
        controls[i].set_active(true);
        controls[i].set_eq_enabled(true);
        controls[i].set_eq_band(4, 3.0);
        fill(&mut producers[i], 50_000, [0.001, -0.001]);
    }
    let mut out = [[0.0; 2]; 80];
    ALLOCATIONS.with(|count| count.set(Some(0)));
    // Includes the incremental sinc table builds, four halfband stages,
    // priming and EQ crossfades, and subsequent ordinary running audio.
    for _ in 0..16 {
        mixer.render(&mut out);
    }
    master.set_eq_enabled(true);
    master.set_eq_band(3, -6.0);
    for track in &controls {
        track.set_gain_db(-3.0);
        track.set_eq_band(4, -6.0);
    }
    mixer.render(&mut out);
    for track in &controls {
        track.reset();
    }
    mixer.render(&mut out);
    for track in &controls {
        track.set_active(false);
    }
    mixer.render(&mut out);
    let count = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(count, 0);
}
