use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use cpal::{
    SampleFormat, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use lanmix_proto::{Header, MAX_DATAGRAM_BYTES, encode};
use rtrb::{Producer, RingBuffer};
use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod redundancy;
use redundancy::Redundancy;

struct CapturedFrame {
    samples: [f32; 2],
    index: u64,
}

#[derive(Parser)]
#[command(
    version,
    about = "LanMix: stream system audio as float PCM over your LAN"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List output endpoints (Windows loopback) and capture inputs.
    Devices,
    /// Capture system audio (Windows default) or an explicitly selected input/monitor.
    Send {
        #[arg(long)]
        to: SocketAddr,
        /// Windows output endpoint index from `devices`.
        #[arg(long, conflicts_with = "input")]
        output: Option<usize>,
        /// Input index; use a monitor source on Linux. Never silently selects a microphone.
        #[arg(long)]
        input: Option<usize>,
        #[arg(long, default_value_t = 1)]
        stream_id: u32,
        #[arg(long, default_value_t = 2.0)]
        packet_ms: f32,
        /// Send a byte-identical backup this many ms later (0 disables; doubles PCM traffic).
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=50))]
        repair_ms: u32,
        /// Stop after this many seconds (0 runs until Ctrl+C).
        #[arg(long, default_value_t = 0)]
        seconds: u64,
    },
    /// Generate a quiet sine wave to test the full network path without capturing audio.
    Tone {
        #[arg(long)]
        to: SocketAddr,
        #[arg(long, default_value_t = 440.0)]
        frequency: f32,
        #[arg(long, default_value_t = 48000)]
        sample_rate: u32,
        #[arg(long, default_value_t = 1)]
        stream_id: u32,
        /// Delayed identical backups (0 disables; doubles PCM traffic).
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(0..=50))]
        repair_ms: u32,
        #[arg(long, default_value_t = 0)]
        seconds: u64,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Devices => list_devices(),
        Command::Send {
            to,
            output,
            input,
            stream_id,
            packet_ms,
            repair_ms,
            seconds,
        } => send(to, output, input, stream_id, packet_ms, repair_ms, seconds),
        Command::Tone {
            to,
            frequency,
            sample_rate,
            stream_id,
            repair_ms,
            seconds,
        } => tone(to, frequency, sample_rate, stream_id, repair_ms, seconds),
    }
}

fn list_devices() -> Result<()> {
    println!("Output devices (reported OS formats; not a Bluetooth codec/DAC guarantee):");
    for device in lanmix_audio::output_devices()? {
        println!(
            "  [{}] {}{} | default {} Hz, {} ch, {}",
            device.index,
            device.name,
            if device.is_default { " [default]" } else { "" },
            device.default_rate,
            device.default_channels,
            device.default_format
        );
        for config in device.capabilities {
            println!(
                "      {} ch {}..{} Hz {}",
                config.channels, config.min_rate, config.max_rate, config.format
            );
        }
    }
    println!("Input devices (explicit --input; monitor sources depend on host configuration):");
    for (index, device) in cpal::default_host().input_devices()?.enumerate() {
        println!("  [{index}] {}", device.name().unwrap_or_default());
    }
    Ok(())
}

fn session_id() -> u64 {
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64)
        ^ ((std::process::id() as u64) << 32)
}

fn stop_flag() -> Result<Arc<AtomicBool>> {
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    ctrlc::set_handler(move || flag.store(false, Ordering::Release))?;
    Ok(running)
}

