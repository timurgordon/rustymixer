//! Beat detection and BPM analysis.
//!
//! Uses onset detection (spectral flux) combined with autocorrelation
//! to estimate the BPM and build a beat grid. Designed to run as a
//! background analysis task on track load.

use realfft::RealFftPlanner;
use rustymixer_decode::AudioDecoder;

use crate::AnalysisError;

/// A uniform beat grid describing evenly-spaced beats throughout a track.
#[derive(Debug, Clone)]
pub struct BeatGrid {
    /// Beats per minute.
    pub bpm: f64,
    /// Frame position of the first detected beat.
    pub first_beat_frame: f64,
    /// Number of frames between consecutive beats (`sample_rate * 60.0 / bpm`).
    pub beat_length_frames: f64,
}

impl BeatGrid {
    /// Frame position of the Nth beat (0-indexed).
    pub fn beat_at(&self, n: usize) -> f64 {
        self.first_beat_frame + n as f64 * self.beat_length_frames
    }

    /// Frame position of the beat nearest to `frame`.
    pub fn nearest_beat(&self, frame: f64) -> f64 {
        let idx = self.beat_index(frame).round();
        let idx = if idx < 0.0 { 0.0 } else { idx };
        self.first_beat_frame + idx * self.beat_length_frames
    }

    /// Fractional beat index at the given frame position.
    ///
    /// Returns 0.0 at the first beat, 1.0 at the second, etc.
    pub fn beat_index(&self, frame: f64) -> f64 {
        (frame - self.first_beat_frame) / self.beat_length_frames
    }

    /// Position within the current beat as a value in `[0.0, 1.0)`.
    ///
    /// Returns 0.0 exactly on a beat and 0.5 halfway between beats.
    pub fn beat_distance(&self, frame: f64) -> f64 {
        let idx = self.beat_index(frame);
        let frac = idx - idx.floor();
        if frac < 0.0 {
            frac + 1.0
        } else {
            frac
        }
    }
}

/// Analyzes audio to detect BPM and produce a [`BeatGrid`].
pub struct BeatDetector {
    fft_size: usize,
}

impl BeatDetector {
    /// Create a detector with the given FFT size (must be a power of two).
    ///
    /// Typical values: 1024 or 2048.
    pub fn new(fft_size: usize) -> Result<Self, AnalysisError> {
        if fft_size == 0 || !fft_size.is_power_of_two() {
            return Err(AnalysisError::InvalidParameter(
                "fft_size must be a positive power of two".into(),
            ));
        }
        Ok(Self { fft_size })
    }

    /// Analyze a track and return its beat grid.
    ///
    /// Reads up to the first 60 seconds of audio. This should run on a
    /// background thread — NOT in the real-time audio path.
    pub fn analyze(
        &self,
        decoder: &mut dyn AudioDecoder,
    ) -> Result<BeatGrid, AnalysisError> {
        let track_info = decoder.track_info();
        let sample_rate = track_info.sample_rate;

        if sample_rate == 0 {
            return Err(AnalysisError::InvalidParameter(
                "sample rate is 0".into(),
            ));
        }

        // Seek to beginning
        decoder
            .seek(0)
            .map_err(|e| AnalysisError::Decode(format!("failed to seek to start: {e}")))?;

        // Step 1: Decode up to 60 seconds to mono
        let max_frames = sample_rate as usize * 60;
        let mono = self.decode_to_mono(decoder, max_frames)?;

        if mono.len() < self.fft_size * 4 {
            return Err(AnalysisError::InvalidParameter(
                "track too short for beat detection".into(),
            ));
        }

        // Step 2: Compute spectral flux (onset detection function)
        let hop_size = self.fft_size / 2;
        let onset_signal = self.compute_spectral_flux(&mono, hop_size)?;

        if onset_signal.is_empty() {
            return Err(AnalysisError::Internal(
                "spectral flux produced no frames".into(),
            ));
        }

        // Step 3: Peak-pick onsets
        let onset_peaks = peak_pick(&onset_signal, 7, 1.4);

        // Step 4: Compute autocorrelation of the onset signal
        let autocorr = autocorrelate(&onset_signal);

        // Step 5: Find dominant period → BPM
        let onset_rate = sample_rate as f64 / hop_size as f64; // onset frames per second
        let bpm = find_bpm_from_autocorrelation(&autocorr, onset_rate)?;

        // Step 6: Build beat grid aligned to detected onsets
        let beat_length_frames = sample_rate as f64 * 60.0 / bpm;
        let first_beat = find_first_beat(&onset_peaks, hop_size, beat_length_frames);

        tracing::debug!(
            bpm,
            first_beat,
            beat_length_frames,
            onset_count = onset_peaks.len(),
            "beat detection complete"
        );

        Ok(BeatGrid {
            bpm,
            first_beat_frame: first_beat,
            beat_length_frames,
        })
    }

