//! Musical key detection using chromagram analysis and key profile matching.
//!
//! Detects the musical key of a track by:
//! 1. Decoding audio to mono and downsampling to ~4410 Hz
//! 2. Computing a chromagram (12 pitch-class energy distribution) via FFT
//! 3. Comparing against Krumhansl-Kessler major and minor key profiles
//! 4. Returning the best-matching key with a confidence score
//!
//! Supports Camelot, Open Key, and standard musical notation.

use realfft::RealFftPlanner;
use rustymixer_core::audio::SampleRate;
use rustymixer_decode::AudioDecoder;

use crate::AnalysisError;

// ---------------------------------------------------------------------------
// Musical key types
// ---------------------------------------------------------------------------

/// One of the 24 musical keys (12 major + 12 minor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MusicalKey {
    CMajor,
    CMinor,
    CSharpMajor,
    CSharpMinor,
    DMajor,
    DMinor,
    EFlatMajor,
    EFlatMinor,
    EMajor,
    EMinor,
    FMajor,
    FMinor,
    FSharpMajor,
    FSharpMinor,
    GMajor,
    GMinor,
    AFlatMajor,
    AFlatMinor,
    AMajor,
    AMinor,
    BFlatMajor,
    BFlatMinor,
    BMajor,
    BMinor,
}

impl MusicalKey {
    /// All 24 keys in chromatic order (C, C#, D, ... B) with major first.
    pub const ALL: [MusicalKey; 24] = [
        MusicalKey::CMajor,
        MusicalKey::CMinor,
        MusicalKey::CSharpMajor,
        MusicalKey::CSharpMinor,
        MusicalKey::DMajor,
        MusicalKey::DMinor,
        MusicalKey::EFlatMajor,
        MusicalKey::EFlatMinor,
        MusicalKey::EMajor,
        MusicalKey::EMinor,
        MusicalKey::FMajor,
        MusicalKey::FMinor,
        MusicalKey::FSharpMajor,
        MusicalKey::FSharpMinor,
        MusicalKey::GMajor,
        MusicalKey::GMinor,
        MusicalKey::AFlatMajor,
        MusicalKey::AFlatMinor,
        MusicalKey::AMajor,
        MusicalKey::AMinor,
        MusicalKey::BFlatMajor,
        MusicalKey::BFlatMinor,
        MusicalKey::BMajor,
        MusicalKey::BMinor,
    ];

