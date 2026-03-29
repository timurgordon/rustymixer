//! Async audio file reading with chunk caching.
//!
//! The [`CachingReader`] pre-reads and caches audio data from a decoder in a
//! background worker thread, providing low-latency lock-free access to audio
//! frames for the real-time audio callback.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam::channel::{self, Receiver, Sender};
use tracing::{debug, trace};

use rustymixer_decode::AudioDecoder;

/// Default chunk size in frames.
const DEFAULT_CHUNK_SIZE: usize = 8192;

/// Default maximum number of cached chunks.
/// 64 chunks × 8192 frames ≈ 12 seconds at 44.1 kHz.
const DEFAULT_MAX_CHUNKS: usize = 64;

/// Number of chunks to pre-read ahead of the current position.
const LOOKAHEAD_CHUNKS: u64 = 3;

/// Capacity of the hint channel.
const HINT_CHANNEL_CAPACITY: usize = 32;

/// Priority level for a read hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintPriority {
    /// Highest priority: where playback is right now.
    CurrentPosition,
    /// Lower priority: upcoming chunks after current position.
    LookAhead,
}

/// A hint to the worker thread about what audio to pre-read.
#[derive(Debug, Clone, Copy)]
pub struct ReadHint {
    /// Frame position that will be needed.
    pub frame: u64,
    /// How urgently this position is needed.
    pub priority: HintPriority,
}

/// A single cache slot using a seqlock for lock-free reads.
///
/// The version counter implements a seqlock protocol:
/// - **Even** version = data is stable and safe to read
/// - **Odd** version = data is being written by the worker
///
/// Readers load the version before and after copying data. If both values
/// are equal and even, the read was consistent.
struct CacheSlot {
    /// Seqlock version counter (even = stable, odd = writing).
    version: AtomicU64,
    /// Which chunk index this slot holds.
    chunk_index: AtomicU64,
    /// Number of valid frames in this chunk.
    frame_count: AtomicU32,
    /// Whether this slot contains valid data.
    occupied: AtomicBool,
    /// LRU access counter for eviction.
    last_access: AtomicU64,
    /// Pre-allocated sample buffer (chunk_size × 2 for stereo interleaved).
    ///
    /// # Safety
    ///
    /// Access is guarded by the seqlock `version` counter:
    /// - The worker thread writes only when version is odd.
    /// - The audio thread reads only when version is even, and validates
    ///   by checking that version has not changed after the read.
    samples: UnsafeCell<Box<[f32]>>,
}

// Safety: CacheSlot is shared between the worker thread (writes) and the
// audio thread (reads). The seqlock version counter ensures that concurrent
// access never produces inconsistent results visible to the reader.
unsafe impl Send for CacheSlot {}
unsafe impl Sync for CacheSlot {}

impl CacheSlot {
    fn new(chunk_size: usize) -> Self {
        Self {
            version: AtomicU64::new(0),
            chunk_index: AtomicU64::new(u64::MAX),
            frame_count: AtomicU32::new(0),
            occupied: AtomicBool::new(false),
            last_access: AtomicU64::new(0),
            samples: UnsafeCell::new(vec![0.0f32; chunk_size * 2].into_boxed_slice()),
        }
    }
}

/// Fixed-size LRU cache mapping chunk indices to decoded audio data.
struct ChunkCache {
    slots: Box<[CacheSlot]>,
    chunk_size: usize,
    access_counter: AtomicU64,
}

impl ChunkCache {
    fn new(max_chunks: usize, chunk_size: usize) -> Self {
        let slots: Vec<CacheSlot> = (0..max_chunks).map(|_| CacheSlot::new(chunk_size)).collect();
        Self {
            slots: slots.into_boxed_slice(),
            chunk_size,
            access_counter: AtomicU64::new(1),
        }
    }

    /// Find a slot that holds the given chunk and is in a stable (readable) state.
    fn find_ready_slot(&self, chunk_index: u64) -> Option<&CacheSlot> {
        self.slots.iter().find(|slot| {
            slot.occupied.load(Ordering::Relaxed)
                && slot.chunk_index.load(Ordering::Relaxed) == chunk_index
                && slot.version.load(Ordering::Acquire) % 2 == 0
        })
    }

