//! A rodio [`Source`] that plays Ogg Opus files.
//!
//! rodio's bundled symphonia decoders don't cover Opus, so we demux the Ogg
//! container ourselves (the `ogg` crate) and decode each packet with libopus
//! (the `audiopus` bindings). Opus always decodes to 48 kHz; the channel count
//! and encoder pre-skip come from the stream's `OpusHead` header.

use std::fs::File;
use std::io::BufReader;
use std::time::Duration;

use audiopus::coder::Decoder as OpusDecoder;
use audiopus::packet::Packet;
use audiopus::{Channels, MutSignals, SampleRate};
use ogg::reading::PacketReader;
use rodio::source::SeekError;
use rodio::Source;

/// Opus always decodes at 48 kHz regardless of the encoder's input rate.
const OPUS_SAMPLE_RATE: u32 = 48_000;
/// Largest number of samples per channel a single Opus packet can hold
/// (120 ms at 48 kHz). The decode scratch buffer is sized to this.
const MAX_FRAME_SAMPLES: usize = 5_760;

/// Frames over which a seek crossfades the old position's audio into the new
/// one (~10 ms at 48 kHz). Seeking splices two unrelated points of the waveform;
/// blending across this window avoids the step that is otherwise an audible click.
const SEEK_CROSSFADE_FRAMES: usize = 480;

/// An opened Ogg reader plus the `OpusHead` fields needed to decode and seek:
/// `(reader_at_first_audio_packet, channel_count, pre_skip, stream_serial)`.
type OpenedStream = (PacketReader<BufReader<File>>, u8, usize, u32);

pub struct OpusSource {
    reader: PacketReader<BufReader<File>>,
    decoder: OpusDecoder,
    channels: u16,
    /// Reused decode target, length `MAX_FRAME_SAMPLES * channels`.
    scratch: Vec<i16>,
    /// Decoded interleaved samples not yet handed to the iterator.
    buffer: Vec<i16>,
    /// Read cursor into `buffer`.
    pos: usize,
    /// Samples per channel still to discard from the stream start (encoder delay).
    pre_skip: usize,
    /// The stream's original encoder delay (samples/channel). Unlike `pre_skip`
    /// this is never decremented; seeks use it to map output time to Ogg granule.
    pre_skip_total: usize,
    /// Ogg logical-stream serial, for granule-based seeking.
    serial: u32,
    total_duration: Option<Duration>,
    /// Source path, kept so near-start seeks can reopen the stream from the top.
    path: String,
    /// Interleaved samples handed to the iterator so far. `emitted / channels`
    /// is the playback cursor (samples per channel) on the post-skip timeline.
    emitted: u64,
    /// Set when a seek lands past the end of the stream so the source reports EOF.
    ended: bool,
    /// Crossfade samples produced by the last seek, played out before resuming
    /// normal decode. Interleaved; drained via `pending_pos`.
    pending: Vec<i16>,
    /// Read cursor into `pending`.
    pending_pos: usize,
}

impl OpusSource {
    /// Open `path`, parse its `OpusHead`, and prepare a decoder. Fails if the
    /// file isn't a readable Opus stream.
    pub fn new(path: &str) -> Result<Self, String> {
        let (reader, channel_count, pre_skip, serial) = Self::open(path)?;
        let decoder = OpusDecoder::new(SampleRate::Hz48000, Self::channels_enum(channel_count)?)
            .map_err(|e| format!("failed to create opus decoder: {e}"))?;

        Ok(Self {
            reader,
            decoder,
            channels: channel_count as u16,
            scratch: vec![0; MAX_FRAME_SAMPLES * channel_count as usize],
            buffer: Vec::new(),
            pos: 0,
            pre_skip,
            pre_skip_total: pre_skip,
            serial,
            total_duration: None,
            path: path.to_string(),
            emitted: 0,
            ended: false,
            pending: Vec::new(),
            pending_pos: 0,
        })
    }

