mod engine;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam::channel::Sender;
use dioxus::prelude::*;

use engine::{DeckCommand, DeckState, SharedState};
use rustymixer_engine::{AtomicF32, Crossfader, PlaybackState};

fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting RustyMixer");
    dioxus::launch(app);
}

/// Persistent engine state shared via Dioxus signal.
struct EngineCtx {
    shared: Arc<Mutex<SharedState>>,
    crossfader: Arc<Crossfader>,
    master_gain: Arc<AtomicF32>,
    deck_tx: [Sender<DeckCommand>; 2],
}

fn app() -> Element {
    let engine = use_signal(|| {
        let shared = Arc::new(Mutex::new(SharedState::default()));
        let crossfader = Arc::new(Crossfader::default());
        let master_gain = Arc::new(AtomicF32::new(0.8));

        let deck_tx = engine::spawn_engine(
            Arc::clone(&shared),
            Arc::clone(&crossfader),
            Arc::clone(&master_gain),
        );

        EngineCtx {
            shared,
            crossfader,
            master_gain,
            deck_tx,
        }
    });

    let mut deck_a = use_signal(DeckState::default);
    let mut deck_b = use_signal(DeckState::default);
    let mut crossfader_pos = use_signal(|| 0.0f32);
    let mut master_vol = use_signal(|| 0.8f32);
    let mut error_msg = use_signal(|| None::<String>);

    // Sync engine state to UI at ~20 Hz.
    use_future(move || async move {
        loop {
            {
                let eng = engine.read();
                let s = eng.shared.lock().unwrap();
                deck_a.set(s.decks[0].clone());
                deck_b.set(s.decks[1].clone());
                crossfader_pos.set(s.crossfader);
                master_vol.set(s.master_volume);
                error_msg.set(s.error.clone());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });

    let err = error_msg.read().clone();
    let xf = *crossfader_pos.read();
    let mv = *master_vol.read();
    let xf_slider = ((xf + 1.0) / 2.0 * 100.0) as i32;
    let mv_pct = (mv * 100.0) as u32;

    rsx! {
        style { {CSS} }
        div { class: "app",
            div { class: "header",
                h1 { "RustyMixer" }
            }

            if let Some(ref e) = err {
                div { class: "error-banner", "{e}" }
            }

            div { class: "decks",
                DeckPanel { deck_index: 0, state: deck_a, engine: engine, side: "a" }
                DeckPanel { deck_index: 1, state: deck_b, engine: engine, side: "b" }
            }

            div { class: "master-section",
                div { class: "crossfader-section",
                    div { class: "crossfader-labels",
                        span { "A" }
                        span { class: "crossfader-title", "Crossfader" }
                        span { "B" }
                    }
                    input {
                        r#type: "range",
                        class: "crossfader-slider",
                        min: "0",
                        max: "100",
                        value: "{xf_slider}",
                        oninput: move |evt: Event<FormData>| {
                            if let Ok(v) = evt.value().parse::<f32>() {
                                let pos = (v / 50.0) - 1.0;
                                engine.read().crossfader.set_position(pos);
                            }
                        },
                    }
                }
                div { class: "master-volume",
                    span { class: "master-label", "Master: {mv_pct}%" }
                    input {
                        r#type: "range",
                        class: "master-slider",
                        min: "0",
                        max: "100",
                        value: "{mv_pct}",
                        oninput: move |evt: Event<FormData>| {
                            if let Ok(v) = evt.value().parse::<f32>() {
                                engine.read().master_gain.store(v / 100.0, Ordering::Relaxed);
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn DeckPanel(deck_index: usize, state: Signal<DeckState>, engine: Signal<EngineCtx>, side: String) -> Element {
    let ds = state.read().clone();
    let playing = ds.state == PlaybackState::Playing;
    let has_track = ds.loaded;

    let track_display = if has_track {
        if ds.track_artist.is_empty() {
            if ds.track_title.is_empty() {
                "Unknown".to_string()
            } else {
                ds.track_title.clone()
            }
        } else {
            format!("{} \u{2014} {}", ds.track_artist, ds.track_title)
        }
    } else {
        "No track loaded".to_string()
    };

    let pos_text = format!("{} / {}", format_time(ds.position_secs), format_time(ds.duration_secs));
    let progress_pct = if ds.duration_secs > 0.0 {
        ds.position_secs / ds.duration_secs * 100.0
    } else {
        0.0
    };
    let vol_pct = (ds.volume * 100.0) as u32;
    let rate_pct = ((ds.rate - 1.0) * 100.0) as i32;
    let rate_display = if rate_pct >= 0 {
        format!("+{rate_pct}%")
    } else {
        format!("{rate_pct}%")
    };

    let deck_class = format!("deck deck-{side}");
    let deck_label = if side == "a" { "DECK A" } else { "DECK B" };

    rsx! {
        div { class: "{deck_class}",
            div { class: "deck-header",
                span { class: "deck-label", "{deck_label}" }
                button {
                    class: "btn btn-load",
                    onclick: move |_| {
                        let tx = engine.read().deck_tx[deck_index].clone();
                        std::thread::spawn(move || {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Audio", &["mp3", "flac", "wav", "ogg", "m4a", "aac", "aiff"])
                                .pick_file()
                            {
                                let _ = tx.send(DeckCommand::LoadTrack(path));
                            }
                        });
                    },
                    "Load"
                }
            }

            div { class: "track-info",
                div { class: "track-name", "{track_display}" }
            }

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
                        max: "{ds.duration_secs:.1}",
                        step: "0.1",
                        value: "{ds.position_secs:.1}",
                        disabled: !has_track,
                        oninput: move |evt: Event<FormData>| {
                            if let Ok(secs) = evt.value().parse::<f64>() {
                                let sr = state.read().sample_rate;
                                let frame = secs * sr as f64;
                                let _ = engine.read().deck_tx[deck_index].send(DeckCommand::Seek(frame));
                            }
                        },
                    }
                }
            }

            div { class: "controls",
                button {
                    class: if playing { "btn btn-transport active" } else { "btn btn-transport" },
                    disabled: !has_track,
                    onclick: move |_| {
                        let eng = engine.read();
                        if playing {
                            let _ = eng.deck_tx[deck_index].send(DeckCommand::Pause);
                        } else {
                            let _ = eng.deck_tx[deck_index].send(DeckCommand::Play);
                        }
                    },
                    if playing { "\u{23F8} Pause" } else { "\u{25B6} Play" }
                }
                button {
                    class: "btn btn-transport",
                    disabled: !has_track,
                    onclick: move |_| {
                        let _ = engine.read().deck_tx[deck_index].send(DeckCommand::Stop);
                    },
                    "\u{23F9} Stop"
                }
                button {
                    class: "btn btn-transport btn-cue",
                    disabled: !has_track,
                    onclick: move |_| {
                        let eng = engine.read();
                        let _ = eng.deck_tx[deck_index].send(DeckCommand::Stop);
                        let _ = eng.deck_tx[deck_index].send(DeckCommand::Seek(0.0));
                    },
                    "CUE"
                }
            }

            div { class: "slider-section",
                span { class: "slider-label", "Rate: {rate_display}" }
                input {
                    r#type: "range",
                    class: "rate-slider",
                    min: "-8",
                    max: "8",
                    step: "0.1",
                    value: "{(ds.rate - 1.0) * 100.0:.1}",
                    oninput: move |evt: Event<FormData>| {
                        if let Ok(v) = evt.value().parse::<f32>() {
                            let rate = 1.0 + v / 100.0;
                            let _ = engine.read().deck_tx[deck_index].send(DeckCommand::SetRate(rate));
                        }
                    },
                }
            }

            div { class: "slider-section",
                span { class: "slider-label", "Vol: {vol_pct}%" }
                input {
                    r#type: "range",
                    class: "volume-slider",
                    min: "0",
                    max: "100",
                    value: "{vol_pct}",
                    oninput: move |evt: Event<FormData>| {
                        if let Ok(v) = evt.value().parse::<f32>() {
                            let _ = engine.read().deck_tx[deck_index].send(DeckCommand::SetGain(v / 100.0));
                        }
                    },
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
    background: #121218;
    color: #e0e0e0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, monospace;
    user-select: none;
}
.app {
    max-width: 960px;
    margin: 0 auto;
    padding: 12px;
}
.header {
    text-align: center;
    padding: 10px 0;
    margin-bottom: 12px;
}
.header h1 {
    font-size: 22px;
    color: #b0b0b0;
    letter-spacing: 3px;
    text-transform: uppercase;
    font-weight: 300;
}
.error-banner {
    background: #3a1a1a;
    color: #ff6b6b;
    padding: 8px 12px;
    border-radius: 6px;
    margin-bottom: 12px;
    font-size: 13px;
    text-align: center;
}
.decks {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-bottom: 12px;
}
@media (max-width: 640px) {
    .decks { grid-template-columns: 1fr; }
}
.deck {
    background: #1a1a24;
    border-radius: 8px;
    padding: 16px;
    border-top: 3px solid #333;
}
.deck-a { border-top-color: #4a9eff; }
.deck-b { border-top-color: #ff6a4a; }
.deck-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 10px;
}
.deck-label {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 2px;
    color: #888;
}
.deck-a .deck-label { color: #4a9eff; }
.deck-b .deck-label { color: #ff6a4a; }
.track-info { margin-bottom: 10px; min-height: 22px; }
.track-name {
    font-size: 14px;
    color: #ccc;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.btn {
    padding: 6px 12px;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 12px;
    font-family: inherit;
    background: #2a2a3a;
    color: #ccc;
    transition: background 0.15s;
}
.btn:hover:not(:disabled) { background: #3a3a4f; }
.btn:disabled { opacity: 0.3; cursor: not-allowed; }
.btn-load { background: #1a3a1a; color: #8f8; }
.btn-load:hover:not(:disabled) { background: #2a5a2a; }
.controls {
    display: flex;
    gap: 6px;
    margin-bottom: 12px;
}
.btn-transport {
    flex: 1;
    padding: 8px 4px;
    font-size: 12px;
}
.btn-transport.active {
    background: #1a3050;
    color: #4a9eff;
}
.deck-b .btn-transport.active {
    background: #3a1a10;
    color: #ff6a4a;
}
.btn-cue { background: #3a3a1a; color: #ffcc4a; }
.btn-cue:hover:not(:disabled) { background: #4a4a2a; }
.position-section { margin-bottom: 12px; }
.position-text {
    text-align: center;
    font-size: 16px;
    font-variant-numeric: tabular-nums;
    margin-bottom: 4px;
    color: #aaa;
}
.progress-container {
    position: relative;
    height: 20px;
    background: #0f0f1a;
    border-radius: 3px;
    overflow: hidden;
}
.progress-fill {
    position: absolute;
    top: 0; left: 0;
    height: 100%;
    opacity: 0.25;
    transition: width 0.08s linear;
    pointer-events: none;
}
.deck-a .progress-fill { background: #4a9eff; }
.deck-b .progress-fill { background: #ff6a4a; }
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
    width: 3px; height: 20px;
    background: #fff;
    border: none;
    cursor: pointer;
}
.progress-slider::-webkit-slider-runnable-track {
    height: 20px;
    background: transparent;
}
.progress-slider:disabled { cursor: not-allowed; }
.slider-section {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
}
.slider-label {
    font-size: 12px;
    color: #888;
    min-width: 80px;
    font-variant-numeric: tabular-nums;
}
.rate-slider, .volume-slider {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    background: #0f0f1a;
    border-radius: 2px;
    cursor: pointer;
}
.rate-slider::-webkit-slider-thumb, .volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px; height: 14px;
    background: #555;
    border-radius: 50%;
    cursor: pointer;
    transition: background 0.15s;
}
.rate-slider::-webkit-slider-thumb:hover, .volume-slider::-webkit-slider-thumb:hover {
    background: #888;
}
.deck-a .volume-slider::-webkit-slider-thumb { background: #4a9eff; }
.deck-b .volume-slider::-webkit-slider-thumb { background: #ff6a4a; }
.master-section {
    background: #1a1a24;
    border-radius: 8px;
    padding: 16px;
}
.crossfader-section { margin-bottom: 12px; }
.crossfader-labels {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 13px;
    color: #888;
    margin-bottom: 6px;
    padding: 0 4px;
}
.crossfader-title {
    font-size: 11px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: #555;
}
.crossfader-slider {
    width: 100%;
    -webkit-appearance: none;
    appearance: none;
    height: 6px;
    background: linear-gradient(to right, #4a9eff, #333 45%, #333 55%, #ff6a4a);
    border-radius: 3px;
    cursor: pointer;
}
.crossfader-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 20px; height: 20px;
    background: #ddd;
    border-radius: 3px;
    cursor: pointer;
}
.master-volume {
    display: flex;
    align-items: center;
    gap: 12px;
}
.master-label {
    font-size: 12px;
    color: #888;
    min-width: 100px;
    font-variant-numeric: tabular-nums;
}
.master-slider {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    background: #0f0f1a;
    border-radius: 2px;
    cursor: pointer;
}
.master-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px; height: 14px;
    background: #aaa;
    border-radius: 50%;
    cursor: pointer;
}
"#;
