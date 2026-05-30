use std::fs::File;
use std::path::Path;
use kira::Frame;
use ogg::PacketReader;
use opus_rs::OpusDecoder;

const MAX_FRAME_SIZE: usize = 5760;
const OPUS_SAMPLE_RATE: u32 = 48000;

#[derive(Debug)]
pub struct DecodedAudio {
    pub frames: Vec<Frame>,
    pub sample_rate: u32,
}

pub fn decode_opus(path: &Path) -> Result<DecodedAudio, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open opus file: {e}"))?;
    let mut reader = PacketReader::new(file);

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
        let decoded_per_channel = decoder
            .decode(&packet.data, MAX_FRAME_SIZE, &mut pcm)
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

    if pre_skip > 0 && pre_skip < frames.len() {
        frames.drain(0..pre_skip);
    }

    Ok(DecodedAudio { frames, sample_rate: OPUS_SAMPLE_RATE })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sample_opus_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let decoded = decode_opus(&path).expect("opus fixture should decode");
        assert_eq!(decoded.sample_rate, 48000, "opus always decodes at 48kHz");
        assert!(!decoded.frames.is_empty(), "should produce PCM frames");
        assert!(decoded.frames.len() > 48_000, "fixture longer than 1 second");
    }
}
