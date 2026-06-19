//! A rodio [`Source`] for AAC audio in MP4 (`.m4a`) and raw ADTS (`.aac`)
//! containers, decoded with symphonia.
//!
//! rodio bundles symphonia decoders for these formats, but its own `Decoder`
//! still can't play them: rodio wraps the input in a `ReadSeekSource` whose
//! `byte_len()` is always `None`, and symphonia's MP4 demuxer then performs a
//! seek during initialization (to locate the `moov` atom) which rodio's decoder
//! treats as `unreachable!` and panics on — the exact "Seek errors should not
//! occur during initialization" crash. We build the `MediaSourceStream`
//! straight from the `File`, which reports a real length, so the demuxer
//! initializes without that seek; it also lets us drive seeking ourselves.

use std::fs::File;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::Source;
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// A decode error in more than this many consecutive packets is treated as
/// fatal (matches rodio's own symphonia decoder).
const MAX_DECODE_RETRIES: usize = 3;

pub struct SymphoniaSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    /// Only packets for this track are decoded; others (e.g. a cover-art track)
    /// are skipped.
    track_id: u32,
    /// Signal spec of the most recently decoded packet. AAC keeps this constant,
    /// but it is refreshed on each packet in case a stream changes it.
    spec: SignalSpec,
    /// Interleaved i16 samples of the most recently decoded packet.
    buffer: SampleBuffer<i16>,
    /// Read cursor into `buffer`, in samples (not frames).
    offset: usize,
    total_duration: Option<Duration>,
}

impl SymphoniaSource {
    /// Open and probe `path`, decode its first packet, and prepare for playback.
    /// Fails if the file isn't a readable AAC/MP4 stream.
    pub fn new(path: &str) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("failed to open file: {e}"))?;
        // Building the stream from the `File` (not rodio's `ReadSeekSource`) is
        // the whole point: `File` reports a real `byte_len`, so the MP4 demuxer
        // initializes without the seek that crashes rodio's decoder.
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
        {
            hint.with_extension(ext);
        }

        // Gapless trims encoder padding using the container's edit list / priming.
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &MetadataOptions::default())
            .map_err(|e| format!("failed to probe audio: {e}"))?;
        let mut format = probed.format;

        // Pick the first track with an actual codec (skips metadata-only tracks).
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "no decodable audio track".to_string())?;
        let track_id = track.id;

        let total_duration = track
            .codec_params
            .time_base
            .zip(track.codec_params.n_frames)
            .map(|(base, frames)| {
                let Time { seconds, frac } = base.calc_time(frames);
                Duration::from_secs_f64(seconds as f64 + frac)
            });

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("failed to create decoder: {e}"))?;

        // Decode the first packet eagerly: it establishes the real signal spec
        // (AAC-in-MP4 often omits the channel layout from the container, so it's
        // only known after decoding) and rejects a malformed file here with a
        // clean error rather than playing silence.
        let (spec, buffer) = decode_first(format.as_mut(), decoder.as_mut(), track_id)?;

        Ok(Self {
            format,
            decoder,
            track_id,
            spec,
            buffer,
            offset: 0,
            total_duration,
        })
    }

    /// Decode the next packet for our track into `buffer`, refreshing `spec` and
    /// resetting `offset`. Returns `false` at end of stream (or after too many
    /// consecutive decode errors).
    fn decode_next(&mut self) -> bool {
        let mut decode_errors = 0usize;
        let decoded: AudioBufferRef = loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                // Any read error (including the IoError that marks EOF) ends the
                // source; rodio drops it and emits `playback-ended`.
                Err(_) => return false,
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => break decoded,
                Err(SymphoniaError::DecodeError(e)) => {
                    log::error!("aac: decode error: {e}");
                    decode_errors += 1;
                    if decode_errors > MAX_DECODE_RETRIES {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        };

        decoded.spec().clone_into(&mut self.spec);
        self.buffer = make_buffer(decoded, &self.spec);
        self.offset = 0;
        true
    }
}

/// Decode packets until the first one that yields audio for `track_id`,
/// returning its signal spec and samples. Used to initialize the source (and to
/// surface a clean error when a file has no decodable frames).
fn decode_first(
    format: &mut dyn FormatReader,
    decoder: &mut dyn Decoder,
    track_id: u32,
) -> Result<(SignalSpec, SampleBuffer<i16>), String> {
    let mut decode_errors = 0usize;
    let decoded = loop {
        let packet = format
            .next_packet()
            .map_err(|e| format!("no decodable audio frames: {e}"))?;
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => break decoded,
            Err(SymphoniaError::DecodeError(e)) => {
                decode_errors += 1;
                if decode_errors > MAX_DECODE_RETRIES {
                    return Err(format!("decode failed: {e}"));
                }
            }
            Err(e) => return Err(format!("decode failed: {e}")),
        }
    };
    let spec = *decoded.spec();
    let buffer = make_buffer(decoded, &spec);
    Ok((spec, buffer))
}

/// Copy a decoded packet into an interleaved i16 [`SampleBuffer`].
fn make_buffer(decoded: AudioBufferRef, spec: &SignalSpec) -> SampleBuffer<i16> {
    let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, *spec);
    buffer.copy_interleaved_ref(decoded);
    buffer
}