    /// Camelot Wheel notation (e.g. "8B", "5A").
    ///
    /// Major keys use "B", minor keys use "A".
    pub fn camelot(&self) -> &'static str {
        match self {
            MusicalKey::CMajor => "8B",
            MusicalKey::CMinor => "5A",
            MusicalKey::CSharpMajor => "3B",
            MusicalKey::CSharpMinor => "12A",
            MusicalKey::DMajor => "10B",
            MusicalKey::DMinor => "7A",
            MusicalKey::EFlatMajor => "5B",
            MusicalKey::EFlatMinor => "2A",
            MusicalKey::EMajor => "12B",
            MusicalKey::EMinor => "9A",
            MusicalKey::FMajor => "7B",
            MusicalKey::FMinor => "4A",
            MusicalKey::FSharpMajor => "2B",
            MusicalKey::FSharpMinor => "11A",
            MusicalKey::GMajor => "9B",
            MusicalKey::GMinor => "6A",
            MusicalKey::AFlatMajor => "4B",
            MusicalKey::AFlatMinor => "1A",
            MusicalKey::AMajor => "11B",
            MusicalKey::AMinor => "8A",
            MusicalKey::BFlatMajor => "6B",
            MusicalKey::BFlatMinor => "3A",
            MusicalKey::BMajor => "1B",
            MusicalKey::BMinor => "10A",
        }
    }

    /// Open Key notation (e.g. "6d", "1m").
    ///
    /// Major keys use "d", minor keys use "m".
    pub fn open_key(&self) -> &'static str {
        match self {
            MusicalKey::CMajor => "1d",
            MusicalKey::CMinor => "10m",
            MusicalKey::CSharpMajor => "8d",
            MusicalKey::CSharpMinor => "5m",
            MusicalKey::DMajor => "3d",
            MusicalKey::DMinor => "12m",
            MusicalKey::EFlatMajor => "10d",
            MusicalKey::EFlatMinor => "7m",
            MusicalKey::EMajor => "5d",
            MusicalKey::EMinor => "2m",
            MusicalKey::FMajor => "12d",
            MusicalKey::FMinor => "9m",
            MusicalKey::FSharpMajor => "7d",
            MusicalKey::FSharpMinor => "4m",
            MusicalKey::GMajor => "2d",
            MusicalKey::GMinor => "11m",
            MusicalKey::AFlatMajor => "9d",
            MusicalKey::AFlatMinor => "6m",
            MusicalKey::AMajor => "4d",
            MusicalKey::AMinor => "1m",
            MusicalKey::BFlatMajor => "11d",
            MusicalKey::BFlatMinor => "8m",
            MusicalKey::BMajor => "6d",
            MusicalKey::BMinor => "3m",
        }
    }

    /// Standard musical notation (e.g. "C", "Am", "F#m", "Eb").
    pub fn standard(&self) -> &'static str {
        match self {
            MusicalKey::CMajor => "C",
            MusicalKey::CMinor => "Cm",
            MusicalKey::CSharpMajor => "C#",
            MusicalKey::CSharpMinor => "C#m",
            MusicalKey::DMajor => "D",
            MusicalKey::DMinor => "Dm",
            MusicalKey::EFlatMajor => "Eb",
            MusicalKey::EFlatMinor => "Ebm",
            MusicalKey::EMajor => "E",
            MusicalKey::EMinor => "Em",
            MusicalKey::FMajor => "F",
            MusicalKey::FMinor => "Fm",
            MusicalKey::FSharpMajor => "F#",
            MusicalKey::FSharpMinor => "F#m",
            MusicalKey::GMajor => "G",
            MusicalKey::GMinor => "Gm",
            MusicalKey::AFlatMajor => "Ab",
            MusicalKey::AFlatMinor => "Abm",
            MusicalKey::AMajor => "A",
            MusicalKey::AMinor => "Am",
            MusicalKey::BFlatMajor => "Bb",
            MusicalKey::BFlatMinor => "Bbm",
            MusicalKey::BMajor => "B",
            MusicalKey::BMinor => "Bm",
        }
    }

    /// Whether this is a major key.
    pub fn is_major(&self) -> bool {
        matches!(
            self,
            MusicalKey::CMajor
                | MusicalKey::CSharpMajor
                | MusicalKey::DMajor
                | MusicalKey::EFlatMajor
                | MusicalKey::EMajor
                | MusicalKey::FMajor
                | MusicalKey::FSharpMajor
                | MusicalKey::GMajor
                | MusicalKey::AFlatMajor
                | MusicalKey::AMajor
                | MusicalKey::BFlatMajor
                | MusicalKey::BMajor
        )
    }

    /// Pitch-class index (0 = C, 1 = C#, ..., 11 = B).
    fn pitch_class(&self) -> usize {
        match self {
            MusicalKey::CMajor | MusicalKey::CMinor => 0,
            MusicalKey::CSharpMajor | MusicalKey::CSharpMinor => 1,
            MusicalKey::DMajor | MusicalKey::DMinor => 2,
            MusicalKey::EFlatMajor | MusicalKey::EFlatMinor => 3,
            MusicalKey::EMajor | MusicalKey::EMinor => 4,
            MusicalKey::FMajor | MusicalKey::FMinor => 5,
            MusicalKey::FSharpMajor | MusicalKey::FSharpMinor => 6,
            MusicalKey::GMajor | MusicalKey::GMinor => 7,
            MusicalKey::AFlatMajor | MusicalKey::AFlatMinor => 8,
            MusicalKey::AMajor | MusicalKey::AMinor => 9,
            MusicalKey::BFlatMajor | MusicalKey::BFlatMinor => 10,
            MusicalKey::BMajor | MusicalKey::BMinor => 11,
        }
    }
}

impl std::fmt::Display for MusicalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.standard())
    }
}

/// Result of key detection analysis.
#[derive(Debug, Clone)]
pub struct KeyResult {
    /// The detected musical key.
    pub key: MusicalKey,
    /// Confidence score from 0.0 (no confidence) to 1.0 (very confident).
    pub confidence: f64,
    /// Raw chromagram: energy per pitch class [C, C#, D, ..., B].
    pub chromagram: [f64; 12],
}

