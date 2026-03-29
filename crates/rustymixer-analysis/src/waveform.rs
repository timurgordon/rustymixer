//! Waveform data generation for audio visualization.
//!
//! Analyzes audio files using FFT to produce per-point min/max/RMS values
//! in low/mid/high frequency bands. This data drives the waveform display
//! in the UI.

use realfft::RealFftPlanner;
use rustymixer_core::audio::SampleRate;
use rustymixer_decode::AudioDecoder;

use crate::AnalysisError;

/// Pre-computed waveform visualization data for a track.
#[derive(Debug, Clone)]
pub struct WaveformData {
    /// Sample rate the waveform was computed at.
    pub sample_rate: SampleRate,
    /// Total frames in the original audio.
    pub total_frames: u64,
    /// Resolution: how many audio frames per waveform data point.
    pub frames_per_point: u32,
    /// Waveform data points.
    pub points: Vec<WaveformPoint>,
}

/// A single waveform data point representing a time slice of audio.
#[derive(Debug, Clone, Copy)]
pub struct WaveformPoint {
    /// Low frequency band (< ~200 Hz).
    pub low: BandData,
    /// Mid frequency band (~200 Hz - 2 kHz).
    pub mid: BandData,
    /// High frequency band (> ~2 kHz).
    pub high: BandData,
}

/// Min/max/RMS for a single frequency band in a time slice.
#[derive(Debug, Clone, Copy)]
pub struct BandData {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

impl BandData {
    fn new() -> Self {
        Self {
            min: f32::MAX,
            max: f32::MIN,
            rms: 0.0,
        }
    }

    fn finalize(self, count: u32) -> Self {
        if count == 0 {
            return Self {
                min: 0.0,
                max: 0.0,
                rms: 0.0,
            };
        }
        Self {
            min: if self.min == f32::MAX { 0.0 } else { self.min },
            max: if self.max == f32::MIN { 0.0 } else { self.max },
            rms: (self.rms / count as f32).sqrt(),
        }
    }
}

/// Waveform resolution preset.
#[derive(Debug, Clone, Copy)]
pub enum WaveformResolution {
    /// Overview: ~800-1200 points for the entire track (small waveform overview).
    Overview,
    /// Detail: ~4000-8000 points (scrolling zoomed waveform).
    Detail,
    /// Custom number of target points.
    Custom(u32),
}

/// Analyzes audio to produce waveform visualization data.
pub struct WaveformAnalyzer {
    /// FFT size for frequency decomposition.
    fft_size: usize,
}

impl WaveformAnalyzer {
    /// Create a new analyzer with the given FFT size.
    ///
    /// The FFT size must be a power of two. Typical value: 1024.
    pub fn new(fft_size: usize) -> Result<Self, AnalysisError> {
        if fft_size == 0 || !fft_size.is_power_of_two() {
            return Err(AnalysisError::InvalidParameter(
                "fft_size must be a positive power of two".into(),
            ));
        }
        Ok(Self { fft_size })
    }

    /// Analyze an entire track and produce waveform data at the given resolution.
    ///
    /// This should run in a background thread — NOT real-time.
    pub fn analyze(
        &self,
        decoder: &mut dyn AudioDecoder,
        resolution: WaveformResolution,
    ) -> Result<WaveformData, AnalysisError> {
        let track_info = decoder.track_info();
        let sample_rate = SampleRate::new(track_info.sample_rate).ok_or_else(|| {
            AnalysisError::InvalidParameter(format!(
                "unsupported sample rate: {}",
                track_info.sample_rate
            ))
        })?;

        let total_frames = track_info
            .total_frames
            .ok_or(AnalysisError::UnknownDuration)?;

        if total_frames == 0 {
            return Ok(WaveformData {
                sample_rate,
                total_frames: 0,
                frames_per_point: 1,
                points: Vec::new(),
            });
        }

        let target_points = match resolution {
            WaveformResolution::Overview => 1000,
            WaveformResolution::Detail => 4000,
            WaveformResolution::Custom(n) => n,
        };

        let frames_per_point = (total_frames as u32 / target_points).max(1);

        // Seek to beginning
        decoder
            .seek(0)
            .map_err(|e| AnalysisError::Decode(format!("failed to seek to start: {e}")))?;

        let points = self.process_audio(decoder, total_frames, frames_per_point, sample_rate)?;

        Ok(WaveformData {
            sample_rate,
            total_frames,
            frames_per_point,
            points,
        })
    }