fn socket(to: SocketAddr) -> Result<UdpSocket> {
    let socket = UdpSocket::bind(if to.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" })?;
    socket.connect(to)?;
    socket.set_write_timeout(Some(Duration::from_millis(20)))?;
    Ok(socket)
}

fn send(
    to: SocketAddr,
    output: Option<usize>,
    input: Option<usize>,
    stream_id: u32,
    packet_ms: f32,
    repair_ms: u32,
    seconds: u64,
) -> Result<()> {
    ensure!(
        packet_ms.is_finite() && (0.5..=10.0).contains(&packet_ms),
        "packet-ms must be 0.5..10"
    );
    let host = cpal::default_host();
    let is_loopback = input.is_none();
    if is_loopback && !cfg!(target_os = "windows") {
        bail!(
            "automatic system loopback is implemented on Windows; on Linux select a configured monitor with --input; macOS system capture backend is not implemented yet"
        );
    }
    let device = if let Some(index) = input {
        host.input_devices()?
            .nth(index)
            .context("input index not found")?
    } else if let Some(index) = output {
        host.output_devices()?
            .nth(index)
            .context("output index not found")?
    } else {
        host.default_output_device()
            .context("no default output endpoint for loopback")?
    };
    let supported = if is_loopback {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let format = supported.sample_format();
    let config = supported.config();
    ensure!(
        (8000..=192000).contains(&config.sample_rate.0),
        "capture rate outside protocol range"
    );
    ensure!(
        config.channels <= 2,
        "capture endpoint is {}-channel; choose stereo in OS settings (surround downmix is not implemented)",
        config.channels
    );
    let packet_frames = ((config.sample_rate.0 as f32 * packet_ms / 1000.0) as usize)
        .clamp(1, (MAX_DATAGRAM_BYTES - 40) / 8);
    println!(
        "Capturing {} | {} Hz / {} ch / {:?} | {} frames per packet → {}",
        device.name()?,
        config.sample_rate.0,
        config.channels,
        format,
        packet_frames,
        to
    );
    let (producer, mut consumer) =
        RingBuffer::<CapturedFrame>::new(config.sample_rate.0 as usize / 2);
    let dropped = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicBool::new(false));
    let stream = match format {
        SampleFormat::F32 => {
            capture::<f32>(&device, &config, producer, dropped.clone(), errors.clone())?
        }
        SampleFormat::I16 => {
            capture::<i16>(&device, &config, producer, dropped.clone(), errors.clone())?
        }
        SampleFormat::I32 => {
            capture::<i32>(&device, &config, producer, dropped.clone(), errors.clone())?
        }
        _ => bail!("capture format {format:?} not supported yet"),
    };
    let socket = socket(to)?;
    let running = stop_flag()?;
    stream.play()?;
    let mut packet = [0u8; MAX_DATAGRAM_BYTES];
    let mut frames = [[0.0; 2]; (MAX_DATAGRAM_BYTES - 40) / 8];
    let mut filled = 0;
    let mut seq = 0u32;
    let mut frame_index = 0u64;
    let mut session = session_id();
    let mut sent = 0u64;
    let mut last_report = Instant::now();
    let mut send_errors = 0u64;
    let mut resyncs = 0u64;
    let mut backup_sent = 0u64;
    let mut backup_errors = 0u64;
    let mut partial_dropped = 0u64;
    let mut backups = Redundancy::new(repair_ms);
    println!(
        "Delayed PCM backup: {repair_ms} ms (0 = disabled); use a receiver network buffer of at least 70 ms for the default 20 ms repair"
    );
    let started = Instant::now();
    while running.load(Ordering::Acquire)
        && !errors.load(Ordering::Acquire)
        && (seconds == 0 || started.elapsed() < Duration::from_secs(seconds))
    {
        backups.flush(Instant::now(), |bytes| {
            if socket.send(bytes).is_ok() {
                backup_sent += 1;
            } else {
                backup_errors += 1;
            }
        });
        let current_dropped = dropped.load(Ordering::Relaxed);
        let queued = consumer.slots();
        if queued > config.sample_rate.0 as usize / 5 {
            // A stalled sender should return to live audio, not burst half a
            // second of old sound. A new session resets downstream time too.
            let stale = queued.saturating_sub(config.sample_rate.0 as usize / 20);
            if let Ok(chunk) = consumer.read_chunk(stale) {
                chunk.commit_all();
            }
            filled = 0;
            session = session_id();
            backups.clear();
            resyncs += 1;
        }
        while filled < packet_frames {
            match consumer.pop() {
                Ok(frame) => {
                    if filled == 0 {
                        frame_index = frame.index;
                    } else if frame.index != frame_index + filled as u64 {
                        // Each capture frame carries its own source-clock position.
                        // Drop an incomplete packet at a gap rather than falsifying time.
                        partial_dropped += filled as u64;
                        filled = 0;
                        frame_index = frame.index;
                    }
                    frames[filled] = frame.samples;
                    filled += 1;
                }
                Err(_) => break,
            }
        }
        if filled == packet_frames {
            let header = Header {
                session_id: session,
                stream_id,
                seq,
                frame_index,
                sample_rate: config.sample_rate.0,
                frame_count: filled as u16,
            };
            let len = encode(&header, &frames[..filled], &mut packet)?;
            if socket.send(&packet[..len]).is_ok() {
                sent += 1;
            } else {
                send_errors += 1;
            }
            backups.remember(&packet[..len], Instant::now());
            seq = seq.wrapping_add(1);
            frame_index += filled as u64;
            filled = 0;
        } else {
            thread::sleep(Duration::from_micros(500));
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            println!(
                "sent {sent} originals + {backup_sent} backups | original send errors {send_errors} | backup errors {backup_errors} | capture dropped {current_dropped} frames | partial dropped {partial_dropped} frames | resyncs {resyncs} | skipped backups {}",
                backups.skipped
            );
            last_report = Instant::now();
        }
    }
    drop(stream);
    println!(
        "Final: sent {sent} originals + {backup_sent} backups | original send errors {send_errors} | backup errors {backup_errors} | capture dropped {} frames | partial dropped {partial_dropped} frames | resyncs {resyncs}",
        dropped.load(Ordering::Relaxed)
    );
    ensure!(
        !errors.load(Ordering::Acquire),
        "capture stream failed; check the selected endpoint"
    );
    Ok(())
}

trait InputSample: cpal::SizedSample + Copy {
    fn to_float(self) -> f32;
}
impl InputSample for f32 {
    fn to_float(self) -> f32 {
        if self.is_finite() { self } else { 0.0 }
    }
}
impl InputSample for i16 {
    fn to_float(self) -> f32 {
        self as f32 / 32768.0
    }
}
impl InputSample for i32 {
    fn to_float(self) -> f32 {
        self as f64 as f32 / 2147483648.0
    }
}

fn capture<T: InputSample>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut producer: Producer<CapturedFrame>,
    dropped: Arc<AtomicU64>,
    errors: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    let channels = config.channels as usize;
    let mut index = 0u64;
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            for frame in data.chunks_exact(channels) {
                let left = frame[0].to_float();
                let right = if channels > 1 {
                    frame[1].to_float()
                } else {
                    left
                };
                if producer
                    .push(CapturedFrame {
                        samples: [left, right],
                        index,
                    })
                    .is_err()
                {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
                index = index.wrapping_add(1);
            }
        },
        move |_error| {
            errors.store(true, Ordering::Release);
        },
        None,
    )?)
}