    /// Open `path` and consume its two header packets, returning a reader
    /// positioned at the first audio packet plus the channel count, encoder
    /// pre-skip, and Ogg stream serial. Shared by `new` and near-start seeks.
    fn open(path: &str) -> Result<OpenedStream, String> {
        let file = File::open(path).map_err(|e| format!("failed to open file: {e}"))?;
        let mut reader = PacketReader::new(BufReader::new(file));

        // The first packet is the OpusHead identification header:
        // "OpusHead"(8) | version(1) | channels(1) | pre_skip(2 LE) | ...
        let head = reader
            .read_packet()
            .map_err(|e| format!("failed to read ogg stream: {e}"))?
            .ok_or_else(|| "empty ogg stream".to_string())?;
        if head.data.len() < 19 || &head.data[..8] != b"OpusHead" {
            return Err("not an Opus stream".to_string());
        }
        let serial = head.stream_serial();
        let channel_count = head.data[9];
        Self::channels_enum(channel_count)?; // validate early
        let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as usize;

        // The second packet is the OpusTags comment header; we read tags via
        // lofty elsewhere, so just consume it.
        reader
            .read_packet()
            .map_err(|e| format!("failed to read ogg stream: {e}"))?
            .ok_or_else(|| "missing Opus tags header".to_string())?;

        Ok((reader, channel_count, pre_skip, serial))
    }

    /// Map an `OpusHead` channel count to a libopus channel layout. Mono and
    /// stereo are the only counts the decoder is created for.
    fn channels_enum(count: u8) -> Result<Channels, String> {
        match count {
            1 => Ok(Channels::Mono),
            2 => Ok(Channels::Stereo),
            n => Err(format!("unsupported Opus channel count: {n}")),
        }
    }

    /// Attach a known total duration (rodio can't derive it from this source).
    pub fn with_total_duration(mut self, secs: Option<f64>) -> Self {
        self.total_duration = secs.map(Duration::from_secs_f64);
        self
    }

