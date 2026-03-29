//! Two-deck audio engine using EngineBuffer, Crossfader, and CpalOutput.
//!
//! The engine runs on a dedicated thread. The UI sends commands via crossbeam
//! channels. Each deck is an EngineBuffer managed directly (not via EngineMixer)
//! so we can read playback state back to the UI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam::channel::{Receiver, Sender};
use rustymixer_core::audio::FramePos;
use rustymixer_engine::{
    apply_gain, AtomicF32, ChannelId, ChannelOrientation, Crossfader, EngineBuffer,
    EngineChannel, PlaybackState,
};
use rustymixer_io::{AudioConfig, AudioOutput, CpalOutput};

/// Commands sent from the UI to a specific deck.
pub enum DeckCommand {
    LoadTrack(PathBuf),
    Play,
    Pause,
    Stop,
    Seek(f64),
    SetRate(f32),
    SetGain(f32),
}

/// Per-deck state visible to the UI.
#[derive(Debug, Clone)]
pub struct DeckState {
    pub state: PlaybackState,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub volume: f32,
    pub rate: f32,
    pub track_title: String,
    pub track_artist: String,
    pub loaded: bool,
    pub sample_rate: u32,
    pub total_frames: u64,
}

impl Default for DeckState {
    fn default() -> Self {
        Self {
            state: PlaybackState::Empty,
            position_secs: 0.0,
            duration_secs: 0.0,
            volume: 0.8,
            rate: 1.0,
            track_title: String::new(),
            track_artist: String::new(),
            loaded: false,
            sample_rate: 44100,
            total_frames: 0,
        }
    }
}

/// Shared state between the engine thread and the UI.
pub struct SharedState {
    pub decks: [DeckState; 2],
    pub crossfader: f32,
    pub master_volume: f32,
    pub error: Option<String>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            decks: [DeckState::default(), DeckState::default()],
            crossfader: 0.0,
            master_volume: 0.8,
            error: None,
        }
    }
}

const PROCESS_FRAMES: usize = 1024;

/// Spawn the two-deck audio engine.
///
/// Returns per-deck command senders for the UI.
pub fn spawn_engine(
    state: Arc<Mutex<SharedState>>,
    crossfader: Arc<Crossfader>,
    master_gain: Arc<AtomicF32>,
) -> [Sender<DeckCommand>; 2] {
    let (tx_a, rx_a) = crossbeam::channel::unbounded::<DeckCommand>();
    let (tx_b, rx_b) = crossbeam::channel::unbounded::<DeckCommand>();

    std::thread::Builder::new()
        .name("audio-engine".into())
        .spawn(move || {
            engine_loop(rx_a, rx_b, state, crossfader, master_gain);
        })
        .expect("failed to spawn audio engine thread");

    [tx_a, tx_b]
}

fn engine_loop(
    rx_a: Receiver<DeckCommand>,
    rx_b: Receiver<DeckCommand>,
    state: Arc<Mutex<SharedState>>,
    crossfader: Arc<Crossfader>,
    master_gain: Arc<AtomicF32>,
) {
    // Create the two deck buffers and their handles.
    let (mut deck_a, handle_a) = EngineBuffer::new(ChannelId(0));
    let (mut deck_b, handle_b) = EngineBuffer::new(ChannelId(1));
    deck_a.set_orientation(ChannelOrientation::Left);
    deck_b.set_orientation(ChannelOrientation::Right);

    // Start audio output.
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

    let mut buf_a = vec![0.0f32; PROCESS_FRAMES * 2];
    let mut buf_b = vec![0.0f32; PROCESS_FRAMES * 2];
    let mut mix_buf = vec![0.0f32; PROCESS_FRAMES * 2];

    loop {
        // Drain UI commands for each deck.
        process_deck_commands(&rx_a, &handle_a, &state);
        process_deck_commands(&rx_b, &handle_b, &state);

        let frames = PROCESS_FRAMES;
        let samples = frames * 2;

        // Process each deck.
        buf_a[..samples].fill(0.0);
        buf_b[..samples].fill(0.0);
        let active_a = deck_a.process(&mut buf_a[..samples], frames);
        let active_b = deck_b.process(&mut buf_b[..samples], frames);

        // Apply crossfader gains.
        let (xf_left, xf_right) = crossfader.gains();
        if active_a {
            apply_gain(&mut buf_a[..samples], deck_a.gain() * xf_left);
        }
        if active_b {
            apply_gain(&mut buf_b[..samples], deck_b.gain() * xf_right);
        }

        // Mix and apply master gain.
        let mg = master_gain.load(std::sync::atomic::Ordering::Relaxed);
        for i in 0..samples {
            mix_buf[i] = (buf_a[i] + buf_b[i]) * mg;
        }

        // Write to audio output.
        if let Some(ref mut out) = output {
            let mut offset = 0;
            while offset < samples {
                let n = out.write(&mix_buf[offset..samples]);
                offset += n;
                if n == 0 {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }

        // Update shared state.
        if let Ok(mut s) = state.try_lock() {
            update_deck_state(&deck_a, &mut s.decks[0]);
            update_deck_state(&deck_b, &mut s.decks[1]);
            s.crossfader = crossfader.position();
            s.master_volume = mg;
        }
    }
}

fn process_deck_commands(
    rx: &Receiver<DeckCommand>,
    handle: &rustymixer_engine::EngineBufferHandle,
    state: &Arc<Mutex<SharedState>>,
) {
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            DeckCommand::LoadTrack(path) => {
                if let Err(e) = handle.load_track(&path) {
                    tracing::error!("failed to load track: {e}");
                    if let Ok(mut s) = state.lock() {
                        s.error = Some(format!("Failed to load: {e}"));
                    }
                }
            }
            DeckCommand::Play => handle.play(),
            DeckCommand::Pause => handle.pause(),
            DeckCommand::Stop => handle.stop(),
            DeckCommand::Seek(frame) => handle.seek(FramePos::new(frame)),
            DeckCommand::SetRate(r) => handle.set_rate(r),
            DeckCommand::SetGain(g) => handle.set_gain(g),
        }
    }
}

fn update_deck_state(deck: &EngineBuffer, ds: &mut DeckState) {
    ds.state = deck.state();
    ds.volume = deck.gain();
    ds.rate = deck.rate();

    if let Some(info) = deck.track_info() {
        ds.loaded = true;
        ds.sample_rate = info.sample_rate;
        ds.total_frames = info.total_frames.unwrap_or(0);
        ds.duration_secs = if info.sample_rate > 0 {
            ds.total_frames as f64 / info.sample_rate as f64
        } else {
            0.0
        };
        ds.track_title = info.title.clone().unwrap_or_default();
        ds.track_artist = info.artist.clone().unwrap_or_default();
        ds.position_secs = if info.sample_rate > 0 {
            deck.play_pos().value() / info.sample_rate as f64
        } else {
            0.0
        };
    } else {
        ds.loaded = false;
        ds.state = PlaybackState::Empty;
        ds.position_secs = 0.0;
        ds.duration_secs = 0.0;
        ds.track_title.clear();
        ds.track_artist.clear();
        ds.total_frames = 0;
    }
}
