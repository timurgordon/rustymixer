//! Per-deck playback engine.
//!
//! Each deck in the mixer has one [`EngineBuffer`] that manages playback state,
//! position tracking, rate control, and seeking with crossfade smoothing.
//!
//! Inspired by Mixxx `src/engine/enginebuffer.h`.

use std::path::Path;
use std::sync::atomic::Ordering;

use crossbeam::channel::{self, Receiver, Sender};
use tracing::debug;

use rustymixer_core::audio::{FramePos, MAX_ENGINE_FRAMES};

use crate::caching_reader::{CachingReader, HintPriority, ReadHint};
use crate::channel::{ChannelId, ChannelOrientation, EngineChannel};
use crate::gain::AtomicF32;

/// Default crossfade length in frames for seek smoothing.
const CROSSFADE_FRAMES: usize = 64;

/// Command channel capacity.
const COMMAND_CHANNEL_CAPACITY: usize = 64;

/// Maximum read buffer size in frames. Supports up to 4x playback rate.
const MAX_READ_BUFFER_FRAMES: usize = MAX_ENGINE_FRAMES * 4 + 4;

/// Playback state for a single deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    /// No track loaded.
    Empty,
    /// Track loaded, not playing, position at 0.
    Stopped,
    /// Actively playing audio.
    Playing,
    /// Paused at current position.
    Paused,
}

/// Commands sent from the UI thread to the audio thread.
///
/// This is the internal representation. The public API is on [`EngineBufferHandle`].
enum InternalCommand {
    /// A track has been decoded and its reader is ready.
    TrackLoaded {
        reader: CachingReader,
        track_info: rustymixer_decode::TrackInfo,
    },
    Play,
    Pause,
    Stop,
    Seek(FramePos),
    SetRate(f32),
    SetGain(f32),
    Eject,
}

/// Errors from [`EngineBufferHandle`] operations.
#[derive(thiserror::Error, Debug)]
pub enum EngineBufferError {
    #[error("failed to load track: {0}")]
    LoadFailed(String),
    #[error("command channel full")]
    ChannelFull,
}

/// Handle for sending commands to an [`EngineBuffer`] from the UI thread.
///
/// Methods correspond to the `EngineCommand` variants from the design spec.
/// [`load_track`](Self::load_track) performs file I/O on the calling thread and
/// sends the ready-to-use [`CachingReader`] to the audio thread.
pub struct EngineBufferHandle {
    tx: Sender<InternalCommand>,
}

impl EngineBufferHandle {
    /// Load a track from disk.
    ///
    /// Opens the file and creates a [`CachingReader`] on the calling thread,
    /// then sends it to the audio thread. This performs I/O and must not be
    /// called from the real-time audio thread.
    pub fn load_track(&self, path: &Path) -> Result<(), EngineBufferError> {
        use rustymixer_decode::{AudioDecoder, SymphoniaDecoder};

        let decoder = SymphoniaDecoder::open(path)
            .map_err(|e| EngineBufferError::LoadFailed(e.to_string()))?;
        let track_info = decoder.track_info().clone();
        let reader = CachingReader::new(Box::new(decoder));
        self.tx
            .try_send(InternalCommand::TrackLoaded { reader, track_info })
            .map_err(|_| EngineBufferError::ChannelFull)?;
        Ok(())
    }

    /// Load a pre-created [`CachingReader`] directly.
    ///
    /// Useful for testing or when the caller has already created the reader.
    pub fn load_reader(
        &self,
        reader: CachingReader,
        track_info: rustymixer_decode::TrackInfo,
    ) -> Result<(), EngineBufferError> {
        self.tx
            .try_send(InternalCommand::TrackLoaded { reader, track_info })
            .map_err(|_| EngineBufferError::ChannelFull)?;
        Ok(())
    }

    pub fn play(&self) {
        let _ = self.tx.try_send(InternalCommand::Play);
    }

    pub fn pause(&self) {
        let _ = self.tx.try_send(InternalCommand::Pause);
    }

    pub fn stop(&self) {
        let _ = self.tx.try_send(InternalCommand::Stop);
    }

    pub fn seek(&self, pos: FramePos) {
        let _ = self.tx.try_send(InternalCommand::Seek(pos));
    }

    pub fn set_rate(&self, rate: f32) {
        let _ = self.tx.try_send(InternalCommand::SetRate(rate));
    }

