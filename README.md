# RustyMixer

Professional DJ mixing application built in Rust with a Dioxus UI.

## Architecture

RustyMixer is organized as a Cargo workspace with 10 crates:

| Crate | Description |
|---|---|
| `rustymixer-core` | Audio types, sample buffers, control system |
| `rustymixer-engine` | Real-time audio mixing engine |
| `rustymixer-decode` | Symphonia-based audio decoding (MP3, FLAC, WAV, OGG, AAC) |
| `rustymixer-io` | Audio I/O — cpal (desktop) and WebAudio (WASM) |
| `rustymixer-effects` | Audio effects processors (EQ, filter, delay, reverb) |
| `rustymixer-library` | SQLite music library and playlist management |
| `rustymixer-analysis` | BPM/key detection, waveform analysis |
| `rustymixer-controllers` | MIDI/HID controller support |
| `rustymixer-broadcast` | Recording and live streaming |
| `rustymixer-ui` | Dioxus desktop/web application |

## Prerequisites

- Rust stable toolchain (via `rust-toolchain.toml`)
- System audio libraries:
  - **macOS**: CoreAudio (included with Xcode)
  - **Linux**: `libasound2-dev` (ALSA)
  - **Windows**: WASAPI (included with Windows SDK)

## Build

```bash
# Debug build
cargo build

# Release build
cargo build --release

# WASM target (core crate)
cargo build -p rustymixer-core --target wasm32-unknown-unknown
```

## Test

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p rustymixer-core
```

## Run

```bash
# Desktop app via Dioxus CLI
dx serve --platform desktop

# Or directly
cargo run -p rustymixer-ui
```

## Project Status

Early development. Core audio types, decoding, and I/O backends are implemented. The mixing engine, effects, library, analysis, controllers, and broadcast crates are in progress.

## License

MIT