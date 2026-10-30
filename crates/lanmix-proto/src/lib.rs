//! Allocation-free encoding and decoding for LanMix protocol v1.
//!
//! Every integer and floating-point sample is explicitly little-endian. Decoding
//! validates the complete datagram, including every sample, before returning a
//! borrowed [`Packet`]. Ordering and loss recovery belong to the caller.

#![forbid(unsafe_code)]

use core::fmt;

/// Fixed v1 wire header size, independent of Rust's in-memory `Header` layout.
pub const HEADER_BYTES: usize = 40;
/// Maximum complete wire datagram size, including its 40-byte header.
pub const MAX_DATAGRAM_BYTES: usize = 1200;
pub const STEREO_FRAME_BYTES: usize = 8;
pub const MAX_FRAMES: usize = (MAX_DATAGRAM_BYTES - HEADER_BYTES) / STEREO_FRAME_BYTES;
pub const MIN_SAMPLE_RATE: u32 = 8_000;
pub const MAX_SAMPLE_RATE: u32 = 192_000;

pub type StereoFrame = [f32; 2];

/// Caller-supplied fields of the fixed 40-byte v1 header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub session_id: u64,
    pub stream_id: u32,
    pub seq: u32,
    pub frame_index: u64,
    pub sample_rate: u32,
    pub frame_count: u16,
}

/// A validated header and borrowed, interleaved stereo payload.
#[derive(Clone, Copy, Debug)]
pub struct Packet<'a> {
    pub header: Header,
    payload: &'a [u8],
}