    /// Decode audio from the decoder into a mono f32 buffer.
    fn decode_to_mono(
        &self,
        decoder: &mut dyn AudioDecoder,
        max_frames: usize,
    ) -> Result<Vec<f32>, AnalysisError> {
        let chunk = 4096;
        let mut stereo_buf = vec![0.0f32; chunk * 2];
        let mut mono = Vec::with_capacity(max_frames.min(1_000_000));
        let mut total = 0usize;

        loop {
            if total >= max_frames {
                break;
            }
            let to_read = chunk.min(max_frames - total);
            let frames_read = match decoder.read_frames(&mut stereo_buf, to_read) {
                Ok(0) => break,
                Ok(n) => n,
                Err(rustymixer_decode::DecodeError::EndOfStream) => break,
                Err(e) => return Err(AnalysisError::Decode(e.to_string())),
            };

            for i in 0..frames_read {
                let l = stereo_buf[i * 2];
                let r = stereo_buf[i * 2 + 1];
                mono.push((l + r) * 0.5);
            }
            total += frames_read;
        }

        Ok(mono)
    }

    /// Compute the spectral flux onset detection function.
    ///
    /// For each FFT frame, computes the magnitude spectrum and sums the
    /// positive differences from the previous frame's spectrum.
    fn compute_spectral_flux(
        &self,
        mono: &[f32],
        hop_size: usize,
    ) -> Result<Vec<f32>, AnalysisError> {
        let fft_size = self.fft_size;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        let num_bins = fft_size / 2 + 1;

        // Hann window
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let t = i as f32 / (fft_size - 1) as f32;
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * t).cos())
            })
            .collect();

        let mut fft_input = fft.make_input_vec();
        let mut fft_output = fft.make_output_vec();
        let mut prev_magnitudes = vec![0.0f32; num_bins];
        let mut onset_signal = Vec::new();

        let num_hops = if mono.len() >= fft_size {
            (mono.len() - fft_size) / hop_size + 1
        } else {
            0
        };

        for hop_idx in 0..num_hops {
            let start = hop_idx * hop_size;

            // Apply window
            for i in 0..fft_size {
                fft_input[i] = mono[start + i] * window[i];
            }

            // Run FFT
            fft.process(&mut fft_input, &mut fft_output)
                .map_err(|e| AnalysisError::Internal(format!("FFT failed: {e}")))?;

            // Compute spectral flux: sum of positive magnitude differences
            let mut flux = 0.0f32;
            for bin in 0..num_bins {
                let mag = fft_output[bin].norm();
                let diff = mag - prev_magnitudes[bin];
                if diff > 0.0 {
                    flux += diff;
                }
                prev_magnitudes[bin] = mag;
            }

            onset_signal.push(flux);
        }

        Ok(onset_signal)
    }
}