fn tone(
    to: SocketAddr,
    frequency: f32,
    sample_rate: u32,
    stream_id: u32,
    repair_ms: u32,
    seconds: u64,
) -> Result<()> {
    ensure!(
        (8000..=192000).contains(&sample_rate),
        "sample rate must be 8000..192000"
    );
    ensure!(
        frequency.is_finite() && frequency > 0.0 && frequency < sample_rate as f32 / 2.0,
        "frequency must be below Nyquist"
    );
    let running = stop_flag()?;
    let socket = socket(to)?;
    let session = session_id();
    let count = (sample_rate as usize / 500).clamp(1, (MAX_DATAGRAM_BYTES - 40) / 8);
    let mut frames = vec![[0.0f32; 2]; count];
    let mut packet = [0u8; MAX_DATAGRAM_BYTES];
    let mut seq = 0u32;
    let mut index = 0u64;
    let mut backups = Redundancy::new(repair_ms);
    let mut backup_sent = 0u64;
    let mut backup_errors = 0u64;
    let started = Instant::now();
    println!("Test tone {frequency} Hz at -26 dBFS → {to}; Ctrl+C to stop");
    while running.load(Ordering::Acquire)
        && (seconds == 0 || started.elapsed() < Duration::from_secs(seconds))
    {
        backups.flush(Instant::now(), |bytes| {
            if socket.send(bytes).is_ok() {
                backup_sent += 1;
            } else {
                backup_errors += 1;
            }
        });
        for (offset, frame) in frames.iter_mut().enumerate() {
            let phase = std::f64::consts::TAU * frequency as f64 * (index + offset as u64) as f64
                / sample_rate as f64;
            *frame = [(phase.sin() * 0.05) as f32; 2];
        }
        let header = Header {
            session_id: session,
            stream_id,
            seq,
            frame_index: index,
            sample_rate,
            frame_count: count as u16,
        };
        let len = encode(&header, &frames, &mut packet)?;
        socket.send(&packet[..len])?;
        backups.remember(&packet[..len], Instant::now());
        seq = seq.wrapping_add(1);
        index += count as u64;
        let deadline = started + Duration::from_secs_f64(index as f64 / sample_rate as f64);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
    println!(
        "Sent {seq} originals ({index} frames) + {backup_sent} backups | backup errors {backup_errors}"
    );
    Ok(())
}
