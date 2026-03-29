/// Unique identifier for an engine channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId(pub u32);

/// Crossfader orientation for a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOrientation {
    Left,
    Center,
    Right,
}

/// A source of audio in the mixer (deck, sampler, etc.).
///
/// Implementations must be **real-time safe**: [`process`](EngineChannel::process)
/// must not allocate, lock, or perform blocking I/O.
pub trait EngineChannel: Send {
    /// Fill `buffer` with up to `frames` stereo-interleaved samples.
    /// Returns `true` if audio was actually produced.
    fn process(&mut self, buffer: &mut [f32], frames: usize) -> bool;

    /// Current channel gain (volume fader, typically 0.0..=1.0).
    fn gain(&self) -> f32;

    /// Crossfader orientation.
    fn orientation(&self) -> ChannelOrientation;

    /// Whether this channel is active (loaded track, playing).
    fn is_active(&self) -> bool;

    /// Unique channel identifier.
    fn id(&self) -> ChannelId;
}
