//! Desktop audio output backend using cpal.
//!
//! Architecture mirrors [`super::web::WebAudioOutput`]: a lock-free SPSC ring
//! buffer sits between the application and the real-time audio callback.
//!
//!   application → [`write()`](AudioOutput::write) → ring buffer → cpal callback → speakers
//!
//! The cpal callback runs on a dedicated real-time thread — it performs **no
//! allocations, no locks, and no blocking**.  When the ring buffer is empty the
//! callback outputs silence (underrun).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig};
use ringbuf::{
    traits::{Consumer, Producer, Split},
    HeapCons, HeapProd, HeapRb,
};

use crate::{AudioConfig, AudioError, AudioOutput};

/// Ring buffer capacity as a multiple of callback buffer size.
/// 4× gives comfortable headroom before underruns occur.
const RING_BUFFER_CAPACITY_MULTIPLIER: usize = 4;

/// Desktop audio output backend powered by [cpal](https://docs.rs/cpal).
pub struct CpalOutput {
    config: AudioConfig,
    device: Device,
    sample_format: SampleFormat,
    stream: Option<cpal::Stream>,
    producer: Option<HeapProd<f32>>,
    playing: bool,
}

impl CpalOutput {
    /// Create a new output targeting the system default audio device.
    ///
    /// This probes the device but does **not** start playback.
    /// Call [`start()`](AudioOutput::start) when ready.
    pub fn new(config: AudioConfig) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::Backend("no output device available".into()))?;

        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Backend(format!("default_output_config: {e}")))?;

        tracing::info!(
            device = device.name().unwrap_or_else(|_| "<unknown>".into()),
            format = ?supported.sample_format(),
            "cpal output created"
        );

        Ok(Self {
            config,
            sample_format: supported.sample_format(),
            device,
            stream: None,
            producer: None,
            playing: false,
        })
    }

    /// Create a new output targeting a specific device by name.
    pub fn new_with_device(config: AudioConfig, device_name: &str) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .output_devices()
            .map_err(|e| AudioError::Backend(e.to_string()))?
            .find(|d| d.name().map(|n| n == device_name).unwrap_or(false))
            .ok_or(AudioError::Backend(format!(
                "device not found: {device_name}"
            )))?;

        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Backend(format!("default_output_config: {e}")))?;

        Ok(Self {
            config,
            sample_format: supported.sample_format(),
            device,
            stream: None,
            producer: None,
            playing: false,
        })
    }

    /// List the names of all available output devices on the default host.
    pub fn list_devices() -> Result<Vec<String>, AudioError> {
        let host = cpal::default_host();
        let devices = host
            .output_devices()
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        Ok(devices.filter_map(|d| d.name().ok()).collect())
    }
}

impl AudioOutput for CpalOutput {
    fn start(&mut self) -> Result<(), AudioError> {
        if self.playing {
            return Ok(());
        }

        let channels = self.config.channels.count() as u16;
        let sample_rate = self.config.sample_rate.hz();
        let buffer_frames = self.config.buffer_frames;

        let stream_config = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Fixed(buffer_frames as u32),
        };

        // Ring buffer: multiple callback buffers worth of stereo samples.
        let capacity = buffer_frames * channels as usize * RING_BUFFER_CAPACITY_MULTIPLIER;
        let rb = HeapRb::<f32>::new(capacity);
        let (producer, consumer) = rb.split();

        let stream = build_stream(&self.device, &stream_config, self.sample_format, consumer)?;
        stream
            .play()
            .map_err(|e| AudioError::Backend(format!("play: {e}")))?;

        self.stream = Some(stream);
        self.producer = Some(producer);
        self.playing = true;

        tracing::info!(
            sample_rate,
            channels,
            buffer_frames,
            capacity,
            "cpal output started"
        );

        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if !self.playing {
            return Ok(());
        }

        // Dropping the stream stops playback and releases the consumer.
        self.stream = None;
        self.producer = None;
        self.playing = false;

        tracing::info!("cpal output stopped");
        Ok(())
    }

    fn write(&mut self, samples: &[f32]) -> usize {
        match self.producer {
            Some(ref mut prod) => prod.push_slice(samples),
            None => 0,
        }
    }

    fn config(&self) -> &AudioConfig {
        &self.config
    }

    fn is_playing(&self) -> bool {
        self.playing
    }
}