    pub fn set_gain(&self, gain: f32) {
        let _ = self.tx.try_send(InternalCommand::SetGain(gain));
    }

    pub fn eject(&self) {
        let _ = self.tx.try_send(InternalCommand::Eject);
    }
}

/// Per-deck playback engine that manages a single deck's audio playback.
///
/// Handles position tracking, play/pause/stop, seeking with crossfade
/// smoothing, and rate control with linear interpolation. Implements
/// [`EngineChannel`] so it can be registered with the [`EngineMixer`].
///
/// [`EngineMixer`]: crate::EngineMixer
pub struct EngineBuffer {
    /// Unique channel identifier.
    channel_id: ChannelId,
    /// Current playback state.
    state: PlaybackState,
    /// Current play position in frames (fractional for sub-sample accuracy).
    play_pos: FramePos,
    /// Playback rate (1.0 = normal, 0.5 = half speed, 2.0 = double).
    rate: AtomicF32,
    /// Volume/gain for this deck.
    gain: AtomicF32,
    /// Crossfader orientation.
    orientation: ChannelOrientation,
    /// CachingReader for the loaded track.
    reader: Option<CachingReader>,
    /// Track metadata.
    track_info: Option<rustymixer_decode::TrackInfo>,
    /// Small crossfade buffer for seek smoothing (stereo interleaved).
    crossfade_buffer: Vec<f32>,
    /// Number of crossfade frames remaining after a seek.
    crossfade_remaining: usize,
    /// Receiver for commands from the UI thread.
    command_rx: Receiver<InternalCommand>,
    /// Pre-allocated buffer for reading from the CachingReader.
    read_buffer: Vec<f32>,
}

impl EngineBuffer {
    /// Create a new `EngineBuffer` and its [`EngineBufferHandle`].
    ///
    /// The handle is used to send commands from the UI thread.
    pub fn new(channel_id: ChannelId) -> (Self, EngineBufferHandle) {
        let (tx, rx) = channel::bounded(COMMAND_CHANNEL_CAPACITY);

        let buffer = Self {
            channel_id,
            state: PlaybackState::Empty,
            play_pos: FramePos::new(0.0),
            rate: AtomicF32::new(1.0),
            gain: AtomicF32::new(1.0),
            orientation: ChannelOrientation::Center,
            reader: None,
            track_info: None,
            crossfade_buffer: vec![0.0; CROSSFADE_FRAMES * 2],
            crossfade_remaining: 0,
            command_rx: rx,
            read_buffer: vec![0.0; MAX_READ_BUFFER_FRAMES * 2],
        };
        let handle = EngineBufferHandle { tx };
        (buffer, handle)
    }

    /// Current playback state.
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Current play position.
    pub fn play_pos(&self) -> FramePos {
        self.play_pos
    }

    /// Current playback rate.
    pub fn rate(&self) -> f32 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Track info for the currently loaded track.
    pub fn track_info(&self) -> Option<&rustymixer_decode::TrackInfo> {
        self.track_info.as_ref()
    }

    /// Set the crossfader orientation for this deck.
    pub fn set_orientation(&mut self, orientation: ChannelOrientation) {
        self.orientation = orientation;
    }

    /// Total frames in the loaded track, or 0 if none loaded.
    fn total_frames(&self) -> u64 {
        self.reader.as_ref().map_or(0, |r| r.total_frames())
    }