    /// Read frames from the cache. Lock-free, suitable for the audio thread.
    ///
    /// Returns the number of frames actually read. Returns 0 on cache miss.
    fn read(&self, pos: u64, output: &mut [f32], frames: usize) -> usize {
        let mut frames_read = 0;
        let mut current_pos = pos;

        while frames_read < frames {
            let chunk_index = current_pos / self.chunk_size as u64;
            let offset_in_chunk = (current_pos % self.chunk_size as u64) as usize;

            let slot = match self.find_ready_slot(chunk_index) {
                Some(s) => s,
                None => break,
            };

            // Update LRU counter.
            let access = self.access_counter.fetch_add(1, Ordering::Relaxed);
            slot.last_access.store(access, Ordering::Relaxed);

            // Seqlock read: capture version before.
            let v1 = slot.version.load(Ordering::Acquire);
            if v1 % 2 != 0 {
                break; // Slot is being written — treat as cache miss.
            }

            let valid_frames = slot.frame_count.load(Ordering::Relaxed) as usize;
            let available = valid_frames.saturating_sub(offset_in_chunk);
            let to_copy = available.min(frames - frames_read);

            if to_copy == 0 {
                break;
            }

            // Copy sample data.
            let src_start = offset_in_chunk * 2;
            let dst_start = frames_read * 2;
            // Safety: version is even so the worker is not writing to this slot.
            // We validate with the version check below.
            unsafe {
                let samples = &*slot.samples.get();
                output[dst_start..dst_start + to_copy * 2]
                    .copy_from_slice(&samples[src_start..src_start + to_copy * 2]);
            }

            // Seqlock read: verify version after.
            let v2 = slot.version.load(Ordering::Acquire);
            if v1 != v2 || slot.chunk_index.load(Ordering::Relaxed) != chunk_index {
                break; // Data changed during read — treat as cache miss.
            }

            frames_read += to_copy;
            current_pos += to_copy as u64;
        }

        frames_read
    }

    /// Check if a chunk is cached and ready.
    fn is_cached(&self, chunk_index: u64) -> bool {
        self.find_ready_slot(chunk_index).is_some()
    }

    // ---- Worker-thread methods (single-threaded, only called by the worker) ----

    /// Find an available slot, evicting the LRU entry if the cache is full.
    /// Only called from the worker thread.
    fn allocate_slot(&self) -> &CacheSlot {
        // Prefer an empty slot.
        for slot in self.slots.iter() {
            if !slot.occupied.load(Ordering::Relaxed) {
                return slot;
            }
        }

        // Evict the least-recently-used slot.
        let mut min_access = u64::MAX;
        let mut min_idx = 0;
        for (i, slot) in self.slots.iter().enumerate() {
            // Skip slots being written (shouldn't happen from the same worker,
            // but be defensive).
            if slot.version.load(Ordering::Relaxed) % 2 != 0 {
                continue;
            }
            let access = slot.last_access.load(Ordering::Relaxed);
            if access < min_access {
                min_access = access;
                min_idx = i;
            }
        }

        &self.slots[min_idx]
    }

    /// Load a chunk into the cache. Only called from the worker thread.
    fn load_chunk(
        &self,
        decoder: &mut dyn AudioDecoder,
        chunk_index: u64,
        decode_buffer: &mut [f32],
    ) -> bool {
        if self.is_cached(chunk_index) {
            return true;
        }

        let slot = self.allocate_slot();

        // Begin write: increment version to odd.
        let old_version = slot.version.fetch_add(1, Ordering::AcqRel);
        debug_assert!(
            old_version.is_multiple_of(2),
            "version should be even before write"
        );

        slot.chunk_index.store(chunk_index, Ordering::Relaxed);
        slot.occupied.store(true, Ordering::Relaxed);

        // Seek the decoder if needed.
        let target_frame = chunk_index * self.chunk_size as u64;
        if decoder.position() != target_frame && decoder.seek(target_frame).is_err() {
            slot.occupied.store(false, Ordering::Relaxed);
            slot.version.fetch_add(1, Ordering::Release);
            return false;
        }

        // Decode audio into the temporary buffer.
        decode_buffer[..self.chunk_size * 2].fill(0.0);
        let frames_read = match decoder.read_frames(decode_buffer, self.chunk_size) {
            Ok(n) => n,
            Err(e) => {
                trace!("decode error for chunk {chunk_index}: {e}");
                slot.occupied.store(false, Ordering::Relaxed);
                slot.version.fetch_add(1, Ordering::Release);
                return false;
            }
        };

        // Copy decoded samples into the slot.
        // Safety: version is odd so the audio thread will not read this slot.
        unsafe {
            let samples = &mut *slot.samples.get();
            samples[..frames_read * 2].copy_from_slice(&decode_buffer[..frames_read * 2]);
            if frames_read < self.chunk_size {
                samples[frames_read * 2..self.chunk_size * 2].fill(0.0);
            }
        }
        slot.frame_count
            .store(frames_read as u32, Ordering::Relaxed);

        // End write: increment version to even.
        slot.version.fetch_add(1, Ordering::Release);

        true
    }
}

