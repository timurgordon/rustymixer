//! WebAudio output backend for WASM targets.
//!
//! Uses `AudioContext` + `ScriptProcessorNode` to play audio in the browser.
//! ScriptProcessorNode is deprecated but widely supported and simpler than
//! AudioWorklet for Phase 1.
//!
//! Architecture:
//! - A `ringbuf` ring buffer sits between the WASM application code and the
//!   `onaudioprocess` callback.
//! - The application writes interleaved f32 samples via [`WebAudioOutput::write`].
//! - The JS callback reads from the ring buffer and fills the output `AudioBuffer`.
//! - If the ring buffer is empty, silence is output (underrun).

use std::cell::RefCell;
use std::rc::Rc;

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapProd, HeapRb};
use wasm_bindgen::prelude::*;
use web_sys::{AudioContext, AudioContextOptions, AudioProcessingEvent, ScriptProcessorNode};

use crate::{AudioConfig, AudioError, AudioOutput};

/// Ring buffer capacity in samples (not frames).
/// 4 callback buffers worth of stereo samples gives comfortable headroom.
const RING_BUFFER_CAPACITY_MULTIPLIER: usize = 4;

/// WebAudio output backend using ScriptProcessorNode.
pub struct WebAudioOutput {
    config: AudioConfig,
    ctx: AudioContext,
    processor: Option<ScriptProcessorNode>,
    producer: Option<HeapProd<f32>>,
    /// Stored to prevent the closure from being dropped while the node is alive.
    _closure: Option<Closure<dyn FnMut(AudioProcessingEvent)>>,
    playing: bool,
}

impl WebAudioOutput {
    /// Create a new WebAudioOutput with the given configuration.
    ///
    /// This creates the `AudioContext` but does NOT start playback.
    /// Call [`start()`](AudioOutput::start) to begin.
    pub fn new(config: AudioConfig) -> Result<Self, AudioError> {
        let opts = AudioContextOptions::new();
        opts.set_sample_rate(config.sample_rate.hz() as f32);

        let ctx = AudioContext::new_with_context_options(&opts)
            .map_err(|e| AudioError::ContextCreation(format!("{e:?}")))?;

        Ok(Self {
            config,
            ctx,
            processor: None,
            producer: None,
            _closure: None,
            playing: false,
        })
    }

    /// Create a WebAudioOutput with default configuration.
    pub fn new_default() -> Result<Self, AudioError> {
        Self::new(AudioConfig::default())
    }

    /// Returns the actual sample rate negotiated with the browser.
    ///
    /// This may differ from the requested rate if the browser/OS doesn't support it.
    pub fn actual_sample_rate(&self) -> f32 {
        self.ctx.sample_rate()
    }

    /// Resume a suspended AudioContext.
    ///
    /// Browsers require a user gesture before audio can play. Call this from
    /// a click handler if [`start()`](AudioOutput::start) returns
    /// [`AudioError::Suspended`].
    pub fn resume_context(&self) -> Result<(), AudioError> {
        let _ = self
            .ctx
            .resume()
            .map_err(|e| AudioError::Backend(format!("resume failed: {e:?}")))?;
        Ok(())
    }
}

impl AudioOutput for WebAudioOutput {
    fn start(&mut self) -> Result<(), AudioError> {
        if self.playing {
            return Ok(());
        }

        // Check for suspended context (user gesture requirement).
        let state = self.ctx.state();
        if state == web_sys::AudioContextState::Suspended {
            return Err(AudioError::Suspended);
        }

        let channels = self.config.channels.count() as u32;
        let buffer_frames = self.config.buffer_frames as u32;

        // Create ScriptProcessorNode.
        // Args: buffer_size, input_channels (0 = no input), output_channels
        let processor = self
            .ctx
            .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
                buffer_frames,
                0,
                channels,
            )
            .map_err(|e| AudioError::Backend(format!("create_script_processor: {e:?}")))?;

        // Create ring buffer.
        let capacity =
            self.config.buffer_frames * channels as usize * RING_BUFFER_CAPACITY_MULTIPLIER;
        let rb = HeapRb::<f32>::new(capacity);
        let (producer, consumer) = rb.split();

        // Wrap consumer in Rc<RefCell<>> so the closure can own it.
        let consumer: Rc<RefCell<ringbuf::HeapCons<f32>>> = Rc::new(RefCell::new(consumer));

        // Build the onaudioprocess callback.
        let ch = channels as usize;
        let closure = Closure::wrap(Box::new(move |event: AudioProcessingEvent| {
            let output_buffer = event.output_buffer().unwrap();
            let frame_count = output_buffer.length() as usize;
            let mut cons = consumer.borrow_mut();

            // Read all interleaved samples from ring buffer at once.
            let total_samples = frame_count * ch;
            let mut interleaved = vec![0.0f32; total_samples];
            let read = cons.pop_slice(&mut interleaved);
            // Silence any samples we couldn't read (underrun).
            for s in &mut interleaved[read..] {
                *s = 0.0;
            }

            // De-interleave into per-channel AudioBuffer arrays.
            for channel_idx in 0..ch {
                let mut channel_data = output_buffer
                    .get_channel_data(channel_idx as u32)
                    .unwrap();
                for frame in 0..frame_count {
                    channel_data[frame] = interleaved[frame * ch + channel_idx];
                }
                output_buffer
                    .copy_to_channel(&channel_data, channel_idx as i32)
                    .unwrap();
            }
        }) as Box<dyn FnMut(AudioProcessingEvent)>);

        processor.set_onaudioprocess(Some(closure.as_ref().unchecked_ref()));

        // Connect processor → destination.
        processor
            .connect_with_audio_node(&self.ctx.destination())
            .map_err(|e| AudioError::Backend(format!("connect: {e:?}")))?;

        self.processor = Some(processor);
        self.producer = Some(producer);
        self._closure = Some(closure);
        self.playing = true;

        tracing::info!(
            sample_rate = self.ctx.sample_rate(),
            channels = channels,
            buffer_frames = buffer_frames,
            "WebAudio output started"
        );

        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if !self.playing {
            return Ok(());
        }

        // Disconnect and drop the processor node.
        if let Some(ref processor) = self.processor {
            processor.set_onaudioprocess(None);
            let _ = processor.disconnect();
        }

        self.processor = None;
        self.producer = None;
        self._closure = None;
        self.playing = false;

        tracing::info!("WebAudio output stopped");
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

impl Drop for WebAudioOutput {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = self.ctx.close();
    }
}