    /// Decode the next audio packet into `buffer`, discarding any remaining
    /// pre-skip samples. Returns `false` at end of stream.
    fn refill(&mut self) -> bool {
        loop {
            let packet = match self.reader.read_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return false,
                Err(e) => {
                    eprintln!("opus: ogg read error: {e}");
                    return false;
                }
            };
            let Ok(input) = Packet::try_from(&packet.data[..]) else {
                continue; // empty/invalid packet
            };
            let Ok(signals) = MutSignals::try_from(&mut self.scratch[..]) else {
                return false;
            };
            let per_channel = match self.decoder.decode(Some(input), signals, false) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("opus: decode error: {e}");
                    continue;
                }
            };

            let ch = self.channels as usize;
            // Drop encoder-delay samples from the very start of the stream.
            let skip = self.pre_skip.min(per_channel);
            self.pre_skip -= skip;
            let (start, end) = (skip * ch, per_channel * ch);
            if start >= end {
                continue; // whole packet was pre-skip
            }
            self.buffer.clear();
            self.buffer.extend_from_slice(&self.scratch[start..end]);
            self.pos = 0;
            return true;
        }
    }

    /// Replace the decoder with a fresh one, discarding its state. Used after a
    /// seek jumps to a discontinuous point in the stream.
    fn reset_decoder(&mut self) -> Result<(), SeekError> {
        let channels = Self::channels_enum(self.channels as u8).map_err(into_seek_err)?;
        self.decoder = OpusDecoder::new(SampleRate::Hz48000, channels)
            .map_err(|e| into_seek_err(format!("failed to create opus decoder: {e}")))?;
        Ok(())
    }

    /// Decode and discard whole frames from the current reader position until
    /// the playback cursor reaches `target_pcm` (samples per channel), applying
    /// pre-skip via `refill`. Used for near-start seeks that decode from offset 0.
    fn decode_discard_to(&mut self, target_pcm: u64) {
        let ch = self.channels as usize;
        while self.emitted / (ch as u64) < target_pcm {
            if self.pos >= self.buffer.len() && !self.refill() {
                break; // EOF
            }
            let avail = (self.buffer.len() - self.pos) / ch;
            let take = avail.min((target_pcm - self.emitted / (ch as u64)) as usize);
            self.pos += take * ch;
            self.emitted += (take * ch) as u64;
        }
    }

    /// After a coarse `seek_absgp`, decode forward a page at a time until the
    /// page whose end granule reaches `goal` (raw 48 kHz position incl. pre-skip),
    /// then position `buffer`/`pos` exactly at `goal`. Intervening pages are
    /// discarded but warm up the stateful decoder so output at `goal` is correct.
    fn seek_decode_to(&mut self, goal: u64) -> Result<(), SeekError> {
        let ch = self.channels as usize;
        loop {
            // Decode one page worth of packets into `buffer`, ending on the
            // page's absolute granule (the position of its last sample).
            self.buffer.clear();
            let end_granule = loop {
                let packet = match self.reader.read_packet() {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        // Hit EOF before the goal; leave the source empty at the end.
                        self.pos = self.buffer.len();
                        return Ok(());
                    }
                    Err(e) => return Err(into_seek_err(format!("ogg read error: {e}"))),
                };
                let last_in_page = packet.last_in_page();
                let granule = packet.absgp_page();
                if let Ok(input) = Packet::try_from(&packet.data[..]) {
                    if let Ok(signals) = MutSignals::try_from(&mut self.scratch[..]) {
                        match self.decoder.decode(Some(input), signals, false) {
                            Ok(n) => self.buffer.extend_from_slice(&self.scratch[..n * ch]),
                            Err(e) => eprintln!("opus: decode error during seek: {e}"),
                        }
                    }
                }
                // `u64::MAX` marks a page that completes no packet — can't anchor on it.
                if last_in_page && granule != u64::MAX {
                    break granule;
                }
            };
            let page_frames = (self.buffer.len() / ch) as u64;
            let start_granule = end_granule.saturating_sub(page_frames);
            if end_granule >= goal {
                // The goal falls inside this page; skip into it to land exactly.
                let into_page = goal.saturating_sub(start_granule) as usize;
                self.pos = (into_page * ch).min(self.buffer.len());
                return Ok(());
            }
            // Goal is further on: drop this page and keep decoding (decoder warms up).
        }
    }

    /// Position the reader/buffer so the next decoded sample is at `target_pcm`
    /// (output samples/channel); `goal` is the equivalent raw 48 kHz granule
    /// (`target_pcm + pre_skip`). Uses granule bisection when possible, else
    /// rebuilds from the start. Leaves the decoder ready to continue.
    fn seek_to(&mut self, target_pcm: u64, goal: u64) -> Result<(), SeekError> {
        // How far before the target the coarse seek lands; the decoded gap both
        // warms up the decoder (Opus needs ~80 ms of preroll) and absorbs page
        // granularity. ~0.5 s of audio decodes in a couple of milliseconds.
        const WARMUP: u64 = OPUS_SAMPLE_RATE as u64 / 2;

        // Near the start, rebuild and decode from offset 0: cheap, and it lets
        // `refill` apply pre-skip exactly. Also the fallback if bisection fails.
        if goal <= WARMUP {
            return self.rebuild_from_start(target_pcm);
        }
        let landed = self
            .reader
            .seek_absgp(Some(self.serial), goal - WARMUP)
            .map_err(|e| into_seek_err(format!("ogg seek failed: {e}")))?;
        if !landed {
            return self.rebuild_from_start(target_pcm);
        }

        self.reset_decoder()?;
        self.buffer.clear();
        self.pos = 0;
        self.pre_skip = 0; // mid-stream: granule already accounts for the delay
        self.seek_decode_to(goal)
    }

    /// Reopen from the top and decode-discard to `target_pcm`. Used for seeks
    /// near the start (cheap) and as the fallback when granule bisection fails.
    fn rebuild_from_start(&mut self, target_pcm: u64) -> Result<(), SeekError> {
        let (reader, _count, pre_skip, _serial) = Self::open(&self.path).map_err(into_seek_err)?;
        self.reset_decoder()?;
        self.reader = reader;
        self.pre_skip = pre_skip;
        self.buffer.clear();
        self.pos = 0;
        self.emitted = 0;
        self.decode_discard_to(target_pcm);
        Ok(())
    }

    /// Pull up to `frames` whole frames from the current position into a new
    /// buffer, advancing the read cursor. Does not touch `emitted` — the caller
    /// owns position accounting. Returns fewer frames at end of stream.
    fn take_frames(&mut self, frames: usize) -> Vec<i16> {
        let ch = self.channels as usize;
        let target = frames * ch;
        let mut out = Vec::with_capacity(target);
        while out.len() < target {
            if self.pos >= self.buffer.len() && !self.refill() {
                break;
            }
            let take = (target - out.len()).min(self.buffer.len() - self.pos);
            out.extend_from_slice(&self.buffer[self.pos..self.pos + take]);
            self.pos += take;
        }
        out
    }
}

/// Blend `outgoing` (the old position's continuation) into `incoming` (the new
/// position's head) over the overlap, ramping the mix from all-old to all-new.
/// The result starts equal to `outgoing`, so it joins seamlessly onto whatever
/// was already playing; it ends as pure `incoming`.
fn crossfade(outgoing: &[i16], mut incoming: Vec<i16>, ch: usize) -> Vec<i16> {
    let overlap = outgoing.len().min(incoming.len());
    for (i, sample) in incoming.iter_mut().take(overlap).enumerate() {
        let t = ((i / ch) as f32 / SEEK_CROSSFADE_FRAMES as f32).min(1.0);
        *sample = (outgoing[i] as f32 * (1.0 - t) + *sample as f32 * t) as i16;
    }
    incoming
}