/// Peak-pick an onset signal using an adaptive threshold.
///
/// A sample is a peak if it exceeds `threshold_factor` times the local
/// mean over a window of `window_size` on each side.
fn peak_pick(signal: &[f32], window_size: usize, threshold_factor: f32) -> Vec<usize> {
    let len = signal.len();
    if len == 0 {
        return Vec::new();
    }

    let mut peaks = Vec::new();

    for i in 0..len {
        let start = i.saturating_sub(window_size);
        let end = (i + window_size + 1).min(len);
        let count = (end - start) as f32;
        let local_mean: f32 = signal[start..end].iter().sum::<f32>() / count;
        let threshold = local_mean * threshold_factor;

        if signal[i] > threshold && signal[i] > 0.0 {
            // Must be a local maximum
            let is_local_max = (start..end).all(|j| j == i || signal[i] >= signal[j]);
            if is_local_max {
                peaks.push(i);
            }
        }
    }

    peaks
}

/// Compute the (unnormalized) autocorrelation of a signal.
fn autocorrelate(signal: &[f32]) -> Vec<f32> {
    let len = signal.len();
    let max_lag = len; // compute full autocorrelation
    let mut result = vec![0.0f32; max_lag];

    for lag in 0..max_lag {
        let mut sum = 0.0f32;
        for i in 0..(len - lag) {
            sum += signal[i] * signal[i + lag];
        }
        result[lag] = sum;
    }

    result
}

/// Find BPM from the autocorrelation of the onset signal.
///
/// Searches for the dominant peak in the lag range corresponding to
/// 60–200 BPM.
fn find_bpm_from_autocorrelation(
    autocorr: &[f32],
    onset_rate: f64,
) -> Result<f64, AnalysisError> {
    // Lag range: onset_rate * 60 / bpm_max .. onset_rate * 60 / bpm_min
    let min_bpm = 60.0;
    let max_bpm = 200.0;

    let min_lag = (onset_rate * 60.0 / max_bpm).floor() as usize;
    let max_lag = (onset_rate * 60.0 / min_bpm).ceil() as usize;

    let min_lag = min_lag.max(1);
    let max_lag = max_lag.min(autocorr.len() - 1);

    if min_lag >= max_lag {
        return Err(AnalysisError::Internal(
            "autocorrelation range too small for BPM detection".into(),
        ));
    }

    // Find the lag with the highest autocorrelation value
    let mut best_lag = min_lag;
    let mut best_val = autocorr[min_lag];

    for (lag, &val) in autocorr.iter().enumerate().take(max_lag + 1).skip(min_lag + 1) {
        if val > best_val {
            best_val = val;
            best_lag = lag;
        }
    }

    // Parabolic interpolation for sub-sample accuracy
    let lag_refined = if best_lag > min_lag && best_lag < max_lag {
        let y_prev = autocorr[best_lag - 1] as f64;
        let y_curr = autocorr[best_lag] as f64;
        let y_next = autocorr[best_lag + 1] as f64;
        let denom = 2.0 * (2.0 * y_curr - y_prev - y_next);
        if denom.abs() > 1e-10 {
            best_lag as f64 + (y_prev - y_next) / denom
        } else {
            best_lag as f64
        }
    } else {
        best_lag as f64
    };

    let bpm = onset_rate * 60.0 / lag_refined;

    // Normalize to 60-200 range by doubling or halving
    let bpm = normalize_bpm(bpm);

    Ok(bpm)
}

/// Normalize BPM to the 60–200 range by doubling or halving.
fn normalize_bpm(mut bpm: f64) -> f64 {
    while bpm < 60.0 {
        bpm *= 2.0;
    }
    while bpm > 200.0 {
        bpm /= 2.0;
    }
    bpm
}