/// Asynchronous audio file reader with chunk caching.
///
/// Runs a background worker thread that decodes audio ahead of the play
/// position and stores decoded chunks in a lock-free cache. The audio
/// callback reads from the cache via [`read`](Self::read) without blocking.
pub struct CachingReader {
    cache: Arc<ChunkCache>,
    hint_sender: Sender<ReadHint>,
    total_frames: u64,
    chunk_size: usize,
    shutdown: Arc<AtomicBool>,
    /// Worker thread handle, only used on drop. Wrapped in Mutex so that
    /// CachingReader is Sync (the Mutex is never locked on the hot path).
    worker_handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl CachingReader {
    /// Create a new `CachingReader` for the given decoder.
    ///
    /// Spawns a background worker that immediately begins pre-reading from
    /// the start of the track.
    pub fn new(decoder: Box<dyn AudioDecoder>) -> Self {
        Self::with_config(decoder, DEFAULT_CHUNK_SIZE, DEFAULT_MAX_CHUNKS)
    }

    /// Create with custom chunk size and cache capacity.
    pub fn with_config(
        decoder: Box<dyn AudioDecoder>,
        chunk_size: usize,
        max_chunks: usize,
    ) -> Self {
        let total_frames = decoder.total_frames().unwrap_or(0);
        let cache = Arc::new(ChunkCache::new(max_chunks, chunk_size));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (hint_sender, hint_receiver) = channel::bounded(HINT_CHANNEL_CAPACITY);

        let worker_cache = Arc::clone(&cache);
        let worker_shutdown = Arc::clone(&shutdown);

        let worker_handle = thread::Builder::new()
            .name("caching-reader-worker".into())
            .spawn(move || {
                worker_loop(
                    decoder,
                    hint_receiver,
                    worker_cache,
                    worker_shutdown,
                    chunk_size,
                    total_frames,
                );
            })
            .expect("failed to spawn caching reader worker thread");

        // Kick off initial pre-read from the beginning of the track.
        let _ = hint_sender.try_send(ReadHint {
            frame: 0,
            priority: HintPriority::CurrentPosition,
        });

        Self {
            cache,
            hint_sender,
            total_frames,
            chunk_size,
            shutdown,
            worker_handle: Mutex::new(Some(worker_handle)),
        }
    }

    /// Read frames starting at the given position.
    ///
    /// Returns the number of frames actually read (may be less if chunks are
    /// not yet cached). **Lock-free** — safe to call from the audio thread.
    pub fn read(&self, pos: u64, output: &mut [f32], frames: usize) -> usize {
        self.cache.read(pos, output, frames)
    }

    /// Send a hint about where audio will be needed next.
    ///
    /// Non-blocking (`try_send`) — safe to call from the audio thread.
    pub fn hint(&self, hint: ReadHint) {
        let _ = self.hint_sender.try_send(hint);
    }

    /// Total frames in the track.
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Chunk size in frames.
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Check whether a specific frame position is cached and ready to read.
    pub fn is_cached(&self, frame: u64) -> bool {
        let chunk_index = frame / self.chunk_size as u64;
        self.cache.is_cached(chunk_index)
    }
}

impl Drop for CachingReader {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut guard) = self.worker_handle.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

/// Background worker loop that decodes audio ahead of the play position.
fn worker_loop(
    mut decoder: Box<dyn AudioDecoder>,
    hint_receiver: Receiver<ReadHint>,
    cache: Arc<ChunkCache>,
    shutdown: Arc<AtomicBool>,
    chunk_size: usize,
    total_frames: u64,
) {
    let mut decode_buffer = vec![0.0f32; chunk_size * 2];
    let total_chunks = if total_frames > 0 {
        total_frames.div_ceil(chunk_size as u64)
    } else {
        u64::MAX
    };

    debug!(
        chunk_size,
        total_frames, total_chunks, "caching reader worker started"
    );

    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        match hint_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(hint) => {
                process_hint(&hint, &mut *decoder, &cache, &mut decode_buffer, total_chunks);

                // Drain and process remaining queued hints.
                while let Ok(hint) = hint_receiver.try_recv() {
                    if shutdown.load(Ordering::Relaxed) {
                        debug!("caching reader worker shutting down");
                        return;
                    }
                    process_hint(
                        &hint,
                        &mut *decoder,
                        &cache,
                        &mut decode_buffer,
                        total_chunks,
                    );
                }
            }
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam::channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    debug!("caching reader worker shutting down");
}

/// Process a single hint: load the target chunk and lookahead chunks.
fn process_hint(
    hint: &ReadHint,
    decoder: &mut dyn AudioDecoder,
    cache: &ChunkCache,
    decode_buffer: &mut [f32],
    total_chunks: u64,
) {
    let chunk_index = hint.frame / cache.chunk_size as u64;

    cache.load_chunk(decoder, chunk_index, decode_buffer);

    // Pre-read lookahead chunks.
    for i in 1..=LOOKAHEAD_CHUNKS {
        let la = chunk_index + i;
        if la >= total_chunks {
            break;
        }
        cache.load_chunk(decoder, la, decode_buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustymixer_core::audio::Sample;
    use rustymixer_decode::{DecodeError, TrackInfo};

    // ---- Mock decoder -------------------------------------------------------

    /// Deterministic mock decoder that produces sample values derived from
    /// the frame position, making it easy to verify correctness.
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

        /// Deterministic sample value for a given frame.
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

    // ---- helpers ------------------------------------------------------------

    /// Give the worker thread time to process hints and decode.
    fn wait_for_worker() {
        thread::sleep(Duration::from_millis(300));
    }

    // ---- tests --------------------------------------------------------------

    #[test]
    fn sequential_read() {
        let decoder = Box::new(MockDecoder::new(44100));
        let reader = CachingReader::with_config(decoder, 1024, 16);
        wait_for_worker();

        let mut output = vec![0.0f32; 1024 * 2];
        let frames = reader.read(0, &mut output, 1024);
        assert_eq!(frames, 1024);

        for i in 0..frames {
            let expected = MockDecoder::sample_for_frame(i as u64);
            assert!(
                (output[i * 2] - expected).abs() < f32::EPSILON,
                "frame {i}: expected {expected}, got {}",
                output[i * 2]
            );
        }
    }

    #[test]
    fn cache_hit_returns_immediately() {
        let decoder = Box::new(MockDecoder::new(44100));
        let reader = CachingReader::with_config(decoder, 1024, 16);
        wait_for_worker();

        let mut out1 = vec![0.0f32; 512 * 2];
        let n1 = reader.read(0, &mut out1, 512);
        assert_eq!(n1, 512);

        // Second read from the same chunk — should succeed immediately.
        let mut out2 = vec![0.0f32; 512 * 2];
        let n2 = reader.read(0, &mut out2, 512);
        assert_eq!(n2, 512);
        assert_eq!(out1, out2);
    }

    #[test]
    fn cache_miss_returns_zero() {
        // 60 seconds of audio, tiny cache (4 chunks × 1024 frames).
        let decoder = Box::new(MockDecoder::new(44100 * 60));
        let reader = CachingReader::with_config(decoder, 1024, 4);

        // Read far ahead before the worker can possibly cache it.
        let mut output = vec![0.0f32; 1024 * 2];
        let frames = reader.read(44100 * 50, &mut output, 1024);
        assert!(
            frames < 1024,
            "expected cache miss, got {frames} frames"
        );
    }

    #[test]
    fn seek_triggers_preread() {
        let decoder = Box::new(MockDecoder::new(44100 * 10));
        let reader = CachingReader::with_config(decoder, 1024, 32);

        let seek_pos = 44100 * 5;
        reader.hint(ReadHint {
            frame: seek_pos,
            priority: HintPriority::CurrentPosition,
        });
        wait_for_worker();

        let mut output = vec![0.0f32; 1024 * 2];
        let frames = reader.read(seek_pos, &mut output, 1024);
        assert_eq!(frames, 1024, "after hint, chunk should be cached");

        for i in 0..frames {
            let expected = MockDecoder::sample_for_frame(seek_pos + i as u64);
            assert!(
                (output[i * 2] - expected).abs() < f32::EPSILON,
                "frame {i}: expected {expected}, got {}",
                output[i * 2]
            );
        }
    }

    #[test]
    fn concurrent_read_and_hint() {
        let decoder = Box::new(MockDecoder::new(44100 * 5));
        let reader = Arc::new(CachingReader::with_config(decoder, 1024, 32));
        wait_for_worker();

        let reader_clone = Arc::clone(&reader);
        let read_thread = thread::spawn(move || {
            let mut output = vec![0.0f32; 1024 * 2];
            let mut total = 0u64;
            for chunk in 0..10 {
                let pos = chunk * 1024;
                let n = reader_clone.read(pos, &mut output, 1024);
                total += n as u64;
            }
            total
        });

        // Send hints from a second thread.
        for i in 0..10 {
            reader.hint(ReadHint {
                frame: i * 1024,
                priority: HintPriority::LookAhead,
            });
            thread::sleep(Duration::from_millis(5));
        }

        let total = read_thread.join().unwrap();
        assert!(total > 0, "concurrent reader should read some frames");
    }

    #[test]
    fn stress_random_seeks() {
        let decoder = Box::new(MockDecoder::new(44100 * 30));
        let reader = CachingReader::with_config(decoder, 1024, 64);
        wait_for_worker();

        let mut output = vec![0.0f32; 1024 * 2];

        let positions: Vec<u64> = vec![
            0,
            44100,
            22050,
            88200,
            44100 * 5,
            1024,
            44100 * 10,
            44100 * 2,
            44100 * 15,
            0,
            44100 * 20,
        ];

        for &pos in &positions {
            reader.hint(ReadHint {
                frame: pos,
                priority: HintPriority::CurrentPosition,
            });
            let _ = reader.read(pos, &mut output, 1024);
        }

        // After settling, the last hinted position should be cached.
        let last_pos = *positions.last().unwrap();
        reader.hint(ReadHint {
            frame: last_pos,
            priority: HintPriority::CurrentPosition,
        });
        wait_for_worker();

        let frames = reader.read(last_pos, &mut output, 1024);
        assert!(
            frames > 0,
            "after settling, should read from last position"
        );
    }

    #[test]
    fn total_frames_matches_decoder() {
        let decoder = Box::new(MockDecoder::new(44100));
        let reader = CachingReader::new(decoder);
        assert_eq!(reader.total_frames(), 44100);
    }

    #[test]
    fn read_across_chunk_boundary() {
        let chunk_size = 1024;
        let decoder = Box::new(MockDecoder::new(44100));
        let reader = CachingReader::with_config(decoder, chunk_size, 16);
        wait_for_worker();

        // Read starting from the middle of chunk 0, spanning into chunk 1.
        let start = 512u64;
        let frames_to_read = 1024;
        let mut output = vec![0.0f32; frames_to_read * 2];
        let frames = reader.read(start, &mut output, frames_to_read);
        assert_eq!(frames, frames_to_read, "should read across chunk boundary");

        for i in 0..frames {
            let expected = MockDecoder::sample_for_frame(start + i as u64);
            assert!(
                (output[i * 2] - expected).abs() < f32::EPSILON,
                "frame {i}: expected {expected}, got {}",
                output[i * 2]
            );
        }
    }

    #[test]
    fn end_of_track_partial_chunk() {
        // Track is 1500 frames, chunk size 1024 — last chunk has 476 frames.
        let decoder = Box::new(MockDecoder::new(1500));
        let reader = CachingReader::with_config(decoder, 1024, 8);
        wait_for_worker();

        // Read the second (partial) chunk.
        let mut output = vec![0.0f32; 1024 * 2];
        let frames = reader.read(1024, &mut output, 1024);
        assert_eq!(frames, 476, "should read partial last chunk");

        for i in 0..frames {
            let expected = MockDecoder::sample_for_frame(1024 + i as u64);
            assert!(
                (output[i * 2] - expected).abs() < f32::EPSILON,
                "frame {i}: expected {expected}, got {}",
                output[i * 2]
            );
        }
    }

    #[test]
    fn drop_shuts_down_worker() {
        let decoder = Box::new(MockDecoder::new(44100));
        let reader = CachingReader::new(decoder);
        drop(reader);
        // If the worker doesn't shut down, the test will hang.
    }
}
