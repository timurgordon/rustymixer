mod engine;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam::channel::Sender;
use dioxus::prelude::*;

use engine::{PlayerCommand, SharedState};

fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting RustyMixer");
    dioxus::launch(app);
}

/// Handle for sending commands to the engine and reading shared state.
#[derive(Clone)]
struct EngineHandle {
    tx: Sender<PlayerCommand>,
    shared: Arc<Mutex<SharedState>>,
}

fn app() -> Element {
    // Initialise engine once (signal stores the handle, never mutated).
    let engine = use_signal(|| {
        let (tx, rx) = crossbeam::channel::unbounded();
        let shared = Arc::new(Mutex::new(SharedState::default()));
        engine::spawn_engine(rx, shared.clone());
        EngineHandle { tx, shared }
    });

    // Reactive UI state synced from the engine's SharedState.
    let mut is_playing = use_signal(|| false);
    let mut position_secs = use_signal(|| 0.0f64);
    let mut duration_secs = use_signal(|| 0.0f64);
    let mut volume = use_signal(|| 0.8f32);
    let mut track_title = use_signal(|| String::new());
    let mut track_artist = use_signal(|| String::new());
    let mut loaded = use_signal(|| false);
    let mut error_msg = use_signal(|| None::<String>);

    // Sync engine state → UI signals at ~20 Hz.
    use_future(move || async move {
        loop {
            {
                let eng = engine.read();
                let s = eng.shared.lock().unwrap();
                is_playing.set(s.is_playing);
                position_secs.set(s.position_secs);
                duration_secs.set(s.duration_secs);
                volume.set(s.volume);
                track_title.set(s.track_title.clone());
                track_artist.set(s.track_artist.clone());
                loaded.set(s.loaded);
                error_msg.set(s.error.clone());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });

    // Read current values for rendering.
    let dur = *duration_secs.read();
    let pos = *position_secs.read();
    let vol = *volume.read();
    let playing = *is_playing.read();
    let has_track = *loaded.read();
    let title = track_title.read().clone();
    let artist = track_artist.read().clone();
    let err = error_msg.read().clone();

    let track_display = if has_track {
        if artist.is_empty() {
            title.clone()
        } else {
            format!("{artist} \u{2014} {title}")
        }
    } else {
        "No track loaded".to_string()
    };

    let pos_text = format!("{} / {}", format_time(pos), format_time(dur));
    let progress_pct = if dur > 0.0 { pos / dur * 100.0 } else { 0.0 };
    let vol_pct = (vol * 100.0) as u32;

    rsx! {
        style { {CSS} }
        div { class: "app",
            // Header
            div { class: "header",
                h1 { "RustyMixer" }
            }

            div { class: "player",
                // Track info
                div { class: "track-info",
                    span { class: "track-label", "Track: " }
                    span { class: "track-name", "{track_display}" }
                }

                // Error banner
                if let Some(ref e) = err {
                    div { class: "error", "{e}" }
                }

                // Transport controls
                div { class: "controls",
                    button {
                        class: "btn",
                        disabled: !has_track || playing,
                        onclick: move |_| {
                            let _ = engine.read().tx.send(PlayerCommand::Play);
                        },
                        "\u{25B6} Play"
                    }
                    button {
                        class: "btn",
                        disabled: !playing,
                        onclick: move |_| {
                            let _ = engine.read().tx.send(PlayerCommand::Pause);
                        },
                        "\u{23F8} Pause"
                    }
                    button {
                        class: "btn",
                        disabled: !has_track,
                        onclick: move |_| {
                            let _ = engine.read().tx.send(PlayerCommand::Stop);
                        },
                        "\u{23F9} Stop"
                    }
                    button {
                        class: "btn btn-open",
                        onclick: move |_| {
                            let tx = engine.read().tx.clone();
                            std::thread::spawn(move || {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Audio", &[
                                        "mp3", "flac", "wav", "ogg", "m4a", "aac", "aiff",
                                    ])
                                    .pick_file()
                                {
                                    let _ = tx.send(PlayerCommand::Load(path));
                                }
                            });
                        },
                        "\u{1F4C1} Open"
                    }
                }

                // Position + progress bar
                div { class: "position-section",
                    div { class: "position-text", "{pos_text}" }
                    div { class: "progress-container",
                        div {
                            class: "progress-fill",
                            style: "width: {progress_pct:.1}%",
                        }
                        input {
                            r#type: "range",
                            class: "progress-slider",
                            min: "0",
                            max: "{dur:.1}",
                            step: "0.1",
                            value: "{pos:.1}",
                            disabled: !has_track,
                            oninput: move |evt: Event<FormData>| {
                                if let Ok(secs) = evt.value().parse::<f64>() {
                                    let _ = engine.read().tx.send(PlayerCommand::Seek(secs));
                                }
                            },
                        }
                    }
                }

                // Volume slider
                div { class: "volume-section",
                    span { class: "volume-label", "Volume: {vol_pct}%" }
                    input {
                        r#type: "range",
                        class: "volume-slider",
                        min: "0",
                        max: "100",
                        value: "{vol_pct}",
                        oninput: move |evt: Event<FormData>| {
                            if let Ok(v) = evt.value().parse::<f32>() {
                                let _ = engine.read().tx.send(PlayerCommand::SetVolume(v / 100.0));
                            }
                        },
                    }
                }
            }
        }
    }
}

