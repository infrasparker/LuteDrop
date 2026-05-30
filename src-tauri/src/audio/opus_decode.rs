use std::fs::File;
use std::path::Path;
use kira::Frame;
use ogg::PacketReader;
use opus_rs::OpusDecoder;

/// Max samples per channel in a single Opus packet (120 ms at 48 kHz).
const MAX_FRAME_SIZE: usize = 5760;
/// Opus is always decoded at 48 kHz regardless of the original sample rate.
const OPUS_SAMPLE_RATE: u32 = 48000;

#[derive(Debug)]
pub struct DecodedAudio {
    pub frames: Vec<Frame>,
    pub sample_rate: u32,
}

/// Number of PCM samples per channel that a single Opus packet decodes to at
/// 48 kHz, derived from the packet's TOC byte per RFC 6716.
///
/// This MUST be exact: `opus-rs`'s `decode` treats the caller-supplied
/// `frame_size` as the authoritative output length (it computes
/// `sub_frame_size = frame_size / frame_count` and emits that many samples),
/// rather than as a buffer capacity like libopus. Passing a too-large value
/// makes it pad every packet with silence.
fn opus_packet_samples_per_channel(packet: &[u8]) -> usize {
    let toc = packet[0];
    let config = toc >> 3;

    // Samples per frame at 48 kHz, by mode/config (RFC 6716 §3.1).
    let per_frame = if config < 12 {
        // SILK-only: 10 / 20 / 40 / 60 ms.
        let ms = match config & 0x03 {
            0 => 10,
            1 => 20,
            2 => 40,
            _ => 60,
        };
        48 * ms
    } else if config < 16 {
        // Hybrid: 10 or 20 ms.
        let ms = if config & 0x01 == 0 { 10 } else { 20 };
        48 * ms
    } else {
        // CELT-only: 2.5 / 5 / 10 / 20 ms -> 120 / 240 / 480 / 960 samples.
        let period = (config & 0x03) as u32;
        (48000 / (400u32 >> period)) as usize
    };

    // Frame count from the TOC code (low 2 bits); code 3 stores it in byte 1.
    let frame_count = match toc & 0x03 {
        0 => 1,
        1 | 2 => 2,
        _ => {
            if packet.len() >= 2 {
                (packet[1] & 0x3F) as usize
            } else {
                1
            }
        }
    };

    per_frame * frame_count
}

/// Decodes an Ogg-contained Opus file to interleaved stereo PCM frames.
/// Supports mono and stereo streams (the common cases for music).
pub fn decode_opus(path: &Path) -> Result<DecodedAudio, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open opus file: {e}"))?;
    let mut reader = PacketReader::new(file);

    // Packet 0: OpusHead — read channel count and pre-skip.
    let head = reader
        .read_packet()
        .map_err(|e| format!("Failed to read opus header: {e}"))?
        .ok_or_else(|| "Opus file has no packets".to_string())?;
    if head.data.len() < 12 || &head.data[0..8] != b"OpusHead" {
        return Err("Not a valid Ogg Opus stream (missing OpusHead)".to_string());
    }
    let channels = head.data[9] as usize;
    if channels == 0 || channels > 2 {
        return Err(format!("Unsupported opus channel count: {channels}"));
    }
    let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as usize;

    // Packet 1: OpusTags — skip it.
    let _tags = reader
        .read_packet()
        .map_err(|e| format!("Failed to read opus tags: {e}"))?;

    let mut decoder = OpusDecoder::new(OPUS_SAMPLE_RATE as i32, channels)
        .map_err(|e| format!("Failed to create opus decoder: {e}"))?;

    let mut frames: Vec<Frame> = Vec::new();
    let mut pcm = vec![0.0f32; MAX_FRAME_SIZE * channels];

    while let Some(packet) = reader
        .read_packet()
        .map_err(|e| format!("Failed to read opus packet: {e}"))?
    {
        if packet.data.is_empty() {
            continue;
        }
        let frame_size = opus_packet_samples_per_channel(&packet.data);
        if frame_size == 0 || frame_size > MAX_FRAME_SIZE {
            continue;
        }

        let decoded_per_channel = decoder
            .decode(&packet.data, frame_size, &mut pcm)
            .map_err(|e| format!("Opus decode error: {e}"))?;

        for i in 0..decoded_per_channel {
            let frame = if channels == 1 {
                Frame::from_mono(pcm[i])
            } else {
                Frame::new(pcm[i * 2], pcm[i * 2 + 1])
            };
            frames.push(frame);
        }
    }

    // Discard encoder pre-skip samples from the start of the stream.
    if pre_skip > 0 && pre_skip < frames.len() {
        frames.drain(0..pre_skip);
    }

    Ok(DecodedAudio { frames, sample_rate: OPUS_SAMPLE_RATE })
}

