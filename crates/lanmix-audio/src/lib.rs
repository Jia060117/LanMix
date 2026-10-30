//! Platform audio I/O. No networking is performed in an audio callback.
use anyhow::{Context, Result, bail};
use cpal::{
    Device, SampleFormat, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use lanmix_core::Mixer;
use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Serialize)]
pub struct FormatCapability {
    pub channels: u16,
    pub min_rate: u32,
    pub max_rate: u32,
    pub format: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OutputDeviceInfo {
    pub index: usize,
    pub name: String,
    pub is_default: bool,
    pub default_rate: u32,
    pub default_channels: u16,
    pub default_format: String,
    pub capabilities: Vec<FormatCapability>,
}

pub fn output_devices() -> Result<Vec<OutputDeviceInfo>> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    host.output_devices()?
        .enumerate()
        .map(|(index, device)| {
            let name = device.name().unwrap_or_else(|_| format!("Output {index}"));
            let config = device.default_output_config()?;
            let capabilities = device
                .supported_output_configs()?
                .map(|c| FormatCapability {
                    channels: c.channels(),
                    min_rate: c.min_sample_rate().0,
                    max_rate: c.max_sample_rate().0,
                    format: format!("{:?}", c.sample_format()),
                })
                .collect();
            Ok(OutputDeviceInfo {
                index,
                is_default: default_name.as_ref() == Some(&name),
                name,
                default_rate: config.sample_rate().0,
                default_channels: config.channels(),
                default_format: format!("{:?}", config.sample_format()),
                capabilities,
            })
        })
        .collect()
}

pub struct OutputSetup {
    device: Device,
    config: StreamConfig,
    format: SampleFormat,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
}

pub fn prepare_output(name: Option<&str>, rate: Option<u32>) -> Result<OutputSetup> {
    let host = cpal::default_host();
    let device = if let Some(name) = name {
        let mut devices = host
            .output_devices()?
            .filter(|device| device.name().ok().as_deref() == Some(name));
        let device = devices
            .next()
            .context("selected output is disconnected; refresh devices")?;
        if devices.next().is_some() {
            bail!(
                "multiple outputs share this name; rename endpoints in the OS or use the system default"
            );
        }
        device
    } else {
        host.default_output_device()
            .context("no default audio output device")?
    };
    let default = device.default_output_config()?;
    let requested_rate = rate.unwrap_or(default.sample_rate().0);
    let mut candidates: Vec<_> = device
        .supported_output_configs()?
        .filter(|c| {
            c.min_sample_rate().0 <= requested_rate
                && c.max_sample_rate().0 >= requested_rate
                && matches!(
                    c.sample_format(),
                    SampleFormat::F32 | SampleFormat::I16 | SampleFormat::I32
                )
        })
        .collect();
    candidates.sort_by_key(|c| {
        (
            if c.channels() == 2 {
                0
            } else if c.channels() == default.channels() {
                1
            } else {
                2
            },
            match c.sample_format() {
                SampleFormat::F32 => 0,
                SampleFormat::I32 => 1,
                _ => 2,
            },
        )
    });
    let chosen = candidates.into_iter().next().with_context(|| format!("device does not support {requested_rate} Hz in f32/i32/i16; select Auto or another device"))?
        .with_sample_rate(cpal::SampleRate(requested_rate));
    Ok(OutputSetup {
        device_name: device.name().unwrap_or_else(|_| "Unknown output".into()),
        sample_rate: chosen.sample_rate().0,
        channels: chosen.channels(),
        sample_format: format!("{:?}", chosen.sample_format()),
        format: chosen.sample_format(),
        config: chosen.config(),
        device,
    })
}

pub struct OutputStream {
    _stream: Stream,
    error: Arc<Mutex<Option<String>>>,
}

impl OutputStream {
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

impl OutputSetup {
    pub fn start(self, mixer: Mixer) -> Result<OutputStream> {
        let error = Arc::new(Mutex::new(None));
        let stream = match self.format {
            SampleFormat::F32 => build::<f32>(&self.device, &self.config, mixer, error.clone())?,
            SampleFormat::I16 => build::<i16>(&self.device, &self.config, mixer, error.clone())?,
            SampleFormat::I32 => build::<i32>(&self.device, &self.config, mixer, error.clone())?,
            _ => bail!("unsupported output sample format"),
        };
        stream.play()?;
        Ok(OutputStream {
            _stream: stream,
            error,
        })
    }
}

trait OutputSample: cpal::SizedSample {
    fn from_float(value: f32) -> Self;
}
impl OutputSample for f32 {
    fn from_float(value: f32) -> Self {
        value
    }
}
impl OutputSample for i16 {
    fn from_float(value: f32) -> Self {
        (value.clamp(-1.0, 1.0) * 32767.0).round() as i16
    }
}
impl OutputSample for i32 {
    fn from_float(value: f32) -> Self {
        (value.clamp(-1.0, 1.0) as f64 * 2147483647.0).round() as i32
    }
}

fn build<T: OutputSample>(
    device: &Device,
    config: &StreamConfig,
    mut mixer: Mixer,
    error: Arc<Mutex<Option<String>>>,
) -> Result<Stream> {
    let channels = config.channels as usize;
    let mut scratch = [[0.0f32; 2]; 512];
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            for chunk in data.chunks_mut(512 * channels) {
                let count = chunk.len() / channels;
                mixer.render(&mut scratch[..count]);
                for (frame, samples) in scratch[..count]
                    .iter()
                    .zip(chunk.chunks_exact_mut(channels))
                {
                    if channels == 1 {
                        samples[0] = T::from_float((frame[0] + frame[1]) * 0.5);
                    } else {
                        samples[0] = T::from_float(frame[0]);
                        samples[1] = T::from_float(frame[1]);
                        for sample in &mut samples[2..] {
                            *sample = T::from_float(0.0);
                        }
                    }
                }
            }
        },
        move |e| {
            *error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
        },
        None,
    )?)
}