    /// Drain and process all pending commands. Non-blocking.
    fn process_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                InternalCommand::TrackLoaded { reader, track_info } => {
                    self.reader = Some(reader);
                    self.track_info = Some(track_info);
                    self.play_pos = FramePos::new(0.0);
                    self.state = PlaybackState::Stopped;
                    self.crossfade_remaining = 0;
                    debug!(id = self.channel_id.0, "track loaded");
                }
                InternalCommand::Play => {
                    if self.state == PlaybackState::Stopped
                        || self.state == PlaybackState::Paused
                    {
                        self.state = PlaybackState::Playing;
                        if let Some(reader) = &self.reader {
                            reader.hint(ReadHint {
                                frame: self.play_pos.value() as u64,
                                priority: HintPriority::CurrentPosition,
                            });
                        }
                        debug!(id = self.channel_id.0, "playing");
                    }
                }
                InternalCommand::Pause => {
                    if self.state == PlaybackState::Playing {
                        self.state = PlaybackState::Paused;
                        debug!(id = self.channel_id.0, "paused");
                    }
                }
                InternalCommand::Stop => {
                    if self.state != PlaybackState::Empty {
                        self.state = PlaybackState::Stopped;
                        self.play_pos = FramePos::new(0.0);
                        self.crossfade_remaining = 0;
                        debug!(id = self.channel_id.0, "stopped");
                    }
                }
                InternalCommand::Seek(pos) => {
                    if self.state != PlaybackState::Empty && self.reader.is_some() {
                        self.initiate_seek(pos);
                    }
                }
                InternalCommand::SetRate(r) => {
                    self.rate.store(r.max(0.0), Ordering::Relaxed);
                }
                InternalCommand::SetGain(g) => {
                    self.gain.store(g.max(0.0), Ordering::Relaxed);
                }
                InternalCommand::Eject => {
                    self.reader = None;
                    self.track_info = None;
                    self.state = PlaybackState::Empty;
                    self.play_pos = FramePos::new(0.0);
                    self.crossfade_remaining = 0;
                    debug!(id = self.channel_id.0, "ejected");
                }
            }
        }
    }

    /// Start a seek with crossfade: save current audio tail, jump to new pos.
    fn initiate_seek(&mut self, pos: FramePos) {
        if self.state == PlaybackState::Playing {
            if let Some(reader) = &self.reader {
                let current = self.play_pos.value() as u64;
                let start = current.saturating_sub(CROSSFADE_FRAMES as u64);
                self.crossfade_buffer.fill(0.0);
                reader.read(start, &mut self.crossfade_buffer, CROSSFADE_FRAMES);
                self.crossfade_remaining = CROSSFADE_FRAMES;
            }
        }

        self.play_pos = pos;

        if let Some(reader) = &self.reader {
            reader.hint(ReadHint {
                frame: pos.value() as u64,
                priority: HintPriority::CurrentPosition,
            });
        }

        debug!(id = self.channel_id.0, pos = pos.value(), "seeked");
    }

    /// Apply crossfade blending between old audio (pre-seek) and new audio.
    fn apply_crossfade(&mut self, buffer: &mut [f32], frames: usize) {
        let xf_frames = self.crossfade_remaining.min(frames);
        let xf_start = CROSSFADE_FRAMES - self.crossfade_remaining;

        for i in 0..xf_frames {
            let t = (xf_start + i + 1) as f32 / CROSSFADE_FRAMES as f32;
            let out_idx = i * 2;
            let xf_idx = (xf_start + i) * 2;

            if xf_idx + 1 < self.crossfade_buffer.len() {
                buffer[out_idx] =
                    self.crossfade_buffer[xf_idx] * (1.0 - t) + buffer[out_idx] * t;
                buffer[out_idx + 1] =
                    self.crossfade_buffer[xf_idx + 1] * (1.0 - t) + buffer[out_idx + 1] * t;
            }
        }

        self.crossfade_remaining = self.crossfade_remaining.saturating_sub(xf_frames);
    }
}

