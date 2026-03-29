use std::fs::File;
use std::path::Path;

use symphonia::core::{
    audio::SampleBuffer,
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey},
    probe::Hint,
};

use crate::{AudioDecoder, DecodeError, FramePos, Result, TrackInfo};

/// Concrete decoder backed by the Symphonia library.
///
/// Supports MP3, FLAC, WAV/AIFF, Ogg Vorbis, AAC/M4A, and PCM formats.
/// Output is always interleaved stereo f32.
pub struct SymphoniaDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    track_info: TrackInfo,
    /// Interleaved stereo f32 samples buffered from the last decoded packet.
    buffer: Vec<f32>,
    /// Read cursor into `buffer`.
    buffer_pos: usize,
    /// Current playback position in frames.
    current_frame: u64,
}

impl SymphoniaDecoder {
    /// Open an audio file and prepare it for decoding.
    ///
    /// The file format is probed automatically from the extension and
    /// content. The first audio track in the file is selected.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|e| DecodeError::UnsupportedFormat(e.to_string()))?;

        let mut reader = probed.format;

        // Select the first audio track.
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| DecodeError::Decode("no audio tracks found".into()))?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let sample_rate = codec_params
            .sample_rate
            .ok_or_else(|| DecodeError::Decode("unknown sample rate".into()))?;

        let source_channels = codec_params.channels.map(|c| c.count()).unwrap_or(2);
        let total_frames = codec_params.n_frames;

        let decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .map_err(|e| DecodeError::Decode(e.to_string()))?;

        // Extract metadata tags (title, artist, album).
        let (mut title, mut artist, mut album) = (None, None, None);
        if let Some(metadata) = reader.metadata().current() {
            for tag in metadata.tags() {
                if let Some(std_key) = tag.std_key {
                    match std_key {
                        StandardTagKey::TrackTitle => title = Some(tag.value.to_string()),
                        StandardTagKey::Artist => artist = Some(tag.value.to_string()),
                        StandardTagKey::Album => album = Some(tag.value.to_string()),
                        _ => {}
                    }
                }
            }
        }

        let track_info = TrackInfo {
            sample_rate,
            channels: source_channels as u16,
            total_frames,
            title,
            artist,
            album,
        };

        Ok(Self {
            reader,
            decoder,
            track_id,
            track_info,
            buffer: Vec::new(),
            buffer_pos: 0,
            current_frame: 0,
        })
    }
}

impl AudioDecoder for SymphoniaDecoder {
    fn total_frames(&self) -> Option<u64> {
        self.track_info.total_frames
    }

    fn track_info(&self) -> &TrackInfo {
        &self.track_info
    }

    fn position(&self) -> FramePos {
        self.current_frame
    }

