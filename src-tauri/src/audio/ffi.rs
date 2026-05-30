use std::ffi::CString;
use std::os::raw::{c_char, c_float, c_int};

extern "C" {
    fn ld_decode_file(
        path: *const c_char,
        out: *mut *mut c_float,
        frames: *mut u64,
        channels: *mut u32,
        sample_rate: *mut u32,
    ) -> c_int;
    fn ld_free_decoded(p: *mut c_float);
}

pub struct Decoded {
    pub samples: Vec<f32>,
    pub frames: u64,
    pub channels: u32,
    pub sample_rate: u32,
}

pub fn decode_file(path: &str) -> Result<Decoded, String> {
    let c = CString::new(path).map_err(|e| e.to_string())?;
    let (mut out, mut frames, mut channels, mut sr) =
        (std::ptr::null_mut(), 0u64, 0u32, 0u32);
    let rc = unsafe { ld_decode_file(c.as_ptr(), &mut out, &mut frames, &mut channels, &mut sr) };
    if rc != 0 || out.is_null() {
        return Err(format!("decode failed for {path}"));
    }
    let n = (frames as usize) * (channels as usize);
    let samples = unsafe { std::slice::from_raw_parts(out, n) }.to_vec();
    unsafe { ld_free_decoded(out) };
    Ok(Decoded {
        samples,
        frames,
        channels,
        sample_rate: sr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn decodes_opus_fixture() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let d = decode_file(p.to_str().unwrap()).expect("decode");
        assert_eq!(d.sample_rate, 48000);
        assert_eq!(d.channels, 2);
        // ~120s fixture at 48k; allow one packet of slack.
        assert!((d.frames as i64 - 5_760_000).abs() < 10_000, "frames={}", d.frames);
        let peak = d.samples.iter().fold(0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.01, "silent (peak {peak})");
    }
}