impl Iterator for SymphoniaSource {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<i16> {
        if self.offset >= self.buffer.len() && !self.decode_next() {
            return None;
        }
        let sample = *self.buffer.samples().get(self.offset)?;
        self.offset += 1;
        Some(sample)
    }
}

impl Source for SymphoniaSource {
    fn current_frame_len(&self) -> Option<usize> {
        // Samples left in the current packet; after this rodio re-reads the spec,
        // covering the (rare for AAC) case of it changing between packets.
        Some(self.buffer.len() - self.offset)
    }

    fn channels(&self) -> u16 {
        self.spec.channels.count() as u16
    }

    fn sample_rate(&self) -> u32 {
        self.spec.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    /// Seek to `pos`. symphonia's demuxer does the heavy lifting: it jumps to the
    /// packet at or before the target (`actual_ts`); we then reset the decoder
    /// and skip the leftover frames so the next sample lands at `required_ts`.
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        // Clamp a past-the-end seek to just shy of the end so the source finishes
        // promptly instead of erroring out of the container.
        let target = match self.total_duration {
            Some(total) if pos >= total => total.saturating_sub(Duration::from_millis(1)),
            _ => pos,
        };

        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: Time::from(target.as_secs_f64()),
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| into_seek_err(format!("seek failed: {e}")))?;

        // Seeking discontinues the stateful AAC decoder; reset it so the frames
        // decoded right after the seek aren't polluted by pre-seek state.
        self.decoder.reset();

        // The container lands on a packet boundary at or before the request;
        // decode forward and drop whole packets until the one containing the
        // target, then position `offset` exactly at it.
        let channels = self.channels().max(1) as u64;
        let mut frames_to_skip = seeked.required_ts.saturating_sub(seeked.actual_ts);
        loop {
            if !self.decode_next() {
                // Hit EOF while refining; leave the source empty so it ends.
                self.offset = self.buffer.len();
                return Ok(());
            }
            let frames = self.buffer.len() as u64 / channels;
            if frames_to_skip < frames {
                self.offset = (frames_to_skip * channels) as usize;
                return Ok(());
            }
            frames_to_skip -= frames;
        }
    }
}

/// Wrap an error string as a rodio [`SeekError`].
fn into_seek_err(msg: String) -> SeekError {
    SeekError::Other(Box::new(std::io::Error::other(msg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn decodes_m4a_to_samples() {
        let mut source = SymphoniaSource::new(&fixture("tagged.m4a")).unwrap();
        assert!(source.sample_rate() >= 8_000);
        let channels = source.channels() as usize;
        assert!(channels >= 1);

        // The ~1s fixture should yield a substantial run of decoded samples.
        let count = source.by_ref().take(200_000).count();
        assert!(
            count > channels * 30_000,
            "decoded too few samples: {count}"
        );
    }

    #[test]
    fn reports_total_duration() {
        let source = SymphoniaSource::new(&fixture("tagged.m4a")).unwrap();
        let secs = source.total_duration().unwrap().as_secs_f64();
        assert!((0.9..=1.2).contains(&secs), "duration was {secs}s");
    }

    #[test]
    fn rejects_non_aac_file() {
        match SymphoniaSource::new(&fixture("tagged.opus")) {
            // The opus fixture is an Ogg stream; the AAC/MP4 path either fails to
            // probe it or finds no AAC track. Either way, construction must fail.
            Ok(_) => panic!("expected an opus file to be rejected"),
            Err(e) => assert!(!e.is_empty()),
        }
    }

    #[test]
    fn seek_forward_leaves_remaining_audio() {
        let mut source = SymphoniaSource::new(&fixture("tagged.m4a")).unwrap();
        let channels = source.channels() as u64;
        let total = source.total_duration().unwrap();

        source.try_seek(Duration::from_millis(500)).unwrap();

        // Roughly half the file should remain after seeking to its midpoint.
        let remaining = source.by_ref().count() as u64 / channels;
        let expected = (total.as_secs_f64() - 0.5) * source.sample_rate() as f64;
        assert!(
            (remaining as f64 - expected).abs() < 8_000.0,
            "remaining {remaining}, expected ~{expected}",
        );
    }

    #[test]
    fn seek_to_start_replays_full_file() {
        let mut source = SymphoniaSource::new(&fixture("tagged.m4a")).unwrap();
        let channels = source.channels() as u64;
        let full = source.by_ref().count() as u64 / channels;

        source.try_seek(Duration::ZERO).unwrap();
        let after = source.by_ref().count() as u64 / channels;

        // Rewinding to the start yields essentially the whole file again.
        assert!(
            after.abs_diff(full) < 4_000,
            "after rewind {after}, originally {full}",
        );
    }

    #[test]
    fn seek_past_end_ends_source() {
        let mut source = SymphoniaSource::new(&fixture("tagged.m4a")).unwrap();
        source.try_seek(Duration::from_secs(60)).unwrap();
        // A handful of trailing frames may remain; the source must end quickly.
        let remaining = source.by_ref().take(20_000).count();
        assert!(
            remaining < 10_000,
            "expected near-EOF, got {remaining} samples"
        );
    }
}