/// Find the frame position of the first beat by aligning a beat grid
/// to the detected onset peaks.
fn find_first_beat(onset_peaks: &[usize], hop_size: usize, beat_length_frames: f64) -> f64 {
    if onset_peaks.is_empty() {
        return 0.0;
    }

    // Convert onset peak indices to frame positions
    let onset_frames: Vec<f64> = onset_peaks
        .iter()
        .map(|&idx| idx as f64 * hop_size as f64)
        .collect();

    // Try each early onset as a candidate first beat and score alignment
    let candidates = onset_frames.len().min(32);
    let mut best_offset = onset_frames[0];
    let mut best_score = f64::MAX;

    for &candidate in onset_frames.iter().take(candidates) {
        let mut score = 0.0;

        // For each onset, compute distance to the nearest grid beat
        for &onset in &onset_frames {
            let beats_from_start = (onset - candidate) / beat_length_frames;
            let dist = (beats_from_start - beats_from_start.round()).abs();
            score += dist * dist;
        }

        if score < best_score {
            best_score = score;
            best_offset = candidate;
        }
    }

    best_offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustymixer_decode::SymphoniaDecoder;

    /// Create a WAV file with the given generator function (IEEE float).
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
        file.write_all(&3u16.to_le_bytes()).unwrap();
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

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from("/tmp/rustymixer_beat_tests").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Generate a click track: short impulses at the given BPM.
    fn generate_click_track(
        num_frames: usize,
        sample_rate: u32,
        channels: u16,
        bpm: f64,
    ) -> Vec<f32> {
        let frames_per_beat = sample_rate as f64 * 60.0 / bpm;
        let click_length = (sample_rate as f64 * 0.005) as usize; // 5ms click
        let mut samples = vec![0.0f32; num_frames * channels as usize];

        let mut beat_frame = 0.0;
        while (beat_frame as usize) < num_frames {
            let start = beat_frame as usize;
            for i in 0..click_length.min(num_frames.saturating_sub(start)) {
                // Exponentially decaying click
                let amplitude = 0.9 * (-5.0 * i as f32 / click_length as f32).exp();
                for c in 0..channels as usize {
                    samples[(start + i) * channels as usize + c] = amplitude;
                }
            }
            beat_frame += frames_per_beat;
        }

        samples
    }

    /// Generate a synthesized kick drum pattern at the given BPM.
    fn generate_kick_pattern(
        num_frames: usize,
        sample_rate: u32,
        channels: u16,
        bpm: f64,
    ) -> Vec<f32> {
        let frames_per_beat = sample_rate as f64 * 60.0 / bpm;
        let kick_length = (sample_rate as f64 * 0.05) as usize; // 50ms kick
        let mut samples = vec![0.0f32; num_frames * channels as usize];

        let mut beat_frame = 0.0;
        while (beat_frame as usize) < num_frames {
            let start = beat_frame as usize;
            for i in 0..kick_length.min(num_frames.saturating_sub(start)) {
                let t = i as f32 / sample_rate as f32;
                // Frequency sweep from 150 Hz down to 50 Hz
                let freq = 150.0 - 100.0 * (i as f32 / kick_length as f32);
                let env = (-8.0 * i as f32 / kick_length as f32).exp();
                let val = env * (2.0 * std::f32::consts::PI * freq * t).sin();
                for c in 0..channels as usize {
                    samples[(start + i) * channels as usize + c] = val * 0.8;
                }
            }
            beat_frame += frames_per_beat;
        }

        samples
    }

    #[test]
    fn detector_rejects_non_power_of_two_fft() {
        assert!(BeatDetector::new(0).is_err());
        assert!(BeatDetector::new(100).is_err());
        assert!(BeatDetector::new(1024).is_ok());
        assert!(BeatDetector::new(2048).is_ok());
    }

    #[test]
    fn detect_120_bpm_click_track() {
        let dir = test_dir("click_120");
        let path = dir.join("click_120.wav");
        let target_bpm = 120.0;

        create_test_wav(&path, 44100, 2, 15.0, |nf, sr, ch| {
            generate_click_track(nf, sr, ch, target_bpm)
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detector = BeatDetector::new(1024).unwrap();
        let grid = detector.analyze(&mut decoder).unwrap();

        let tolerance = 2.0; // +/- 2 BPM
        assert!(
            (grid.bpm - target_bpm).abs() < tolerance,
            "expected ~{target_bpm} BPM, got {:.1} BPM",
            grid.bpm
        );
    }

    #[test]
    fn detect_140_bpm_kick_pattern() {
        let dir = test_dir("kick_140");
        let path = dir.join("kick_140.wav");
        let target_bpm = 140.0;

        create_test_wav(&path, 44100, 2, 15.0, |nf, sr, ch| {
            generate_kick_pattern(nf, sr, ch, target_bpm)
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detector = BeatDetector::new(1024).unwrap();
        let grid = detector.analyze(&mut decoder).unwrap();

        let tolerance = 3.0;
        assert!(
            (grid.bpm - target_bpm).abs() < tolerance,
            "expected ~{target_bpm} BPM, got {:.1} BPM",
            grid.bpm
        );
    }

    #[test]
    fn beat_grid_beat_at() {
        let grid = BeatGrid {
            bpm: 120.0,
            first_beat_frame: 1000.0,
            beat_length_frames: 22050.0, // 44100 * 60 / 120
        };

        assert_eq!(grid.beat_at(0), 1000.0);
        assert_eq!(grid.beat_at(1), 1000.0 + 22050.0);
        assert_eq!(grid.beat_at(2), 1000.0 + 2.0 * 22050.0);
    }

    #[test]
    fn beat_grid_nearest_beat_snaps_correctly() {
        let grid = BeatGrid {
            bpm: 120.0,
            first_beat_frame: 0.0,
            beat_length_frames: 22050.0,
        };

        // Exactly on beat 0
        assert!((grid.nearest_beat(0.0) - 0.0).abs() < 1e-6);

        // Exactly on beat 1
        assert!((grid.nearest_beat(22050.0) - 22050.0).abs() < 1e-6);

        // Slightly before beat 1 → should snap to beat 1
        assert!((grid.nearest_beat(21000.0) - 22050.0).abs() < 1e-6);

        // Slightly after beat 0 → should snap to beat 0
        assert!((grid.nearest_beat(5000.0) - 0.0).abs() < 1e-6);

        // Exactly halfway → should snap to one of the beats
        let mid = 11025.0;
        let nearest = grid.nearest_beat(mid);
        assert!(
            (nearest - 0.0).abs() < 1e-6 || (nearest - 22050.0).abs() < 1e-6,
            "midpoint should snap to a beat: got {nearest}"
        );
    }

    #[test]
    fn beat_grid_beat_distance() {
        let grid = BeatGrid {
            bpm: 120.0,
            first_beat_frame: 0.0,
            beat_length_frames: 22050.0,
        };

        // On the beat
        assert!((grid.beat_distance(0.0)).abs() < 1e-6);
        assert!((grid.beat_distance(22050.0)).abs() < 1e-6);

        // Half beat
        let half_beat = 11025.0;
        assert!(
            (grid.beat_distance(half_beat) - 0.5).abs() < 1e-6,
            "half beat distance should be 0.5, got {}",
            grid.beat_distance(half_beat)
        );

        // Quarter beat
        let quarter = 22050.0 * 0.25;
        assert!(
            (grid.beat_distance(quarter) - 0.25).abs() < 1e-6,
            "quarter beat distance should be 0.25, got {}",
            grid.beat_distance(quarter)
        );
    }

    #[test]
    fn beat_grid_beat_index() {
        let grid = BeatGrid {
            bpm: 120.0,
            first_beat_frame: 1000.0,
            beat_length_frames: 22050.0,
        };

        assert!((grid.beat_index(1000.0) - 0.0).abs() < 1e-6);
        assert!((grid.beat_index(1000.0 + 22050.0) - 1.0).abs() < 1e-6);
        assert!((grid.beat_index(1000.0 + 44100.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_bpm_stays_in_range() {
        assert!((normalize_bpm(120.0) - 120.0).abs() < 1e-6);
        assert!((normalize_bpm(60.0) - 60.0).abs() < 1e-6);
        assert!((normalize_bpm(200.0) - 200.0).abs() < 1e-6);

        // 240 should halve to 120
        assert!((normalize_bpm(240.0) - 120.0).abs() < 1e-6);

        // 50 should double to 100
        assert!((normalize_bpm(50.0) - 100.0).abs() < 1e-6);

        // 30 should double to 60
        assert!((normalize_bpm(30.0) - 60.0).abs() < 1e-6);
    }
}