impl EngineChannel for EngineBuffer {
    fn process(&mut self, buffer: &mut [f32], frames: usize) -> bool {
        // 1. Drain command queue (non-blocking).
        self.process_commands();

        // 2. If not Playing, return false (silence).
        if self.state != PlaybackState::Playing {
            return false;
        }

        let reader = match &self.reader {
            Some(r) => r,
            None => return false,
        };

        let rate = self.rate.load(Ordering::Relaxed) as f64;
        if rate <= 0.0 {
            return false;
        }

        let play_pos = self.play_pos.value();
        let total = self.total_frames();

        // Check if already past end of track.
        if total > 0 && play_pos as u64 >= total {
            self.state = PlaybackState::Stopped;
            self.play_pos = FramePos::new(0.0);
            return false;
        }

        // 3. Read frames from CachingReader at current play_pos.
        let frac_start = play_pos - play_pos.floor();
        let source_frames_needed =
            (((frames as f64 * rate) + frac_start).ceil() as usize + 2)
                .min(MAX_READ_BUFFER_FRAMES);
        let start_frame = play_pos.floor() as u64;
        let frames_read = reader.read(
            start_frame,
            &mut self.read_buffer[..source_frames_needed * 2],
            source_frames_needed,
        );

        // Send lookahead hint for the next batch.
        reader.hint(ReadHint {
            frame: start_frame + frames_read as u64,
            priority: HintPriority::LookAhead,
        });

        if frames_read < 2 {
            // Not enough data for interpolation — cache miss or end of track.
            if total > 0 && start_frame + 2 >= total {
                self.state = PlaybackState::Stopped;
                self.play_pos = FramePos::new(0.0);
            }
            return false;
        }

        // 4. Resample with linear interpolation for rate control.
        //    For non-integer rates, interpolate between adjacent samples.
        let mut src_pos = frac_start;
        let mut out_frames = 0;

        for frame_idx in 0..frames {
            let src_frame = src_pos as usize;
            if src_frame + 1 >= frames_read {
                break;
            }

            let frac = (src_pos - src_frame as f64) as f32;
            let idx = src_frame * 2;

            buffer[frame_idx * 2] =
                self.read_buffer[idx] * (1.0 - frac) + self.read_buffer[idx + 2] * frac;
            buffer[frame_idx * 2 + 1] =
                self.read_buffer[idx + 1] * (1.0 - frac) + self.read_buffer[idx + 3] * frac;

            src_pos += rate;
            out_frames += 1;
        }

        // Fill remaining output with silence.
        buffer[out_frames * 2..frames * 2].fill(0.0);

        // 5. If crossfading (after seek), blend crossfade_buffer with new audio.
        if self.crossfade_remaining > 0 {
            self.apply_crossfade(buffer, out_frames);
        }

        // 6. Advance play_pos by actual source frames consumed.
        let source_advance = src_pos - frac_start;
        self.play_pos = FramePos::new(play_pos + source_advance);

        // 7. If past end of track, transition to Stopped.
        if total > 0 && self.play_pos.value() as u64 >= total {
            self.state = PlaybackState::Stopped;
            self.play_pos = FramePos::new(0.0);
        }

        out_frames > 0
    }

    fn gain(&self) -> f32 {
        self.gain.load(Ordering::Relaxed)
    }

    fn orientation(&self) -> ChannelOrientation {
        self.orientation
    }

    fn is_active(&self) -> bool {
        self.state == PlaybackState::Playing
    }

    fn id(&self) -> ChannelId {
        self.channel_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustymixer_core::audio::Sample;
    use rustymixer_decode::{AudioDecoder, DecodeError, TrackInfo};
    use std::thread;
    use std::time::Duration;

    // ---- Mock decoder -------------------------------------------------------

    /// Deterministic mock decoder that produces sample values derived from the
    /// frame position: `value = (frame * 0.001) % 1.0`.
    struct MockDecoder {
        total_frames: u64,
        position: u64,
        track_info: TrackInfo,
    }

    impl MockDecoder {
        fn new(total_frames: u64) -> Self {
            Self {
                total_frames,
                position: 0,
                track_info: TrackInfo {
                    sample_rate: 44100,
                    channels: 2,
                    total_frames: Some(total_frames),
                    title: Some("Test Track".into()),
                    artist: None,
                    album: None,
                },
            }
        }

        fn sample_for_frame(frame: u64) -> f32 {
            (frame as f32 * 0.001) % 1.0
        }
    }

    impl AudioDecoder for MockDecoder {
        fn total_frames(&self) -> Option<u64> {
            Some(self.total_frames)
        }

        fn track_info(&self) -> &TrackInfo {
            &self.track_info
        }

        fn read_frames(
            &mut self,
            output: &mut [Sample],
            max_frames: usize,
        ) -> rustymixer_decode::Result<usize> {
            let remaining = self.total_frames.saturating_sub(self.position);
            let frames = (max_frames as u64).min(remaining) as usize;
            for i in 0..frames {
                let val = Self::sample_for_frame(self.position + i as u64);
                output[i * 2] = val;
                output[i * 2 + 1] = val;
            }
            self.position += frames as u64;
            Ok(frames)
        }

        fn seek(&mut self, pos: u64) -> rustymixer_decode::Result<u64> {
            if pos > self.total_frames {
                return Err(DecodeError::Seek(format!(
                    "position {pos} beyond end {}",
                    self.total_frames
                )));
            }
            self.position = pos;
            Ok(pos)
        }

        fn position(&self) -> u64 {
            self.position
        }
    }

    // ---- Helpers ------------------------------------------------------------

    fn wait_for_worker() {
        thread::sleep(Duration::from_millis(300));
    }

    /// Create a test EngineBuffer pre-loaded with a mock track.
    fn create_loaded_buffer(total_frames: u64) -> (EngineBuffer, EngineBufferHandle) {
        let (mut eb, handle) = EngineBuffer::new(ChannelId(1));
        let decoder = Box::new(MockDecoder::new(total_frames));
        let track_info = decoder.track_info().clone();
        let reader = CachingReader::with_config(decoder, 1024, 32);
        wait_for_worker();
        handle.load_reader(reader, track_info).unwrap();
        // Process once to drain the TrackLoaded command.
        let mut scratch = vec![0.0f32; 256];
        eb.process(&mut scratch, 128);
        (eb, handle)
    }

    // ---- Tests --------------------------------------------------------------

    #[test]
    fn state_transitions() {
        let (mut eb, handle) = EngineBuffer::new(ChannelId(1));
        let mut buf = vec![0.0f32; 256];

        // Initial state: Empty.
        assert_eq!(eb.state(), PlaybackState::Empty);

        // Load track → Stopped.
        let decoder = Box::new(MockDecoder::new(44100));
        let info = decoder.track_info().clone();
        let reader = CachingReader::with_config(decoder, 1024, 16);
        wait_for_worker();
        handle.load_reader(reader, info).unwrap();
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Stopped);

        // Play → Playing.
        handle.play();
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Playing);

