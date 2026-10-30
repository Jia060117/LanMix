//! LanMix patch: substitute owned silence for WASAPI SILENT packets.
use crate::{BackendSpecificError, SampleFormat, StreamError};
use windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_SILENT;

pub(super) struct CaptureBuffer {
    // Eight-byte alignment covers every sample type supported by WASAPI.
    silence: Vec<u64>,
    max_bytes: usize,
    bytes_per_frame: usize,
    sample_size: usize,
    invalid_packet_error: Option<StreamError>,
}

fn error(description: &str) -> StreamError {
    BackendSpecificError {
        description: description.into(),
    }
    .into()
}

impl CaptureBuffer {
    /// Called once before the capture event loop, never from a data callback.
    pub(super) fn new(
        max_frames: u32,
        bytes_per_frame: u16,
        format: SampleFormat,
    ) -> Result<Self, StreamError> {
        let bytes_per_frame = usize::from(bytes_per_frame);
        let sample_size = format.sample_size();
        if max_frames == 0 || bytes_per_frame == 0 || bytes_per_frame % sample_size != 0 {
            return Err(error("WASAPI capture returned an invalid buffer layout"));
        }
        let max_bytes = (max_frames as usize)
            .checked_mul(bytes_per_frame)
            .ok_or_else(|| error("WASAPI capture buffer size overflow"))?;
        let words = max_bytes
            .checked_add(7)
            .ok_or_else(|| error("WASAPI capture buffer size overflow"))?
            / 8;
        let mut silence = Vec::new();
        silence
            .try_reserve_exact(words)
            .map_err(|_| error("could not preallocate WASAPI capture silence"))?;
        // Unsigned PCM's equilibrium is its midpoint, not zero bits. Current
        // WASAPI supports U8; retain correct equilibrium for other unsigned
        // formats as well. Signed PCM and float use all-zero bits.
        let equilibrium = match format {
            SampleFormat::U8 => 0x8080_8080_8080_8080,
            SampleFormat::U16 => 0x8000_8000_8000_8000,
            SampleFormat::U32 => 0x8000_0000_8000_0000,
            SampleFormat::U64 => 0x8000_0000_0000_0000,
            _ => 0,
        };
        silence.resize(words, equilibrium);
        Ok(Self {
            silence,
            max_bytes,
            bytes_per_frame,
            sample_size,
            // Construct this before the event loop too: rejecting a malformed
            // packet must not allocate merely to create its error description.
            invalid_packet_error: Some(error(
                "WASAPI capture returned an oversized, null, or misaligned data buffer",
            )),
        })
    }

    /// Select without dereferencing the driver pointer or allocating. The
    /// caller retains WASAPI's packet until the callback has completed.
    pub(super) fn select(
        &mut self,
        driver: *mut u8,
        flags: u32,
        frames: u32,
    ) -> Result<(*mut (), usize), ()> {
        let bytes = (frames as usize)
            .checked_mul(self.bytes_per_frame)
            .ok_or(())?;
        if bytes > self.max_bytes {
            return Err(());
        }
        let pointer = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            // Do not read, write, or validate the driver pointer: it may be NULL
            // or point to stale values when SILENT is set.
            self.silence.as_mut_ptr().cast::<()>()
        } else {
            if driver.is_null() || driver as usize % self.sample_size != 0 {
                return Err(());
            }
            driver.cast::<()>()
        };
        Ok((pointer, bytes / self.sample_size))
    }

    pub(super) fn take_packet_error(&mut self) -> StreamError {
        // A rejected packet terminates capture, so this is taken at most once.
        self.invalid_packet_error
            .take()
            .unwrap_or(StreamError::DeviceNotAvailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{mem, ptr, slice};
    use windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY;

    #[test]
    fn silent_null_and_stale_packets_select_aligned_zeros() {
        let mut buffer = CaptureBuffer::new(16, 8, SampleFormat::F32).unwrap();
        let flags =
            (AUDCLNT_BUFFERFLAGS_SILENT.0 | AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0) as u32;
        let mut stale = [0.75_f32; 32];
        for driver in [
            ptr::null_mut(),
            stale.as_mut_ptr().cast::<u8>(),
            ptr::NonNull::<u8>::dangling().as_ptr(),
        ] {
            let (pointer, len) = buffer.select(driver, flags, 16).unwrap();
            assert_eq!(pointer as usize % mem::align_of::<u64>(), 0);
            assert_eq!(len, 32);
            // SAFETY: SILENT selects the owned, initialized 128-byte buffer.
            let samples = unsafe { slice::from_raw_parts(pointer.cast::<f32>(), len) };
            assert!(samples.iter().all(|&sample| sample == 0.0));
        }
        assert_eq!(stale, [0.75; 32]);
    }

    #[test]
    fn nonsilent_packets_preserve_pointer_length_and_contents() {
        let mut buffer = CaptureBuffer::new(16, 8, SampleFormat::F32).unwrap();
        let mut real = [0.25_f32; 32];
        let (pointer, len) = buffer
            .select(
                real.as_mut_ptr().cast(),
                AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32,
                7,
            )
            .unwrap();
        assert_eq!(pointer, real.as_mut_ptr().cast());
        assert_eq!(len, 14);
        assert_eq!(real, [0.25; 32]);
        assert!(buffer.select(ptr::null_mut(), 0, 1).is_err());
    }

    #[test]
    fn oversized_and_misaligned_packets_are_rejected_without_resizing() {
        let mut buffer = CaptureBuffer::new(16, 8, SampleFormat::F32).unwrap();
        let capacity = buffer.silence.capacity();
        assert!(buffer
            .select(ptr::null_mut(), AUDCLNT_BUFFERFLAGS_SILENT.0 as u32, 17)
            .is_err());
        assert!(buffer.select(ptr::NonNull::<u8>::dangling().as_ptr(), 0, 1).is_err());
        assert_eq!(buffer.silence.capacity(), capacity);
        assert!(CaptureBuffer::new(0, 8, SampleFormat::F32).is_err());
        assert!(CaptureBuffer::new(16, 7, SampleFormat::F32).is_err());
    }

    #[test]
    fn unsigned_eight_bit_silence_uses_pcm_equilibrium() {
        let mut buffer = CaptureBuffer::new(5, 1, SampleFormat::U8).unwrap();
        let (pointer, len) = buffer
            .select(ptr::null_mut(), AUDCLNT_BUFFERFLAGS_SILENT.0 as u32, 5)
            .unwrap();
        // SAFETY: SILENT selects owned initialized storage of at least 5 bytes.
        let samples = unsafe { slice::from_raw_parts(pointer.cast::<u8>(), len) };
        assert_eq!(samples, &[128; 5]);
    }
}