    fn read_frames(&mut self, output: &mut [f32], max_frames: usize) -> Result<usize> {
        let target_samples = (max_frames * 2).min(output.len());
        let mut written = 0;

        while written < target_samples {
            // Drain leftover samples from the internal buffer first.
            let available = self.buffer.len() - self.buffer_pos;
            if available > 0 {
                let to_copy = available.min(target_samples - written);
                output[written..written + to_copy]
                    .copy_from_slice(&self.buffer[self.buffer_pos..self.buffer_pos + to_copy]);
                self.buffer_pos += to_copy;
                written += to_copy;

                if self.buffer_pos >= self.buffer.len() {
                    self.buffer.clear();
                    self.buffer_pos = 0;
                }
                continue;
            }

            // Decode the next packet from the format reader.
            let packet = match self.reader.next_packet() {
                Ok(p) => p,
                Err(e) => {
                    if is_end_of_stream(&e) {
                        break;
                    }
                    return Err(DecodeError::Decode(e.to_string()));
                }
            };

            // Skip packets from other tracks.
            if packet.track_id() != self.track_id {
                continue;
            }

            let audio_buf = match self.decoder.decode(&packet) {
                Ok(buf) => buf,
                Err(SymphoniaError::DecodeError(_)) => {
                    // Skip corrupted packets.
                    continue;
                }
                Err(e) => return Err(DecodeError::Decode(e.to_string())),
            };

            let num_frames = audio_buf.frames();
            if num_frames == 0 {
                continue;
            }

            // Convert to interleaved f32 via SampleBuffer.
            let spec = *audio_buf.spec();
            let src_channels = spec.channels.count();
            let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
            sample_buf.copy_interleaved_ref(audio_buf);
            let interleaved = sample_buf.samples();

            // Convert to stereo and store in the internal buffer.
            self.buffer.clear();
            self.buffer_pos = 0;

            match src_channels {
                1 => {
                    self.buffer.reserve(num_frames * 2);
                    for &s in interleaved.iter().take(num_frames) {
                        self.buffer.push(s);
                        self.buffer.push(s);
                    }
                }
                2 => {
                    self.buffer
                        .extend_from_slice(&interleaved[..num_frames * 2]);
                }
                n => {
                    // Downmix to stereo. Take L (ch0) and R (ch1), and if a
                    // centre channel exists (ch2), mix it into both at −3 dB.
                    self.buffer.reserve(num_frames * 2);
                    for frame in 0..num_frames {
                        let base = frame * n;
                        let left = interleaved[base];
                        let right = interleaved[base + 1];
                        if n >= 3 {
                            let center = interleaved[base + 2] * 0.707;
                            self.buffer.push((left + center).clamp(-1.0, 1.0));
                            self.buffer.push((right + center).clamp(-1.0, 1.0));
                        } else {
                            self.buffer.push(left);
                            self.buffer.push(right);
                        }
                    }
                }
            }
        }

        let frames_read = written / 2;
        self.current_frame += frames_read as u64;
        Ok(frames_read)
    }

    fn seek(&mut self, pos: FramePos) -> Result<FramePos> {
        let seeked = self
            .reader
            .seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp {
                    ts: pos,
                    track_id: self.track_id,
                },
            )
            .map_err(|e| DecodeError::Seek(e.to_string()))?;

        self.decoder.reset();
        self.buffer.clear();
        self.buffer_pos = 0;
        self.current_frame = seeked.actual_ts;

        Ok(seeked.actual_ts)
    }
}