    fn process_audio(
        &self,
        decoder: &mut dyn AudioDecoder,
        total_frames: u64,
        frames_per_point: u32,
        sample_rate: SampleRate,
    ) -> Result<Vec<WaveformPoint>, AnalysisError> {
        let fft_size = self.fft_size;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        // Frequency bin boundaries
        let bin_freq = sample_rate.hz() as f32 / fft_size as f32;
        let num_bins = fft_size / 2 + 1;
        let low_cutoff = (200.0 / bin_freq).ceil() as usize;
        let mid_cutoff = (2000.0 / bin_freq).ceil() as usize;
        let low_end = low_cutoff.min(num_bins);
        let mid_end = mid_cutoff.min(num_bins);

        // Hann window
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let t = i as f32 / (fft_size - 1) as f32;
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * t).cos())
            })
            .collect();

        // Buffers
        let mut read_buf = vec![0.0f32; fft_size * 2]; // stereo interleaved
        let mut mono_buf = vec![0.0f32; fft_size];
        let mut fft_input = fft.make_input_vec();
        let mut fft_output = fft.make_output_vec();

        // Accumulators for the current waveform point
        let mut low_acc = BandData::new();
        let mut mid_acc = BandData::new();
        let mut high_acc = BandData::new();
        let mut acc_frames: u64 = 0;
        let mut acc_chunks: u32 = 0;

        let mut points =
            Vec::with_capacity((total_frames / frames_per_point as u64 + 1) as usize);
        let mut frames_read_total: u64 = 0;

        loop {
            let frames_read = match decoder.read_frames(&mut read_buf, fft_size) {
                Ok(0) => break,
                Ok(n) => n,
                Err(rustymixer_decode::DecodeError::EndOfStream) => break,
                Err(e) => return Err(AnalysisError::Decode(e.to_string())),
            };

            // Convert stereo interleaved to mono (average L+R)
            for i in 0..frames_read {
                mono_buf[i] = (read_buf[i * 2] + read_buf[i * 2 + 1]) * 0.5;
            }

            // Zero-pad if we got fewer frames than fft_size
            for s in mono_buf[frames_read..fft_size].iter_mut() {
                *s = 0.0;
            }

            // Apply window and copy to FFT input
            for i in 0..fft_size {
                fft_input[i] = mono_buf[i] * window[i];
            }

            // Run FFT
            fft.process(&mut fft_input, &mut fft_output)
                .map_err(|e| AnalysisError::Internal(format!("FFT failed: {e}")))?;

            // Compute band magnitudes from FFT output
            let (low_mag, mid_mag, high_mag) =
                compute_band_magnitudes(&fft_output, low_end, mid_end, num_bins);

            // Accumulate into current point
            low_acc.min = low_acc.min.min(low_mag);
            low_acc.max = low_acc.max.max(low_mag);
            low_acc.rms += low_mag * low_mag;

            mid_acc.min = mid_acc.min.min(mid_mag);
            mid_acc.max = mid_acc.max.max(mid_mag);
            mid_acc.rms += mid_mag * mid_mag;

            high_acc.min = high_acc.min.min(high_mag);
            high_acc.max = high_acc.max.max(high_mag);
            high_acc.rms += high_mag * high_mag;

            acc_frames += frames_read as u64;
            acc_chunks += 1;
            frames_read_total += frames_read as u64;

            // Emit a waveform point when we've accumulated enough frames
            while acc_frames >= frames_per_point as u64 {
                points.push(WaveformPoint {
                    low: low_acc.finalize(acc_chunks),
                    mid: mid_acc.finalize(acc_chunks),
                    high: high_acc.finalize(acc_chunks),
                });

                // Reset accumulators
                low_acc = BandData::new();
                mid_acc = BandData::new();
                high_acc = BandData::new();
                acc_frames -= frames_per_point as u64;
                acc_chunks = 0;
            }
        }

        // Emit any remaining accumulated data as a final point
        if acc_chunks > 0 {
            points.push(WaveformPoint {
                low: low_acc.finalize(acc_chunks),
                mid: mid_acc.finalize(acc_chunks),
                high: high_acc.finalize(acc_chunks),
            });
        }

        tracing::debug!(
            total_frames = frames_read_total,
            points = points.len(),
            frames_per_point,
            "waveform analysis complete"
        );

        Ok(points)
    }
}