// ---------------------------------------------------------------------------
// Krumhansl-Kessler key profiles
// ---------------------------------------------------------------------------

/// Krumhansl-Kessler major key profile (starting from the tonic).
/// These are empirically derived ratings of pitch-class stability.
const KK_MAJOR: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];

/// Krumhansl-Kessler minor key profile (starting from the tonic).
const KK_MINOR: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// Rotate a 12-element profile by `shift` positions.
fn rotate_profile(profile: &[f64; 12], shift: usize) -> [f64; 12] {
    let mut rotated = [0.0; 12];
    for i in 0..12 {
        rotated[i] = profile[(i + 12 - shift) % 12];
    }
    rotated
}

/// Pearson correlation coefficient between two 12-element vectors.
fn pearson_correlation(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let n = 12.0;
    let mean_a: f64 = a.iter().sum::<f64>() / n;
    let mean_b: f64 = b.iter().sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;

    for i in 0..12 {
        let da = a[i] - mean_a;
        let db = b[i] - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }

    let denom = (var_a * var_b).sqrt();
    if denom < 1e-12 {
        return 0.0;
    }
    cov / denom
}

// ---------------------------------------------------------------------------
// Key detector
// ---------------------------------------------------------------------------

/// Analyzes audio tracks to detect their musical key.
pub struct KeyDetector {
    /// FFT size for chromagram computation. Must be a power of 2.
    fft_size: usize,
    /// Target sample rate for analysis (~4410 Hz). Lower = faster.
    target_sample_rate: u32,
}

impl KeyDetector {
    /// Create a new key detector.
    ///
    /// `fft_size` must be a power of two (typical: 4096 or 8192).
    pub fn new(fft_size: usize) -> Result<Self, AnalysisError> {
        if fft_size == 0 || !fft_size.is_power_of_two() {
            return Err(AnalysisError::InvalidParameter(
                "fft_size must be a positive power of two".into(),
            ));
        }
        Ok(Self {
            fft_size,
            target_sample_rate: 4410,
        })
    }

    /// Detect the musical key of a track.
    ///
    /// Reads the entire track from the decoder, computes a chromagram,
    /// and returns the best-matching key with a confidence score.
    ///
    /// This should run in a background thread, not on the audio thread.
    pub fn analyze(
        &self,
        decoder: &mut dyn AudioDecoder,
    ) -> Result<KeyResult, AnalysisError> {
        let track_info = decoder.track_info();
        let source_rate = track_info.sample_rate;

        SampleRate::new(source_rate).ok_or_else(|| {
            AnalysisError::InvalidParameter(format!("unsupported sample rate: {source_rate}"))
        })?;

        let total_frames = track_info
            .total_frames
            .ok_or(AnalysisError::UnknownDuration)?;

        if total_frames == 0 {
            return Err(AnalysisError::InvalidParameter(
                "track has zero frames".into(),
            ));
        }

        // Seek to beginning
        decoder
            .seek(0)
            .map_err(|e| AnalysisError::Decode(format!("failed to seek to start: {e}")))?;

        // Step 1: Read and downsample to mono at ~target_sample_rate
        let mono_samples = self.read_and_downsample(decoder, source_rate)?;

        if mono_samples.len() < self.fft_size {
            return Err(AnalysisError::InvalidParameter(
                "track too short for key detection".into(),
            ));
        }

        // Step 2: Compute chromagram from the downsampled mono audio
        let chromagram = self.compute_chromagram(&mono_samples)?;

        // Step 3: Match against all 24 key profiles
        let (key, confidence) = self.match_key_profiles(&chromagram);

        tracing::debug!(
            key = %key,
            confidence = confidence,
            "key detection complete"
        );

        Ok(KeyResult {
            key,
            confidence,
            chromagram,
        })
    }