/// Returns `true` if a Symphonia error indicates end-of-stream.
fn is_end_of_stream(err: &SymphoniaError) -> bool {
    match err {
        SymphoniaError::IoError(e) => e.kind() == std::io::ErrorKind::UnexpectedEof,
        SymphoniaError::ResetRequired => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Create a temporary directory for test fixtures.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("rustymixer_decode_tests")
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a minimal PCM16 WAV file with a 440 Hz sine wave.
    fn create_test_wav(path: &Path, sample_rate: u32, channels: u16, duration_secs: f32) {
        let num_frames = (sample_rate as f32 * duration_secs) as usize;
        let bits_per_sample: u16 = 16;
        let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
        let block_align = channels * bits_per_sample / 8;
        let data_size = (num_frames * channels as usize * 2) as u32;
        let file_size = 36 + data_size;

        let mut buf = Vec::with_capacity(44 + data_size as usize);

        // RIFF header
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        // fmt chunk
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());

        // 440 Hz sine wave
        for frame in 0..num_frames {
            let t = frame as f32 / sample_rate as f32;
            let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin();
            let sample_i16 = (sample * i16::MAX as f32) as i16;
            for _ in 0..channels {
                buf.extend_from_slice(&sample_i16.to_le_bytes());
            }
        }

        std::fs::write(path, buf).unwrap();
    }

    #[test]
    fn open_and_read_wav() {
        let dir = test_dir("open_read");
        let path = dir.join("stereo.wav");
        create_test_wav(&path, 44100, 2, 0.5);

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        assert_eq!(decoder.track_info().sample_rate, 44100);
        assert_eq!(decoder.position(), 0);

        let mut buf = vec![0.0f32; 1024];
        let frames = decoder.read_frames(&mut buf, 512).unwrap();
        assert!(frames > 0);
        assert!(frames <= 512);

        // Samples should contain the sine wave (non-silent).
        assert!(buf[..frames * 2].iter().any(|&s| s.abs() > 0.01));
    }

    #[test]
    fn seek_to_middle() {
        let dir = test_dir("seek");
        let path = dir.join("seek.wav");
        create_test_wav(&path, 44100, 2, 1.0);

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let total = decoder.total_frames().expect("WAV should report total frames");
        assert!(total > 0);

        let mid = total / 2;
        let actual = decoder.seek(mid).unwrap();
        // Allow some imprecision from seeking to the nearest packet boundary.
        assert!(actual <= mid + 4096, "seeked too far: {actual} vs target {mid}");

        let mut buf = vec![0.0f32; 1024];
        let frames = decoder.read_frames(&mut buf, 512).unwrap();
        assert!(frames > 0);
        assert!(decoder.position() > 0);
    }

    #[test]
    fn mono_converted_to_stereo() {
        let dir = test_dir("mono");
        let path = dir.join("mono.wav");
        create_test_wav(&path, 44100, 1, 0.1);

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        assert_eq!(decoder.track_info().channels, 1);

        let mut buf = vec![0.0f32; 2048];
        let frames = decoder.read_frames(&mut buf, 1024).unwrap();
        assert!(frames > 0);

        // Each stereo pair should be identical (duplicated mono sample).
        for i in 0..frames {
            let left = buf[i * 2];
            let right = buf[i * 2 + 1];
            assert!(
                (left - right).abs() < f32::EPSILON,
                "frame {i}: left={left} != right={right}"
            );
        }
    }

    #[test]
    fn error_on_missing_file() {
        match SymphoniaDecoder::open(Path::new("/nonexistent/file.wav")) {
            Err(DecodeError::Io(_)) => {}
            Err(e) => panic!("expected Io error, got: {e}"),
            Ok(_) => panic!("expected error for missing file"),
        }
    }

    #[test]
    fn error_on_invalid_format() {
        let dir = test_dir("invalid");
        let path = dir.join("garbage.wav");
        std::fs::write(&path, b"this is not a wav file at all").unwrap();

        match SymphoniaDecoder::open(&path) {
            Err(DecodeError::UnsupportedFormat(_)) => {}
            Err(e) => panic!("expected UnsupportedFormat error, got: {e}"),
            Ok(_) => panic!("expected error for garbage data"),
        }
    }

    #[test]
    fn track_info_populated() {
        let dir = test_dir("info");
        let path = dir.join("info.wav");
        create_test_wav(&path, 48000, 2, 0.5);

        let decoder = SymphoniaDecoder::open(&path).unwrap();
        let info = decoder.track_info();
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.channels, 2);
        assert!(info.total_frames.unwrap() > 0);
        // Plain WAV files have no metadata tags.
        assert!(info.title.is_none());
        assert!(info.artist.is_none());
        assert!(info.album.is_none());
    }

    #[test]
    fn read_entire_file() {
        let dir = test_dir("full_read");
        let path = dir.join("full.wav");
        create_test_wav(&path, 44100, 2, 0.1);

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        let total = decoder.total_frames().unwrap();

        let mut total_read = 0u64;
        let mut buf = vec![0.0f32; 2048];
        loop {
            let frames = decoder.read_frames(&mut buf, 1024).unwrap();
            if frames == 0 {
                break;
            }
            total_read += frames as u64;
        }

        assert!(total_read > 0);
        // Allow a small tolerance for codec padding.
        let diff = (total_read as i64 - total as i64).unsigned_abs();
        assert!(diff < 100, "read {total_read} frames but expected ~{total}");
    }

    #[test]
    fn position_advances() {
        let dir = test_dir("position");
        let path = dir.join("position.wav");
        create_test_wav(&path, 44100, 2, 0.5);

        let mut decoder = SymphoniaDecoder::open(&path).unwrap();
        assert_eq!(decoder.position(), 0);

        let mut buf = vec![0.0f32; 2048];
        let frames = decoder.read_frames(&mut buf, 1024).unwrap();
        assert_eq!(decoder.position(), frames as u64);
    }
}
