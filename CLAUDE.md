# CLAUDE.md — RustyMixer project conventions

## Project overview

RustyMixer is a professional DJ mixing application built in Rust. It uses a Cargo workspace with 10 crates under `crates/`. The UI is built with Dioxus (desktop and web targets).

## Build & test commands

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test               # all tests
cargo test -p <crate>    # single crate tests
cargo clippy --workspace -- -D warnings  # lint
cargo build -p rustymixer-core --target wasm32-unknown-unknown  # WASM check
```

Useful aliases are defined in `.cargo/config.toml` (e.g. `cargo t`, `cargo c`, `cargo cl`).

## Workspace structure

All crates live in `crates/` and are prefixed with `rustymixer-`:
- **core** — fundamental audio types (`Sample`, `SampleRate`, `ChannelCount`, `SampleBuffer`, `TrackInfo`)
- **engine** — real-time mixing engine (deck management, sync, effects chain)
- **decode** — audio file decoding via Symphonia (`AudioDecoder` trait, `SymphoniaDecoder`)
- **io** — audio output backends (`AudioOutput` trait, `CpalOutput` for desktop, `WebAudioOutput` for WASM)
- **effects** — audio effect processors
- **library** — SQLite-backed music library
- **analysis** — BPM/key detection, waveform analysis
- **controllers** — MIDI/HID controller input
- **broadcast** — recording and streaming output
- **ui** — Dioxus application shell

## Code conventions

- **Edition**: Rust 2021, stable toolchain
- **Error handling**: Use `thiserror` for library error types, `anyhow` for application code
- **Logging**: Use `tracing` crate (not `log` or `println!`)
- **Concurrency**: Lock-free ring buffers (`ringbuf`) for real-time audio paths; never hold mutexes in audio callbacks
- **Sample format**: `f32` everywhere internally; convert at I/O boundaries only
- **Buffer size**: Max `MAX_ENGINE_FRAMES` (8192) frames per processing block
- **Channels**: Stereo interleaved (L, R, L, R, ...) throughout the engine

## Testing

- Unit tests go in the same file as the code (`#[cfg(test)]` module)
- Test audio fixtures live in `crates/rustymixer-decode/tests/fixtures/`
- All tests must pass with `cargo test`

## Architecture principles

- Traits define boundaries between crates (`AudioDecoder`, `AudioOutput`)
- The real-time audio thread must never allocate, lock, or do I/O
- WASM compatibility is maintained for `rustymixer-core` (no std-only deps)