    /// Read the entire track, convert to mono, and downsample.
    fn read_and_downsample(
        &self,
        decoder: &mut dyn AudioDecoder,
        source_rate: u32,
    ) -> Result<Vec<f32>, AnalysisError> {
        let decimation = (source_rate / self.target_sample_rate).max(1) as usize;
        let chunk_frames = 8192;
        let mut read_buf = vec![0.0f32; chunk_frames * 2]; // stereo interleaved
        let mut mono_downsampled = Vec::new();
        let mut frame_index: usize = 0;

        loop {
            let frames_read = match decoder.read_frames(&mut read_buf, chunk_frames) {
                Ok(0) => break,
                Ok(n) => n,
                Err(rustymixer_decode::DecodeError::EndOfStream) => break,
                Err(e) => return Err(AnalysisError::Decode(e.to_string())),
            };

            for i in 0..frames_read {
                if frame_index.is_multiple_of(decimation) {
                    // Average L+R to mono
                    let mono = (read_buf[i * 2] + read_buf[i * 2 + 1]) * 0.5;
                    mono_downsampled.push(mono);
                }
                frame_index += 1;
            }
        }

        Ok(mono_downsampled)
    }

    /// Compute the chromagram: 12-bin pitch-class energy distribution.
    ///
    /// Uses FFT to get the frequency spectrum, then maps each FFT bin
    /// to its nearest pitch class using equal-temperament tuning (A4 = 440 Hz).
    fn compute_chromagram(&self, samples: &[f32]) -> Result<[f64; 12], AnalysisError> {
        let fft_size = self.fft_size;
        let effective_rate = self.target_sample_rate as f32;

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        // Hann window
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let t = i as f32 / (fft_size - 1) as f32;
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * t).cos())
            })
            .collect();

        let mut fft_input = fft.make_input_vec();
        let mut fft_output = fft.make_output_vec();
        let mut chroma = [0.0f64; 12];
        let mut total_energy = 0.0f64;

        // Hop size: 50% overlap
        let hop_size = fft_size / 2;
        let num_windows = if samples.len() >= fft_size {
            (samples.len() - fft_size) / hop_size + 1
        } else {
            0
        };

        let bin_freq = effective_rate / fft_size as f32;
        // Pre-compute pitch-class mapping for each FFT bin
        let num_bins = fft_size / 2 + 1;
        let bin_to_pitch_class = precompute_bin_pitch_classes(num_bins, bin_freq);

        for w in 0..num_windows {
            let offset = w * hop_size;

            // Apply window
            for i in 0..fft_size {
                fft_input[i] = samples[offset + i] * window[i];
            }

            // FFT
            fft.process(&mut fft_input, &mut fft_output)
                .map_err(|e| AnalysisError::Internal(format!("FFT failed: {e}")))?;

            // Accumulate magnitude into pitch-class bins
            for (bin_idx, complex) in fft_output.iter().enumerate() {
                let mag = complex.norm() as f64;
                let mag_sq = mag * mag;
                if let Some(pc) = bin_to_pitch_class[bin_idx] {
                    chroma[pc] += mag_sq;
                    total_energy += mag_sq;
                }
            }
        }

        // Normalize the chromagram
        if total_energy > 1e-12 {
            for c in &mut chroma {
                *c /= total_energy;
            }
        }

        Ok(chroma)
    }

    /// Match the chromagram against all 24 Krumhansl-Kessler key profiles.
    /// Returns the best-matching key and a confidence score.
    fn match_key_profiles(&self, chromagram: &[f64; 12]) -> (MusicalKey, f64) {
        let mut best_key = MusicalKey::CMajor;
        let mut best_corr = f64::NEG_INFINITY;
        let mut correlations = Vec::with_capacity(24);

        for &key in &MusicalKey::ALL {
            let profile = if key.is_major() {
                rotate_profile(&KK_MAJOR, key.pitch_class())
            } else {
                rotate_profile(&KK_MINOR, key.pitch_class())
            };

            let corr = pearson_correlation(chromagram, &profile);
            correlations.push(corr);

            if corr > best_corr {
                best_corr = corr;
                best_key = key;
            }
        }

        // Confidence: how much the best correlation stands out from the rest.
        // Map the best correlation to [0, 1] — Pearson ranges from -1 to 1.
        // Then scale by the gap between best and second-best.
        let mut sorted = correlations.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let second_best = sorted.get(1).copied().unwrap_or(0.0);

        // Base confidence from the correlation value (map -1..1 to 0..1)
        let base = ((best_corr + 1.0) / 2.0).clamp(0.0, 1.0);
        // Separation bonus (how far ahead the best key is)
        let gap = (best_corr - second_best).max(0.0);
        // Combined confidence
        let confidence = (base * 0.7 + gap * 3.0 * 0.3).clamp(0.0, 1.0);

        (best_key, confidence)
    }
}