/// Duration of an Ogg-Opus file in seconds, read from the final page's granule
/// position — no decoding required. Opus granules are counted at 48 kHz and
/// include the encoder pre-skip, which is subtracted out.
pub fn opus_duration_secs(path: &Path) -> Option<f64> {
    let data = std::fs::read(path).ok()?;
    let mut pre_skip: i64 = 0;
    let mut last_granule: i64 = 0;
    let mut idx = 0;
    while let Some(rel) = data[idx..].windows(4).position(|w| w == b"OggS") {
        let p = idx + rel;
        if p + 14 > data.len() {
            break;
        }
        let granule = i64::from_le_bytes(data[p + 6..p + 14].try_into().ok()?);
        if granule >= 0 {
            last_granule = granule;
        }
        let nseg = *data.get(p + 26)? as usize;
        let payload = p + 27 + nseg;
        if data.len() >= payload + 12 && &data[payload..payload + 8] == b"OpusHead" {
            pre_skip = u16::from_le_bytes([data[payload + 10], data[payload + 11]]) as i64;
        }
        idx = p + 4;
    }
    let samples = last_granule - pre_skip;
    if samples <= 0 {
        return None;
    }
    Some(samples as f64 / OPUS_SAMPLE_RATE as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns the true sample count (per channel, pre-skip removed) of an
    /// Ogg-Opus file by reading the last page's granule position directly —
    /// independent of our decoder, so it can validate decode correctness.
    fn true_sample_count(path: &Path) -> i64 {
        let data = std::fs::read(path).expect("read fixture");
        let mut pre_skip: i64 = 0;
        let mut last_granule: i64 = 0;
        let mut idx = 0;
        while let Some(rel) = data[idx..].windows(4).position(|w| w == b"OggS") {
            let p = idx + rel;
            let granule = i64::from_le_bytes(data[p + 6..p + 14].try_into().unwrap());
            if granule >= 0 {
                last_granule = granule;
            }
            let nseg = data[p + 26] as usize;
            let payload = p + 27 + nseg;
            if data.len() >= payload + 12 && &data[payload..payload + 8] == b"OpusHead" {
                pre_skip = u16::from_le_bytes([data[payload + 10], data[payload + 11]]) as i64;
            }
            idx = p + 4;
        }
        last_granule - pre_skip
    }

    #[test]
    fn decodes_sample_opus_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let decoded = decode_opus(&path).expect("opus fixture should decode");

        assert_eq!(decoded.sample_rate, 48000, "opus always decodes at 48kHz");
        assert!(!decoded.frames.is_empty(), "should produce PCM frames");

        // Frame count must match the file's true length (within one packet's
        // worth of samples). Catches the silence-padding / frame_size bug where
        // every packet was inflated to MAX_FRAME_SIZE.
        let expected = true_sample_count(&path);
        let got = decoded.frames.len() as i64;
        assert!(
            (got - expected).abs() <= 5760,
            "decoded {got} frames but file is {expected} samples (off by {})",
            got - expected
        );

        // Output must be real audio, not silence padding.
        let peak = decoded
            .frames
            .iter()
            .map(|f| f.left.abs().max(f.right.abs()))
            .fold(0.0f32, f32::max);
        assert!(peak > 0.01, "decoded audio is essentially silent (peak {peak})");
    }
}