        // Pause → Paused.
        handle.pause();
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Paused);

        // Play again → Playing.
        handle.play();
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Playing);

        // Stop → Stopped, position reset to 0.
        handle.stop();
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Stopped);
        assert_eq!(eb.play_pos().value(), 0.0);

        // Eject → Empty.
        handle.eject();
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Empty);
    }

    #[test]
    fn play_produces_audio() {
        let (mut eb, handle) = create_loaded_buffer(44100);

        handle.play();
        let mut buf = vec![0.0f32; 2048];
        let produced = eb.process(&mut buf, 1024);
        assert!(produced, "should produce audio when playing");
        assert!(
            buf[..2048].iter().any(|&s| s.abs() > 0.0001),
            "output should contain non-silent samples"
        );
    }

    #[test]
    fn silence_when_not_playing() {
        let (mut eb, _handle) = create_loaded_buffer(44100);

        // State is Stopped, should not produce audio.
        let mut buf = vec![1.0f32; 256];
        let produced = eb.process(&mut buf, 128);
        assert!(!produced, "stopped deck should not produce audio");
    }

    #[test]
    fn rate_control_double_speed() {
        let (mut eb, handle) = create_loaded_buffer(44100);

        // Play at rate 1.0, record position after 1024 frames.
        handle.play();
        handle.set_rate(1.0);
        let mut buf = vec![0.0f32; 2048];
        eb.process(&mut buf, 1024);
        let pos_normal = eb.play_pos().value();

        // Reset and play at rate 2.0.
        handle.stop();
        eb.process(&mut buf, 128);
        handle.play();
        handle.set_rate(2.0);
        eb.process(&mut buf, 1024);
        let pos_double = eb.play_pos().value();

        // At 2x rate, position should advance roughly twice as far.
        let ratio = pos_double / pos_normal;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "expected ~2x position advance, got ratio {ratio} \
             (normal={pos_normal}, double={pos_double})"
        );
    }

    #[test]
    fn end_of_track_stops_playback() {
        // Very short track: 512 frames.
        let (mut eb, handle) = create_loaded_buffer(512);

        handle.play();
        let mut buf = vec![0.0f32; 2048];

        // Process enough frames to exhaust the track.
        for _ in 0..10 {
            eb.process(&mut buf, 1024);
            if eb.state() == PlaybackState::Stopped {
                break;
            }
        }

        assert_eq!(
            eb.state(),
            PlaybackState::Stopped,
            "should stop at end of track"
        );
    }

    #[test]
    fn seek_changes_position() {
        let (mut eb, handle) = create_loaded_buffer(44100);

        handle.play();
        let mut buf = vec![0.0f32; 2048];
        eb.process(&mut buf, 1024);
        let pos_before = eb.play_pos().value();

        let seek_pos = FramePos::new(22050.0);
        handle.seek(seek_pos);
        // Give caching reader time to pre-read the new position.
        wait_for_worker();

        // Process several times to allow the cache to populate.
        for _ in 0..5 {
            eb.process(&mut buf, 1024);
        }

        let actual_pos = eb.play_pos().value();
        assert!(
            actual_pos >= 22050.0,
            "after seeking to 22050, position should be at or past 22050, got {actual_pos}"
        );
        assert!(
            (actual_pos - pos_before).abs() > 1000.0,
            "position should have jumped from {pos_before} to near 22050, got {actual_pos}"
        );
    }

    #[test]
    fn seek_crossfade_no_discontinuity() {
        let (mut eb, handle) = create_loaded_buffer(44100);

        handle.play();
        let mut buf = vec![0.0f32; 2048];
        // Play a bit to fill the crossfade source.
        eb.process(&mut buf, 1024);

        // Seek to a different position.
        handle.seek(FramePos::new(20000.0));
        wait_for_worker();

        // Process a small buffer (within the crossfade window).
        let mut xf_buf = vec![0.0f32; CROSSFADE_FRAMES * 2];
        eb.process(&mut xf_buf, CROSSFADE_FRAMES);

        // Check smoothness: consecutive samples should not jump by more
        // than a reasonable amount (no clicks).
        let mut max_jump = 0.0f32;
        for i in 1..CROSSFADE_FRAMES {
            let jump = (xf_buf[i * 2] - xf_buf[(i - 1) * 2]).abs();
            max_jump = max_jump.max(jump);
        }
        // A click would cause a jump > 0.5. Crossfaded audio should be smooth.
        assert!(
            max_jump < 0.5,
            "crossfade should produce smooth audio, max jump = {max_jump}"
        );
    }

    #[test]
    fn cross_thread_commands() {
        let (mut eb, handle) = create_loaded_buffer(44100);

        // Send commands from a separate thread.
        let handle_clone = {
            // EngineBufferHandle contains a Sender which is Send.
            // We need to move it into the thread.
            handle
        };

        let cmd_thread = thread::spawn(move || {
            handle_clone.play();
            thread::sleep(Duration::from_millis(10));
            handle_clone.set_rate(1.5);
            thread::sleep(Duration::from_millis(10));
            handle_clone.set_gain(0.8);
            thread::sleep(Duration::from_millis(10));
            handle_clone.pause();
            handle_clone
        });

        // Process in the "audio thread" while commands arrive.
        let mut buf = vec![0.0f32; 2048];
        for _ in 0..20 {
            eb.process(&mut buf, 1024);
            thread::sleep(Duration::from_millis(5));
        }

        let _handle = cmd_thread.join().unwrap();

        // After the command thread paused, state should be Paused.
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Paused);
    }

    #[test]
    fn gain_is_reported() {
        let (mut eb, handle) = create_loaded_buffer(44100);

        assert!((eb.gain() - 1.0).abs() < f32::EPSILON, "default gain is 1.0");

        handle.set_gain(0.5);
        let mut buf = vec![0.0f32; 256];
        eb.process(&mut buf, 128);
        assert!(
            (eb.gain() - 0.5).abs() < f32::EPSILON,
            "gain should update to 0.5"
        );
    }

    #[test]
    fn track_info_available_after_load() {
        let (eb, _handle) = create_loaded_buffer(44100);
        let info = eb.track_info().expect("track info should be present");
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.title.as_deref(), Some("Test Track"));
    }

    #[test]
    fn eject_clears_everything() {
        let (mut eb, handle) = create_loaded_buffer(44100);

        handle.play();
        let mut buf = vec![0.0f32; 2048];
        eb.process(&mut buf, 1024);
        assert!(eb.play_pos().value() > 0.0);

        handle.eject();
        eb.process(&mut buf, 128);
        assert_eq!(eb.state(), PlaybackState::Empty);
        assert_eq!(eb.play_pos().value(), 0.0);
        assert!(eb.track_info().is_none());
    }

    #[test]
    fn empty_deck_ignores_play() {
        let (mut eb, handle) = EngineBuffer::new(ChannelId(1));
        let mut buf = vec![0.0f32; 256];

        handle.play();
        eb.process(&mut buf, 128);
        assert_eq!(
            eb.state(),
            PlaybackState::Empty,
            "play on empty deck should be ignored"
        );
    }
}