/// Map each FFT bin to its nearest pitch class (0-11), or `None` if
/// the bin frequency is too low to correspond to a musical note.
fn precompute_bin_pitch_classes(num_bins: usize, bin_freq: f32) -> Vec<Option<usize>> {
    let mut mapping = Vec::with_capacity(num_bins);
    // A4 = 440 Hz. MIDI note 69 = A4. Pitch class = midi_note % 12.
    // freq = 440 * 2^((midi - 69) / 12)
    // midi = 69 + 12 * log2(freq / 440)

    for bin_idx in 0..num_bins {
        let freq = bin_idx as f32 * bin_freq;

        // Skip DC and very low frequencies (below C1 ~32.7 Hz)
        if freq < 30.0 {
            mapping.push(None);
            continue;
        }

        // Convert frequency to pitch class
        let midi = 69.0 + 12.0 * (freq / 440.0).log2();
        let pitch_class = ((midi.round() as i32) % 12 + 12) % 12;
        mapping.push(Some(pitch_class as usize));
    }

    mapping
}

// ---------------------------------------------------------------------------
// Public helper: detect key from a raw chromagram (useful for testing)
// ---------------------------------------------------------------------------

/// Detect the best-matching key from a pre-computed chromagram.
///
/// This is useful for testing the key profile matching independently
/// from the FFT/audio processing pipeline.
pub fn detect_key_from_chromagram(chromagram: &[f64; 12]) -> (MusicalKey, f64) {
    // Reuse the matching logic from KeyDetector
    let detector = KeyDetector {
        fft_size: 4096,
        target_sample_rate: 4410,
    };
    detector.match_key_profiles(chromagram)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustymixer_decode::SymphoniaDecoder;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from("/tmp/rustymixer_key_tests").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Create a WAV file with the given generator function.
    fn create_test_wav(
        path: &std::path::Path,
        sample_rate: u32,
        channels: u16,
        duration_secs: f32,
        generator: impl Fn(usize, u32, u16) -> Vec<f32>,
    ) {
        use std::io::Write;

        let num_frames = (sample_rate as f32 * duration_secs) as usize;
        let samples = generator(num_frames, sample_rate, channels);
        let data_bytes = samples.len() * 4;

        let mut file = std::fs::File::create(path).unwrap();
        let bits_per_sample: u16 = 32;
        let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
        let block_align = channels * (bits_per_sample / 8);

        // RIFF header
        file.write_all(b"RIFF").unwrap();
        file.write_all(&((36 + data_bytes) as u32).to_le_bytes())
            .unwrap();
        file.write_all(b"WAVE").unwrap();

        // fmt chunk (IEEE float)
        file.write_all(b"fmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&3u16.to_le_bytes()).unwrap(); // IEEE float
        file.write_all(&channels.to_le_bytes()).unwrap();
        file.write_all(&sample_rate.to_le_bytes()).unwrap();
        file.write_all(&byte_rate.to_le_bytes()).unwrap();
        file.write_all(&block_align.to_le_bytes()).unwrap();
        file.write_all(&bits_per_sample.to_le_bytes()).unwrap();

        // data chunk
        file.write_all(b"data").unwrap();
        file.write_all(&(data_bytes as u32).to_le_bytes()).unwrap();
        for sample in &samples {
            file.write_all(&sample.to_le_bytes()).unwrap();
        }
    }

    // -----------------------------------------------------------------------
    // Notation tests
    // -----------------------------------------------------------------------

    #[test]
    fn camelot_notation_all_24_keys() {
        // Verify that every key has a unique Camelot code and that
        // major = B, minor = A.
        let mut codes = std::collections::HashSet::new();
        for key in &MusicalKey::ALL {
            let code = key.camelot();
            assert!(
                codes.insert(code),
                "duplicate Camelot code: {code} for {key:?}"
            );
            if key.is_major() {
                assert!(code.ends_with('B'), "{key:?} Camelot should end with B: {code}");
            } else {
                assert!(code.ends_with('A'), "{key:?} Camelot should end with A: {code}");
            }
        }
        assert_eq!(codes.len(), 24);
    }

    #[test]
    fn open_key_notation_all_24_keys() {
        let mut codes = std::collections::HashSet::new();
        for key in &MusicalKey::ALL {
            let code = key.open_key();
            assert!(
                codes.insert(code),
                "duplicate Open Key code: {code} for {key:?}"
            );
            if key.is_major() {
                assert!(code.ends_with('d'), "{key:?} Open Key should end with d: {code}");
            } else {
                assert!(code.ends_with('m'), "{key:?} Open Key should end with m: {code}");
            }
        }
        assert_eq!(codes.len(), 24);
    }

    #[test]
    fn standard_notation_all_24_keys() {
        let mut codes = std::collections::HashSet::new();
        for key in &MusicalKey::ALL {
            let code = key.standard();
            assert!(
                codes.insert(code),
                "duplicate standard notation: {code} for {key:?}"
            );
            // Minor keys end with 'm'
            if !key.is_major() {
                assert!(code.ends_with('m'), "{key:?} standard should end with m: {code}");
            }
        }
        assert_eq!(codes.len(), 24);
    }

    #[test]
    fn specific_notation_values() {
        // Spot-check a few well-known mappings
        assert_eq!(MusicalKey::AMinor.camelot(), "8A");
        assert_eq!(MusicalKey::AMinor.open_key(), "1m");
        assert_eq!(MusicalKey::AMinor.standard(), "Am");

        assert_eq!(MusicalKey::CMajor.camelot(), "8B");
        assert_eq!(MusicalKey::CMajor.open_key(), "1d");
        assert_eq!(MusicalKey::CMajor.standard(), "C");

        assert_eq!(MusicalKey::FSharpMinor.camelot(), "11A");
        assert_eq!(MusicalKey::FSharpMinor.open_key(), "4m");
        assert_eq!(MusicalKey::FSharpMinor.standard(), "F#m");

        assert_eq!(MusicalKey::BFlatMajor.camelot(), "6B");
        assert_eq!(MusicalKey::BFlatMajor.open_key(), "11d");
        assert_eq!(MusicalKey::BFlatMajor.standard(), "Bb");
    }

    #[test]
    fn display_trait() {
        assert_eq!(format!("{}", MusicalKey::CMajor), "C");
        assert_eq!(format!("{}", MusicalKey::AMinor), "Am");
        assert_eq!(format!("{}", MusicalKey::EFlatMajor), "Eb");
    }

    // -----------------------------------------------------------------------
    // Key profile matching tests with synthetic chromagram
    // -----------------------------------------------------------------------

    #[test]
    fn synthetic_c_major_chromagram() {
        // Create a chromagram that strongly matches C major:
        // C major scale notes = C, D, E, F, G, A, B (indices 0, 2, 4, 5, 7, 9, 11)
        let mut chroma = [0.01; 12];
        // Emphasize tonic (C), dominant (G), and third (E)
        chroma[0] = 1.0; // C
        chroma[2] = 0.5; // D
        chroma[4] = 0.8; // E
        chroma[5] = 0.4; // F
        chroma[7] = 0.9; // G
        chroma[9] = 0.5; // A
        chroma[11] = 0.3; // B

        let (key, confidence) = detect_key_from_chromagram(&chroma);
        assert_eq!(key, MusicalKey::CMajor, "expected C major, got {key:?}");
        assert!(
            confidence > 0.3,
            "confidence should be reasonable: {confidence}"
        );
    }

    #[test]
    fn synthetic_a_minor_chromagram() {
        // A minor scale notes = A, B, C, D, E, F, G (indices 9, 11, 0, 2, 4, 5, 7)
        // Emphasize tonic (A), fifth (E), and minor third (C)
        let mut chroma = [0.01; 12];
        chroma[9] = 1.0; // A (tonic)
        chroma[11] = 0.3; // B
        chroma[0] = 0.7; // C (minor third)
        chroma[2] = 0.4; // D
        chroma[4] = 0.8; // E (fifth)
        chroma[5] = 0.5; // F
        chroma[7] = 0.5; // G

        let (key, _confidence) = detect_key_from_chromagram(&chroma);
        assert_eq!(key, MusicalKey::AMinor, "expected A minor, got {key:?}");
    }

    #[test]
    fn synthetic_g_major_chromagram() {
        // G major scale = G, A, B, C, D, E, F# (indices 7, 9, 11, 0, 2, 4, 6)
        let mut chroma = [0.01; 12];
        chroma[7] = 1.0; // G (tonic)
        chroma[9] = 0.5; // A
        chroma[11] = 0.6; // B (third)
        chroma[0] = 0.4; // C
        chroma[2] = 0.8; // D (fifth)
        chroma[4] = 0.5; // E
        chroma[6] = 0.3; // F#

        let (key, _confidence) = detect_key_from_chromagram(&chroma);
        assert_eq!(key, MusicalKey::GMajor, "expected G major, got {key:?}");
    }

    #[test]
    fn synthetic_d_minor_chromagram() {
        // D minor scale = D, E, F, G, A, Bb, C (indices 2, 4, 5, 7, 9, 10, 0)
        let mut chroma = [0.01; 12];
        chroma[2] = 1.0; // D (tonic)
        chroma[4] = 0.3; // E
        chroma[5] = 0.7; // F (minor third)
        chroma[7] = 0.5; // G
        chroma[9] = 0.8; // A (fifth)
        chroma[10] = 0.4; // Bb
        chroma[0] = 0.4; // C

        let (key, _confidence) = detect_key_from_chromagram(&chroma);
        assert_eq!(key, MusicalKey::DMinor, "expected D minor, got {key:?}");
    }

    // -----------------------------------------------------------------------
    // Detector construction tests
    // -----------------------------------------------------------------------

    #[test]
    fn detector_rejects_non_power_of_two() {
        assert!(KeyDetector::new(0).is_err());
        assert!(KeyDetector::new(100).is_err());
        assert!(KeyDetector::new(1023).is_err());
        assert!(KeyDetector::new(4096).is_ok());
        assert!(KeyDetector::new(8192).is_ok());
    }

    // -----------------------------------------------------------------------
    // Pearson correlation tests
    // -----------------------------------------------------------------------

    #[test]
    fn pearson_identical_vectors() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        let corr = pearson_correlation(&a, &a);
        assert!((corr - 1.0).abs() < 1e-10, "self-correlation should be 1.0: {corr}");
    }

    #[test]
    fn pearson_opposite_vectors() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        let b: [f64; 12] = std::array::from_fn(|i| -a[i] + 13.0); // reversed
        let corr = pearson_correlation(&a, &b);
        assert!(corr < -0.9, "negatively correlated vectors should have r < -0.9: {corr}");
    }

    // -----------------------------------------------------------------------
    // Full pipeline test with synthetic audio
    // -----------------------------------------------------------------------

    #[test]
    fn detect_key_from_a_minor_chord() {
        // Generate a 5-second audio file with A minor chord: A4 (440Hz), C5 (523Hz), E5 (659Hz)
        let dir = test_dir("a_minor_chord");
        let path = dir.join("a_minor_chord.wav");

        create_test_wav(&path, 44100, 2, 5.0, |num_frames, sr, ch| {
            let mut samples = vec![0.0f32; num_frames * ch as usize];
            let freqs = [440.0f32, 523.25, 659.25]; // A4, C5, E5
            for i in 0..num_frames {
                let t = i as f32 / sr as f32;
                let mut val = 0.0f32;
                for &f in &freqs {
                    val += (2.0 * std::f32::consts::PI * f * t).sin();
                }
                val /= freqs.len() as f32;
                val *= 0.8;
                for c in 0..ch as usize {
                    samples[i * ch as usize + c] = val;
                }
            }
            samples
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detector = KeyDetector::new(4096).unwrap();
        let result = detector.analyze(&mut decoder).unwrap();

        // With a simple A minor chord, we expect A minor or the relative C major
        let acceptable = [MusicalKey::AMinor, MusicalKey::CMajor];
        assert!(
            acceptable.contains(&result.key),
            "expected A minor or C major for Am chord, got {:?} (confidence: {:.3})",
            result.key,
            result.confidence
        );
        assert!(result.confidence > 0.0, "confidence should be positive");
    }

    #[test]
    fn detect_key_from_c_major_chord() {
        // Generate a 5-second audio file with C major chord: C4 (261Hz), E4 (329Hz), G4 (392Hz)
        let dir = test_dir("c_major_chord");
        let path = dir.join("c_major_chord.wav");

        create_test_wav(&path, 44100, 2, 5.0, |num_frames, sr, ch| {
            let mut samples = vec![0.0f32; num_frames * ch as usize];
            let freqs = [261.63f32, 329.63, 392.0]; // C4, E4, G4
            for i in 0..num_frames {
                let t = i as f32 / sr as f32;
                let mut val = 0.0f32;
                for &f in &freqs {
                    val += (2.0 * std::f32::consts::PI * f * t).sin();
                }
                val /= freqs.len() as f32;
                val *= 0.8;
                for c in 0..ch as usize {
                    samples[i * ch as usize + c] = val;
                }
            }
            samples
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detector = KeyDetector::new(4096).unwrap();
        let result = detector.analyze(&mut decoder).unwrap();

        // C major chord should detect as C major or its relative A minor
        let acceptable = [MusicalKey::CMajor, MusicalKey::AMinor];
        assert!(
            acceptable.contains(&result.key),
            "expected C major or A minor for C chord, got {:?} (confidence: {:.3})",
            result.key,
            result.confidence
        );
    }

    #[test]
    fn detect_key_from_g_major_chord() {
        // G major chord: G3 (196Hz), B3 (247Hz), D4 (293Hz)
        let dir = test_dir("g_major_chord");
        let path = dir.join("g_major_chord.wav");

        create_test_wav(&path, 44100, 2, 5.0, |num_frames, sr, ch| {
            let mut samples = vec![0.0f32; num_frames * ch as usize];
            let freqs = [196.0f32, 246.94, 293.66]; // G3, B3, D4
            for i in 0..num_frames {
                let t = i as f32 / sr as f32;
                let mut val = 0.0f32;
                for &f in &freqs {
                    val += (2.0 * std::f32::consts::PI * f * t).sin();
                }
                val /= freqs.len() as f32;
                val *= 0.8;
                for c in 0..ch as usize {
                    samples[i * ch as usize + c] = val;
                }
            }
            samples
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detector = KeyDetector::new(4096).unwrap();
        let result = detector.analyze(&mut decoder).unwrap();

        // G major chord should detect as G major or its relative E minor
        let acceptable = [MusicalKey::GMajor, MusicalKey::EMinor];
        assert!(
            acceptable.contains(&result.key),
            "expected G major or E minor for G chord, got {:?} (confidence: {:.3})",
            result.key,
            result.confidence
        );
    }

    #[test]
    fn chromagram_has_12_bins() {
        let dir = test_dir("chroma_bins");
        let path = dir.join("chroma.wav");

        create_test_wav(&path, 44100, 2, 3.0, |num_frames, sr, ch| {
            let mut samples = vec![0.0f32; num_frames * ch as usize];
            for i in 0..num_frames {
                let t = i as f32 / sr as f32;
                let val = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
                for c in 0..ch as usize {
                    samples[i * ch as usize + c] = val;
                }
            }
            samples
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detector = KeyDetector::new(4096).unwrap();
        let result = detector.analyze(&mut decoder).unwrap();

        assert_eq!(result.chromagram.len(), 12);
        // Sum should be approximately 1.0 (normalized)
        let sum: f64 = result.chromagram.iter().sum();
        assert!(
            (sum - 1.0).abs() < 0.01,
            "normalized chromagram should sum to ~1.0, got {sum}"
        );
    }

    #[test]
    fn a440_sine_has_energy_in_a_bin() {
        // A 440 Hz sine wave should show dominant energy in pitch class A (index 9)
        let dir = test_dir("a440");
        let path = dir.join("a440.wav");

        create_test_wav(&path, 44100, 2, 5.0, |num_frames, sr, ch| {
            let mut samples = vec![0.0f32; num_frames * ch as usize];
            for i in 0..num_frames {
                let t = i as f32 / sr as f32;
                let val = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.8;
                for c in 0..ch as usize {
                    samples[i * ch as usize + c] = val;
                }
            }
            samples
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detector = KeyDetector::new(4096).unwrap();
        let result = detector.analyze(&mut decoder).unwrap();

        // Pitch class 9 = A should have the highest energy
        let max_idx = result
            .chromagram
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert_eq!(
            max_idx, 9,
            "A440 should have max energy in pitch class A (9), got {max_idx}"
        );
    }
}