impl Drop for CpalOutput {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// ---------------------------------------------------------------------------
// Stream construction — dispatches on the device's native sample format
// ---------------------------------------------------------------------------

fn build_stream(
    device: &Device,
    config: &StreamConfig,
    format: SampleFormat,
    consumer: HeapCons<f32>,
) -> Result<cpal::Stream, AudioError> {
    match format {
        SampleFormat::F32 => build_stream_typed::<f32>(device, config, consumer, write_f32),
        SampleFormat::I16 => build_stream_typed::<i16>(device, config, consumer, write_i16),
        SampleFormat::U16 => build_stream_typed::<u16>(device, config, consumer, write_u16),
        other => Err(AudioError::UnsupportedConfig(format!(
            "sample format {other:?}"
        ))),
    }
}

fn build_stream_typed<T: cpal::SizedSample + Send + 'static>(
    device: &Device,
    config: &StreamConfig,
    mut consumer: HeapCons<f32>,
    writer: fn(&mut HeapCons<f32>, &mut [T]),
) -> Result<cpal::Stream, AudioError> {
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                writer(&mut consumer, data);
            },
            |err| tracing::error!("cpal stream error: {err}"),
            None,
        )
        .map_err(|e| AudioError::Backend(format!("build_output_stream: {e}")))?;
    Ok(stream)
}

// ---------------------------------------------------------------------------
// Per-format callback writers — no allocations, no locks, no blocking
// ---------------------------------------------------------------------------

/// f32 → f32: direct ring buffer read.
fn write_f32(cons: &mut HeapCons<f32>, data: &mut [f32]) {
    let filled = cons.pop_slice(data);
    if filled < data.len() {
        // Underrun — zero the remainder.
        data[filled..].fill(0.0);
        if filled > 0 {
            tracing::warn!("audio underrun: {filled}/{} samples", data.len());
        }
    }
}

/// f32 → i16: read float samples, convert in a stack scratch buffer.
fn write_i16(cons: &mut HeapCons<f32>, data: &mut [i16]) {
    let mut buf = [0.0f32; 4096];
    let mut written = 0;

    while written < data.len() {
        let chunk = (data.len() - written).min(buf.len());
        let filled = cons.pop_slice(&mut buf[..chunk]);
        for i in 0..filled {
            data[written + i] = (buf[i].clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        }
        if filled < chunk {
            for s in &mut data[written + filled..] {
                *s = 0;
            }
            if written + filled > 0 {
                tracing::warn!(
                    "audio underrun: {}/{} samples",
                    written + filled,
                    data.len()
                );
            }
            return;
        }
        written += filled;
    }
}

/// f32 → u16: read float samples, convert in a stack scratch buffer.
fn write_u16(cons: &mut HeapCons<f32>, data: &mut [u16]) {
    const SILENCE: u16 = u16::MAX / 2;
    let mut buf = [0.0f32; 4096];
    let mut written = 0;

    while written < data.len() {
        let chunk = (data.len() - written).min(buf.len());
        let filled = cons.pop_slice(&mut buf[..chunk]);
        for i in 0..filled {
            let normalised = (buf[i].clamp(-1.0, 1.0) + 1.0) * 0.5;
            data[written + i] = (normalised * u16::MAX as f32) as u16;
        }
        if filled < chunk {
            for s in &mut data[written + filled..] {
                *s = SILENCE;
            }
            if written + filled > 0 {
                tracing::warn!(
                    "audio underrun: {}/{} samples",
                    written + filled,
                    data.len()
                );
            }
            return;
        }
        written += filled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_devices_does_not_panic() {
        let devices = CpalOutput::list_devices();
        assert!(devices.is_ok());
    }

    #[test]
    fn default_config_creates_output() {
        let result = CpalOutput::new(AudioConfig::default());
        match result {
            Ok(output) => {
                assert_eq!(output.config().sample_rate.hz(), 44100);
                assert!(!output.is_playing());
            }
            Err(AudioError::Backend(msg)) if msg.contains("no output device") => {
                // CI without audio hardware — acceptable.
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn start_stop_cycle() {
        let result = CpalOutput::new(AudioConfig::default());
        let mut output = match result {
            Ok(o) => o,
            Err(_) => return, // no device
        };

        // Before start, write returns 0.
        assert_eq!(output.write(&[0.0; 128]), 0);

        match output.start() {
            Ok(()) => {
                assert!(output.is_playing());

                // Push a short sine burst.
                let samples: Vec<f32> = (0..4410)
                    .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 44100.0).sin())
                    .collect();
                let pushed = output.write(&samples);
                assert!(pushed > 0);

                assert!(output.stop().is_ok());
                assert!(!output.is_playing());
            }
            Err(_) => {
                // Device may reject Fixed buffer size — acceptable.
            }
        }
    }
}