impl Iterator for OpusSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.ended {
            return None;
        }
        // Play out the post-seek crossfade before resuming normal decode.
        if self.pending_pos < self.pending.len() {
            let sample = self.pending[self.pending_pos];
            self.pending_pos += 1;
            self.emitted += 1;
            return Some(sample);
        }
        if self.pos >= self.buffer.len() && !self.refill() {
            return None;
        }
        let sample = self.buffer[self.pos];
        self.pos += 1;
        self.emitted += 1;
        Some(sample)
    }
}

impl Source for OpusSource {
    fn current_frame_len(&self) -> Option<usize> {
        None // channel count and sample rate never change mid-stream
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        OPUS_SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    /// Seek to `pos`. Granule bisection (`seek_absgp`) jumps close to the target
    /// cheaply, then a short decode lands sample-accurately and warms the stateful
    /// decoder — so the work stays bounded and never stalls the audio callback
    /// rodio runs the seek on. The old position's continuation is crossfaded into
    /// the new one so the splice has no audible click.
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.ended = false;
        self.pending.clear();
        self.pending_pos = 0;
        let ch = self.channels as usize;
        let target_pcm = (pos.as_secs_f64() * OPUS_SAMPLE_RATE as f64) as u64;

        // Past the end: end the source instead of decoding the whole file.
        if let Some(total) = self.total_duration {
            let total_pcm = (total.as_secs_f64() * OPUS_SAMPLE_RATE as f64) as u64;
            if target_pcm >= total_pcm {
                self.buffer.clear();
                self.pos = 0;
                self.emitted = target_pcm * self.channels as u64;
                self.ended = true;
                return Ok(());
            }
        }

        // Capture the old position's continuation (seamless with audio already
        // queued downstream) before jumping, for the crossfade.
        let outgoing = self.take_frames(SEEK_CROSSFADE_FRAMES);

        // Ogg granule includes the encoder delay: target maps to raw `+ pre_skip`.
        let goal = target_pcm + self.pre_skip_total as u64;
        self.seek_to(target_pcm, goal)?;

        // Blend the new position's head onto the old tail; play it out first.
        let incoming = self.take_frames(SEEK_CROSSFADE_FRAMES);
        self.pending = crossfade(&outgoing, incoming, ch);
        self.pending_pos = 0;
        self.emitted = target_pcm * self.channels as u64;
        Ok(())
    }
}