/// Compute the average magnitude for each frequency band from FFT output.
fn compute_band_magnitudes(
    spectrum: &[rustfft::num_complex::Complex<f32>],
    low_end: usize,
    mid_end: usize,
    num_bins: usize,
) -> (f32, f32, f32) {
    let band_magnitude = |start: usize, end: usize| -> f32 {
        if start >= end {
            return 0.0;
        }
        let sum: f32 = spectrum[start..end.min(spectrum.len())]
            .iter()
            .map(|c| c.norm())
            .sum();
        sum / (end - start) as f32
    };

    let low = band_magnitude(0, low_end);
    let mid = band_magnitude(low_end, mid_end);
    let high = band_magnitude(mid_end, num_bins);

    (low, mid, high)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustymixer_decode::SymphoniaDecoder;

    /// Helper to create a WAV file with the given generator function.
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

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from("/tmp/rustymixer_analysis_tests").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn analyzer_rejects_non_power_of_two_fft() {
        assert!(WaveformAnalyzer::new(0).is_err());
        assert!(WaveformAnalyzer::new(100).is_err());
        assert!(WaveformAnalyzer::new(1023).is_err());
        assert!(WaveformAnalyzer::new(1024).is_ok());
        assert!(WaveformAnalyzer::new(2048).is_ok());
    }

    #[test]
    fn silence_produces_near_zero_values() {
        let dir = test_dir("silence");
        let path = dir.join("silence.wav");
        create_test_wav(&path, 44100, 2, 1.0, |num_frames, _sr, ch| {
            vec![0.0f32; num_frames * ch as usize]
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let analyzer = WaveformAnalyzer::new(1024).unwrap();
        let data = analyzer
            .analyze(&mut decoder, WaveformResolution::Overview)
            .unwrap();

        assert!(!data.points.is_empty());
        for point in &data.points {
            assert!(
                point.low.rms < 1e-6,
                "low RMS should be ~0: {}",
                point.low.rms
            );
            assert!(
                point.mid.rms < 1e-6,
                "mid RMS should be ~0: {}",
                point.mid.rms
            );
            assert!(
                point.high.rms < 1e-6,
                "high RMS should be ~0: {}",
                point.high.rms
            );
        }
    }

    #[test]
    fn sine_100hz_shows_energy_in_low_band() {
        let dir = test_dir("sine_100hz");
        let path = dir.join("sine_100hz.wav");
        let freq = 100.0f32;

        create_test_wav(&path, 44100, 2, 2.0, |num_frames, sr, ch| {
            let mut samples = vec![0.0f32; num_frames * ch as usize];
            for i in 0..num_frames {
                let t = i as f32 / sr as f32;
                let val = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8;
                for c in 0..ch as usize {
                    samples[i * ch as usize + c] = val;
                }
            }
            samples
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let analyzer = WaveformAnalyzer::new(1024).unwrap();
        let data = analyzer
            .analyze(&mut decoder, WaveformResolution::Overview)
            .unwrap();

        assert!(!data.points.is_empty());

        // Average RMS across all points (skip first and last which may have edge effects)
        let inner_points = if data.points.len() > 4 {
            &data.points[2..data.points.len() - 2]
        } else {
            &data.points[..]
        };

        let avg_low_rms: f32 =
            inner_points.iter().map(|p| p.low.rms).sum::<f32>() / inner_points.len() as f32;
        let avg_mid_rms: f32 =
            inner_points.iter().map(|p| p.mid.rms).sum::<f32>() / inner_points.len() as f32;
        let avg_high_rms: f32 =
            inner_points.iter().map(|p| p.high.rms).sum::<f32>() / inner_points.len() as f32;

        // 100 Hz is firmly in the low band — low should dominate
        assert!(
            avg_low_rms > avg_mid_rms,
            "100 Hz: low band ({avg_low_rms}) should exceed mid ({avg_mid_rms})"
        );
        assert!(
            avg_low_rms > avg_high_rms,
            "100 Hz: low band ({avg_low_rms}) should exceed high ({avg_high_rms})"
        );
    }

    #[test]
    fn sine_5000hz_shows_energy_in_high_band() {
        let dir = test_dir("sine_5000hz");
        let path = dir.join("sine_5000hz.wav");
        let freq = 5000.0f32;

        create_test_wav(&path, 44100, 2, 2.0, |num_frames, sr, ch| {
            let mut samples = vec![0.0f32; num_frames * ch as usize];
            for i in 0..num_frames {
                let t = i as f32 / sr as f32;
                let val = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8;
                for c in 0..ch as usize {
                    samples[i * ch as usize + c] = val;
                }
            }
            samples
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let analyzer = WaveformAnalyzer::new(1024).unwrap();
        let data = analyzer
            .analyze(&mut decoder, WaveformResolution::Overview)
            .unwrap();

        assert!(!data.points.is_empty());

        let inner_points = if data.points.len() > 4 {
            &data.points[2..data.points.len() - 2]
        } else {
            &data.points[..]
        };

        let avg_low_rms: f32 =
            inner_points.iter().map(|p| p.low.rms).sum::<f32>() / inner_points.len() as f32;
        let avg_high_rms: f32 =
            inner_points.iter().map(|p| p.high.rms).sum::<f32>() / inner_points.len() as f32;

        // 5 kHz is firmly in the high band
        assert!(
            avg_high_rms > avg_low_rms,
            "5 kHz: high band ({avg_high_rms}) should exceed low ({avg_low_rms})"
        );
    }

    #[test]
    fn point_count_matches_expected() {
        let dir = test_dir("point_count");
        let path = dir.join("count.wav");
        let sample_rate = 44100u32;
        let duration_secs = 5.0f32;

        create_test_wav(
            &path,
            sample_rate,
            2,
            duration_secs,
            |num_frames, sr, ch| {
                let mut samples = vec![0.0f32; num_frames * ch as usize];
                for i in 0..num_frames {
                    let t = i as f32 / sr as f32;
                    let val = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
                    for c in 0..ch as usize {
                        samples[i * ch as usize + c] = val;
                    }
                }
                samples
            },
        );

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let analyzer = WaveformAnalyzer::new(1024).unwrap();
        let data = analyzer
            .analyze(&mut decoder, WaveformResolution::Overview)
            .unwrap();

        let expected_points = data.total_frames / data.frames_per_point as u64;
        let actual = data.points.len() as u64;

        // Allow +/- 2 tolerance for rounding and partial final chunks
        assert!(
            actual.abs_diff(expected_points) <= 2,
            "expected ~{expected_points} points, got {actual} (frames_per_point={})",
            data.frames_per_point
        );
    }

    #[test]
    fn overview_and_detail_resolutions() {
        let dir = test_dir("resolutions");
        let path = dir.join("res.wav");

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
        let analyzer = WaveformAnalyzer::new(1024).unwrap();
        let overview = analyzer
            .analyze(&mut decoder, WaveformResolution::Overview)
            .unwrap();

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let detail = analyzer
            .analyze(&mut decoder, WaveformResolution::Detail)
            .unwrap();

        // Detail should have more points than overview
        assert!(
            detail.points.len() > overview.points.len(),
            "detail ({}) should have more points than overview ({})",
            detail.points.len(),
            overview.points.len()
        );

        // Detail frames_per_point should be smaller
        assert!(detail.frames_per_point < overview.frames_per_point);
    }

    #[test]
    fn waveform_data_has_correct_metadata() {
        let dir = test_dir("metadata");
        let path = dir.join("meta.wav");

        create_test_wav(&path, 48000, 2, 1.0, |num_frames, _sr, ch| {
            vec![0.0f32; num_frames * ch as usize]
        });

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let analyzer = WaveformAnalyzer::new(1024).unwrap();
        let data = analyzer
            .analyze(&mut decoder, WaveformResolution::Overview)
            .unwrap();

        assert_eq!(data.sample_rate.hz(), 48000);
        assert!(data.total_frames > 0);
        assert!(data.frames_per_point > 0);
    }
}