fn format_time(secs: f64) -> String {
    if secs.is_nan() || secs < 0.0 {
        return "00:00".to_string();
    }
    let total = secs as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m:02}:{s:02}")
}

const CSS: &str = r#"
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
    background: #1a1a2e;
    color: #e0e0e0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
}
.app { max-width: 560px; margin: 40px auto; padding: 0 16px; }
.header {
    text-align: center;
    padding: 16px 0;
    border-bottom: 1px solid #333;
    margin-bottom: 24px;
}
.header h1 { font-size: 28px; color: #4a9eff; letter-spacing: 1px; }
.player {
    background: #16213e;
    border-radius: 12px;
    padding: 24px;
}
.track-info { margin-bottom: 16px; font-size: 16px; }
.track-label { color: #888; }
.track-name { color: #fff; font-weight: 600; }
.error {
    background: #3a1a1a;
    color: #ff6b6b;
    padding: 8px 12px;
    border-radius: 6px;
    margin-bottom: 12px;
    font-size: 13px;
}
.controls { display: flex; gap: 8px; margin-bottom: 20px; flex-wrap: wrap; }
.btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    font-size: 14px;
    background: #2a3a5c;
    color: #e0e0e0;
    transition: background 0.15s;
}
.btn:hover:not(:disabled) { background: #3a4f7a; }
.btn:disabled { opacity: 0.4; cursor: not-allowed; }
.btn-open { background: #1a4a2e; }
.btn-open:hover:not(:disabled) { background: #2a6a3e; }
.position-section { margin-bottom: 20px; }
.position-text {
    text-align: center;
    font-size: 18px;
    font-variant-numeric: tabular-nums;
    margin-bottom: 8px;
    color: #ccc;
}
.progress-container {
    position: relative;
    height: 24px;
    background: #0f1a2e;
    border-radius: 4px;
    overflow: hidden;
}
.progress-fill {
    position: absolute;
    top: 0; left: 0;
    height: 100%;
    background: #4a9eff;
    opacity: 0.3;
    transition: width 0.1s linear;
    pointer-events: none;
}
.progress-slider {
    position: absolute;
    top: 0; left: 0;
    width: 100%; height: 100%;
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    cursor: pointer;
    margin: 0;
}
.progress-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px; height: 24px;
    background: #4a9eff;
    border: none;
    border-radius: 2px;
    cursor: pointer;
}
.progress-slider::-webkit-slider-runnable-track {
    height: 24px;
    background: transparent;
}
.progress-slider:disabled { cursor: not-allowed; }
.volume-section { display: flex; align-items: center; gap: 12px; }
.volume-label { font-size: 14px; color: #888; min-width: 110px; }
.volume-slider {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 6px;
    background: #0f1a2e;
    border-radius: 3px;
    cursor: pointer;
}
.volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 16px; height: 16px;
    background: #4a9eff;
    border-radius: 50%;
    cursor: pointer;
}
"#;