/// Wrap a decoder/IO error string as a rodio [`SeekError`].
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
    fn decodes_opus_to_48khz_samples() {
        let mut source = OpusSource::new(&fixture("tagged.opus")).unwrap();
        assert_eq!(source.sample_rate(), 48_000);
        let channels = source.channels() as usize;
        assert!(channels >= 1);

        // The 1-second fixture should yield roughly 48k frames; just assert we
        // get a substantial, non-empty run of decoded samples.
        let count = source.by_ref().take(200_000).count();
        assert!(
            count > channels * 40_000,
            "decoded too few samples: {count}"
        );
    }

    #[test]
    fn rejects_non_opus_file() {
        match OpusSource::new(&fixture("tagged.mp3")) {
            Ok(_) => panic!("expected an mp3 to be rejected as non-Opus"),
            Err(e) => assert!(!e.is_empty()),
        }
    }

    /// Total per-channel frames decodable from the start of the 1s fixture.
    fn total_frames(source: &mut OpusSource) -> u64 {
        let ch = source.channels() as u64;
        source.by_ref().count() as u64 / ch
    }

    #[test]
    fn seek_forward_advances_cursor() {
        let mut source = OpusSource::new(&fixture("tagged.opus")).unwrap();
        source.try_seek(Duration::from_millis(500)).unwrap();

        // Cursor should sit ~0.5s in (24000 frames at 48 kHz), within a frame or two.
        let cursor = source.emitted / source.channels() as u64;
        assert!((23_900..=24_100).contains(&cursor), "cursor at {cursor}");
        // Audio remains after the seek point.
        assert!(source.next().is_some());
    }

    #[test]
    fn seek_past_end_ends_source() {
        let mut source = OpusSource::new(&fixture("tagged.opus")).unwrap();
        source.try_seek(Duration::from_secs(5)).unwrap();
        assert!(source.next().is_none());
    }

    #[test]
    fn decodes_and_seeks_mono_opus() {
        let mut source = OpusSource::new(&fixture("mono.opus")).unwrap();
        assert_eq!(source.channels(), 1);
        source.try_seek(Duration::from_millis(500)).unwrap();
        let cursor = source.emitted / source.channels() as u64;
        assert!((23_900..=24_100).contains(&cursor), "cursor at {cursor}");
        assert!(source.next().is_some());
    }

    /// Drain the source and return decoded frames (samples per channel).
    fn drain_frames(source: &mut OpusSource) -> u64 {
        let ch = source.channels() as u64;
        source.by_ref().count() as u64 / ch
    }

    #[test]
    #[ignore = "needs an audio device; run with --ignored --nocapture"]
    fn seeks_through_a_real_sink() {
        use rodio::{OutputStream, Sink};
        let (_stream, handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&handle).unwrap();
        let source = OpusSource::new(&fixture("long.opus"))
            .unwrap()
            .with_total_duration(Some(120.0));
        sink.append(source);
        std::thread::sleep(Duration::from_millis(200));
        let before = sink.get_pos();
        let t = std::time::Instant::now();
        sink.try_seek(Duration::from_secs(90)).unwrap();
        eprintln!("sink seek call returned in {:?}", t.elapsed());
        std::thread::sleep(Duration::from_millis(200));
        eprintln!(
            "pos before {before:?}, after seek+200ms {:?}",
            sink.get_pos()
        );
    }

    #[test]
    fn granule_seek_lands_accurately() {
        let mut source = OpusSource::new(&fixture("long.opus"))
            .unwrap()
            .with_total_duration(Some(120.0));
        let ch = source.channels() as u64;

        // Seek far forward into the 120s stream.
        source.try_seek(Duration::from_secs(90)).unwrap();
        assert_eq!(source.emitted / ch, 90 * 48_000, "cursor should report 90s");

        // Draining yields ~the remaining 30s, within a small page/rounding margin.
        let remaining = drain_frames(&mut source);
        let expected = 30 * 48_000;
        assert!(
            remaining.abs_diff(expected) < 5_000,
            "remaining {remaining}, expected ~{expected}",
        );
    }

    #[test]
    fn seek_crossfade_is_continuous() {
        // Tone fixture: a click at the splice shows up as a large jump between
        // consecutive output samples (a 440 Hz tone steps by ~2k at most; a raw
        // splice would jump tens of thousands).
        let mut source = OpusSource::new(&fixture("tone.opus"))
            .unwrap()
            .with_total_duration(Some(5.0));
        let ch = source.channels() as usize;

        // Play ~1s so there is real audio to crossfade out of, then seek away.
        let pre: Vec<i16> = source.by_ref().take(ch * 48_000).collect();
        let last_before = *pre.last().unwrap();
        source.try_seek(Duration::from_secs(3)).unwrap();

        // The crossfade joins onto the audio that was already playing, so the
        // waveform stays smooth across the splice and through the fade window.
        let mut stream = vec![last_before];
        stream.extend(source.by_ref().take(ch * SEEK_CROSSFADE_FRAMES * 2));
        let max_step = stream
            .windows(2)
            .map(|w| (w[0] as i32 - w[1] as i32).abs())
            .max()
            .unwrap();
        assert!(
            max_step < 10_000,
            "discontinuity of {max_step} suggests a click"
        );
    }

    #[test]
    fn granule_seek_is_cheap() {
        // A far seek must not decode the whole gap (that was ~400ms and stalled
        // the audio thread). Granule bisection keeps it well under that.
        let mut source = OpusSource::new(&fixture("long.opus"))
            .unwrap()
            .with_total_duration(Some(120.0));
        let start = std::time::Instant::now();
        source.try_seek(Duration::from_secs(115)).unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(150),
            "seek took {elapsed:?}"
        );
    }

    #[test]
    fn seek_backward_resets() {
        let mut source = OpusSource::new(&fixture("tagged.opus")).unwrap();
        // Consume part of the stream, then rewind to the start.
        let _ = source.by_ref().take(50_000).count();
        source.try_seek(Duration::ZERO).unwrap();
        assert_eq!(source.emitted, 0);

        // The full ~1s of audio is decodable again from the top.
        let frames = total_frames(&mut source);
        assert!(frames > 40_000, "only {frames} frames after rewind");
    }
}
