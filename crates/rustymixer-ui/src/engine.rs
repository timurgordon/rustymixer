//! Background audio engine — decodes audio and writes to the output device.
//!
//! The engine runs on a dedicated thread and communicates with the UI via
//! a crossbeam channel (commands in) and shared state (status out).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam::channel::Receiver;
use rustymixer_decode::{AudioDecoder, SymphoniaDecoder};
use rustymixer_io::{AudioConfig, AudioOutput, CpalOutput};

/// Commands sent from the UI to the audio engine.
pub enum PlayerCommand {
    Load(PathBuf),
    Play,
    Pause,
    Stop,
    Seek(f64),
    SetVolume(f32),
}

/// State shared between the engine thread and the UI.
pub struct SharedState {
    pub is_playing: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub volume: f32,
    pub track_title: String,
    pub track_artist: String,
    pub loaded: bool,
    pub error: Option<String>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            is_playing: false,
            position_secs: 0.0,
            duration_secs: 0.0,
            volume: 0.8,
            track_title: String::new(),
            track_artist: String::new(),
            loaded: false,
            error: None,
        }
    }
}

/// Spawn the audio engine on a named background thread.
pub fn spawn_engine(rx: Receiver<PlayerCommand>, state: Arc<Mutex<SharedState>>) {
    std::thread::Builder::new()
        .name("audio-engine".into())
        .spawn(move || engine_loop(rx, state))
        .expect("failed to spawn audio engine thread");
}

/// Number of stereo frames to decode per iteration.
const DECODE_FRAMES: usize = 1024;

fn engine_loop(rx: Receiver<PlayerCommand>, state: Arc<Mutex<SharedState>>) {
    // Create and start the audio output once.
    let config = AudioConfig::default();
    let mut output: Option<CpalOutput> = match CpalOutput::new(config) {
        Ok(mut out) => match out.start() {
            Ok(()) => Some(out),
            Err(e) => {
                tracing::error!("failed to start audio output: {e}");
                state.lock().unwrap().error = Some(format!("Audio start failed: {e}"));
                None
            }
        },
        Err(e) => {
            tracing::error!("no audio output device: {e}");
            state.lock().unwrap().error = Some(format!("No audio device: {e}"));
            None
        }
    };

    let mut decoder: Option<SymphoniaDecoder> = None;
    let mut playing = false;
    let mut volume: f32 = 0.8;
    let mut buf = vec![0.0f32; DECODE_FRAMES * 2];

    loop {
        // Drain all pending commands.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                PlayerCommand::Load(path) => {
                    playing = false;
                    match SymphoniaDecoder::open(&path) {
                        Ok(dec) => {
                            let info = dec.track_info();
                            let duration = info
                                .total_frames
                                .map(|f| f as f64 / info.sample_rate as f64)
                                .unwrap_or(0.0);
                            let title = info.title.clone().unwrap_or_else(|| {
                                path.file_stem()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default()
                            });
                            let artist = info.artist.clone().unwrap_or_default();
                            {
                                let mut s = state.lock().unwrap();
                                s.track_title = title;
                                s.track_artist = artist;
                                s.duration_secs = duration;
                                s.position_secs = 0.0;
                                s.loaded = true;
                                s.is_playing = false;
                                s.error = None;
                            }
                            decoder = Some(dec);
                            tracing::info!("loaded {}", path.display());
                        }
                        Err(e) => {
                            tracing::error!("failed to open {}: {e}", path.display());
                            state.lock().unwrap().error =
                                Some(format!("Failed to open file: {e}"));
                        }
                    }
                }
                PlayerCommand::Play => {
                    if decoder.is_some() && output.is_some() {
                        playing = true;
                        state.lock().unwrap().is_playing = true;
                    }
                }
                PlayerCommand::Pause => {
                    playing = false;
                    state.lock().unwrap().is_playing = false;
                }
                PlayerCommand::Stop => {
                    playing = false;
                    if let Some(ref mut dec) = decoder {
                        let _ = dec.seek(0);
                    }
                    let mut s = state.lock().unwrap();
                    s.is_playing = false;
                    s.position_secs = 0.0;
                }
                PlayerCommand::Seek(secs) => {
                    if let Some(ref mut dec) = decoder {
                        let sr = dec.track_info().sample_rate;
                        let frame = (secs * sr as f64) as u64;
                        if dec.seek(frame).is_ok() {
                            state.lock().unwrap().position_secs = secs;
                        }
                    }
                }
                PlayerCommand::SetVolume(v) => {
                    volume = v;
                    state.lock().unwrap().volume = v;
                }
            }
        }

        if playing {
            if let (Some(ref mut dec), Some(ref mut out)) = (&mut decoder, &mut output) {
                match dec.read_frames(&mut buf, DECODE_FRAMES) {
                    Ok(0) => {
                        // End of track.
                        playing = false;
                        let mut s = state.lock().unwrap();
                        s.is_playing = false;
                    }
                    Ok(frames) => {
                        let samples = &mut buf[..frames * 2];
                        // Apply volume gain.
                        for s in samples.iter_mut() {
                            *s *= volume;
                        }
                        // Write all samples to the ring buffer, busy-waiting if full.
                        let mut offset = 0;
                        while offset < samples.len() {
                            let n = out.write(&samples[offset..]);
                            offset += n;
                            if n == 0 {
                                std::thread::sleep(Duration::from_millis(1));
                            }
                        }
                        // Update position.
                        let sr = dec.track_info().sample_rate;
                        state.lock().unwrap().position_secs =
                            dec.position() as f64 / sr as f64;
                    }
                    Err(e) => {
                        tracing::error!("decode error: {e}");
                        playing = false;
                        let mut s = state.lock().unwrap();
                        s.is_playing = false;
                        s.error = Some(format!("Playback error: {e}"));
                    }
                }
            }
        } else {
            // Not playing — avoid busy-waiting.
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