impl Packet<'_> {
    /// Iterate over decoded left/right samples without allocating.
    ///
    /// The iterator length comes from the validated payload, so modifying the
    /// public header after decoding cannot change its bounds.
    pub fn frames(&self) -> impl ExactSizeIterator<Item = StereoFrame> + '_ {
        self.payload
            .chunks_exact(STEREO_FRAME_BYTES)
            .map(read_frame)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    TruncatedHeader { actual: usize },
    DatagramTooLarge { actual: usize },
    InvalidMagic,
    UnsupportedVersion(u8),
    InvalidFlags(u8),
    InvalidChannels(u8),
    UnsupportedFormat(u8),
    InvalidReserved(u32),
    InvalidSampleRate(u32),
    FrameIndexOverflow { frame_index: u64, frame_count: u16 },
    TooManyFrames { actual: usize },
    FrameCountMismatch { header: u16, actual: usize },
    InvalidLength { expected: usize, actual: usize },
    BufferTooSmall { required: usize, available: usize },
    NonFiniteSample { frame: usize, channel: usize },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { actual } => {
                write!(f, "header requires {HEADER_BYTES} bytes, received {actual}")
            }
            Self::DatagramTooLarge { actual } => {
                write!(
                    f,
                    "datagram has {actual} bytes; maximum is {MAX_DATAGRAM_BYTES}"
                )
            }
            Self::InvalidMagic => f.write_str("invalid LanMix magic"),
            Self::UnsupportedVersion(value) => write!(f, "unsupported version {value}"),
            Self::InvalidFlags(value) => write!(f, "flags must be zero, received {value}"),
            Self::InvalidChannels(value) => write!(f, "expected 2 channels, received {value}"),
            Self::UnsupportedFormat(value) => {
                write!(f, "expected f32 LE format 1, received {value}")
            }
            Self::InvalidReserved(value) => {
                write!(f, "reserved field must be zero, received {value}")
            }
            Self::InvalidSampleRate(value) => {
                write!(
                    f,
                    "sample rate {value} is outside {MIN_SAMPLE_RATE}..={MAX_SAMPLE_RATE}"
                )
            }
            Self::FrameIndexOverflow {
                frame_index,
                frame_count,
            } => {
                write!(
                    f,
                    "frame index {frame_index} plus frame count {frame_count} overflows u64"
                )
            }
            Self::TooManyFrames { actual } => {
                write!(f, "frame count {actual} exceeds {MAX_FRAMES}")
            }
            Self::FrameCountMismatch { header, actual } => {
                write!(f, "header declares {header} frames, received {actual}")
            }
            Self::InvalidLength { expected, actual } => {
                write!(f, "expected {expected} datagram bytes, received {actual}")
            }
            Self::BufferTooSmall {
                required,
                available,
            } => {
                write!(f, "output requires {required} bytes, available {available}")
            }
            Self::NonFiniteSample { frame, channel } => {
                write!(f, "non-finite sample at frame {frame}, channel {channel}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

/// Encode a complete v1 datagram into caller-owned storage.
///
/// The frame count must match `frames.len()`. Samples must be finite, and sample
/// rates must be in `8000..=192000`. Validation finishes before any output is
/// written. On success, only the returned byte count is modified; any remaining
/// output capacity is left untouched. Zero-frame packets are permitted.
/// The exclusive end index `frame_index + frame_count` must fit in `u64`.
pub fn encode(
    header: &Header,
    frames: &[StereoFrame],
    out: &mut [u8],
) -> Result<usize, ProtocolError> {
    validate_header(header)?;
    if usize::from(header.frame_count) != frames.len() {
        return Err(ProtocolError::FrameCountMismatch {
            header: header.frame_count,
            actual: frames.len(),
        });
    }
    let required = HEADER_BYTES + frames.len() * STEREO_FRAME_BYTES;
    if out.len() < required {
        return Err(ProtocolError::BufferTooSmall {
            required,
            available: out.len(),
        });
    }
    validate_frames(frames.iter().copied())?;

    out[0..2].copy_from_slice(b"LM");
    out[2] = 1;
    out[3] = 0;
    out[4..12].copy_from_slice(&header.session_id.to_le_bytes());
    out[12..16].copy_from_slice(&header.stream_id.to_le_bytes());
    out[16..20].copy_from_slice(&header.seq.to_le_bytes());
    out[20..28].copy_from_slice(&header.frame_index.to_le_bytes());
    out[28..32].copy_from_slice(&header.sample_rate.to_le_bytes());
    out[32..34].copy_from_slice(&header.frame_count.to_le_bytes());
    out[34] = 2;
    out[35] = 1;
    out[36..40].fill(0);
    for (frame, bytes) in frames
        .iter()
        .zip(out[HEADER_BYTES..required].chunks_exact_mut(8))
    {
        bytes[..4].copy_from_slice(&frame[0].to_le_bytes());
        bytes[4..].copy_from_slice(&frame[1].to_le_bytes());
    }
    Ok(required)
}

/// Decode and strictly validate one complete v1 datagram without allocating.
///
/// Rejects unsupported fixed fields, invalid sample rates or frame counts,
/// truncated or trailing payload bytes, oversized datagrams, and all NaN or
/// infinite samples. The returned packet borrows the input bytes.
/// The exclusive end index `frame_index + frame_count` must fit in `u64`.
pub fn decode(bytes: &[u8]) -> Result<Packet<'_>, ProtocolError> {
    if bytes.len() < HEADER_BYTES {
        return Err(ProtocolError::TruncatedHeader {
            actual: bytes.len(),
        });
    }
    if bytes.len() > MAX_DATAGRAM_BYTES {
        return Err(ProtocolError::DatagramTooLarge {
            actual: bytes.len(),
        });
    }
    if &bytes[..2] != b"LM" {
        return Err(ProtocolError::InvalidMagic);
    }
    if bytes[2] != 1 {
        return Err(ProtocolError::UnsupportedVersion(bytes[2]));
    }
    if bytes[3] != 0 {
        return Err(ProtocolError::InvalidFlags(bytes[3]));
    }
    if bytes[34] != 2 {
        return Err(ProtocolError::InvalidChannels(bytes[34]));
    }
    if bytes[35] != 1 {
        return Err(ProtocolError::UnsupportedFormat(bytes[35]));
    }
    let reserved = read_u32(bytes, 36);
    if reserved != 0 {
        return Err(ProtocolError::InvalidReserved(reserved));
    }
    let header = Header {
        session_id: read_u64(bytes, 4),
        stream_id: read_u32(bytes, 12),
        seq: read_u32(bytes, 16),
        frame_index: read_u64(bytes, 20),
        sample_rate: read_u32(bytes, 28),
        frame_count: u16::from_le_bytes([bytes[32], bytes[33]]),
    };
    validate_header(&header)?;
    let expected = HEADER_BYTES + usize::from(header.frame_count) * STEREO_FRAME_BYTES;
    if bytes.len() != expected {
        return Err(ProtocolError::InvalidLength {
            expected,
            actual: bytes.len(),
        });
    }
    let packet = Packet {
        header,
        payload: &bytes[HEADER_BYTES..],
    };
    validate_frames(packet.frames())?;
    Ok(packet)
}

fn validate_header(header: &Header) -> Result<(), ProtocolError> {
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&header.sample_rate) {
        return Err(ProtocolError::InvalidSampleRate(header.sample_rate));
    }
    if usize::from(header.frame_count) > MAX_FRAMES {
        return Err(ProtocolError::TooManyFrames {
            actual: usize::from(header.frame_count),
        });
    }
    if header
        .frame_index
        .checked_add(u64::from(header.frame_count))
        .is_none()
    {
        return Err(ProtocolError::FrameIndexOverflow {
            frame_index: header.frame_index,
            frame_count: header.frame_count,
        });
    }
    Ok(())
}

fn validate_frames(frames: impl Iterator<Item = StereoFrame>) -> Result<(), ProtocolError> {
    for (frame, samples) in frames.enumerate() {
        for (channel, sample) in samples.into_iter().enumerate() {
            if !sample.is_finite() {
                return Err(ProtocolError::NonFiniteSample { frame, channel });
            }
        }
    }
    Ok(())
}

fn read_frame(bytes: &[u8]) -> StereoFrame {
    [
        f32::from_bits(read_u32(bytes, 0)),
        f32::from_bits(read_u32(bytes, 4)),
    ]
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
