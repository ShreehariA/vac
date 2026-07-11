//! MP4 decoder backed by the system `ffmpeg` and `ffprobe` binaries.
//!
//! Same rationale as the encoder: the Rust FFmpeg bindings (`ffmpeg-next`
//! 7.x, `ffmpeg-the-third` 3.x) target FFmpeg 6/7 and don't compile
//! against the FFmpeg 8 series yet.  Subprocess works against any
//! version, has no build-time dependencies, and gives us the same
//! exact-FPS metadata path.
//!
//! Pipeline:
//!
//! ```text
//! ffprobe(path) -> (width, height, fps_numer/fps_denom)
//! ffmpeg -i path -f rawvideo -pix_fmt rgba pipe:  ->  Vec<VideoFrame>
//! ```
//!
//! `delay_ms` is computed per-frame from the rational fps so that
//! accumulated `t_ms` over N frames lands on the exact wall-clock
//! time, not on rounded multiples of `1000/fps`.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::frames::VideoFrame;
use crate::DecompileError;

/// Decode every frame of an MP4 file into [`VideoFrame`]s.
pub fn load_mp4(path: &Path) -> Result<Vec<VideoFrame>, DecompileError> {
    let meta = ffprobe(path)?;
    let raw = run_ffmpeg_rawvideo(path)?;

    let frame_size = (meta.width as usize) * (meta.height as usize) * 4;
    if frame_size == 0 {
        return Err(DecompileError::Other(format!(
            "ffprobe reported zero-area video: {}x{}",
            meta.width, meta.height
        )));
    }
    if raw.len() % frame_size != 0 {
        return Err(DecompileError::Other(format!(
            "ffmpeg returned {} bytes, not a multiple of frame size {}",
            raw.len(),
            frame_size
        )));
    }
    let n_frames = raw.len() / frame_size;
    if n_frames == 0 {
        return Err(DecompileError::Other("no frames decoded from MP4".into()));
    }

    // Compute per-frame `delay_ms` from the rational FPS so that
    // accumulated `t_ms` lands on the exact wall-clock time of each
    // frame, not on integer-rounded multiples of (1000 / fps).
    //
    // For 30fps the sequence becomes 33, 34, 33, 33, 34, 33, … —
    // averaging exactly 33.33ms — instead of a flat 33ms that would
    // drift the apex of a 3-second animation by 15ms.
    let denom = meta.fps_denom.max(1);
    let numer = meta.fps_numer.max(1);
    let stamp =
        |i: u64| -> u32 { ((i.saturating_mul(1000).saturating_mul(denom)) / numer) as u32 };

    let mut frames = Vec::with_capacity(n_frames);
    for i in 0..n_frames {
        let start = i * frame_size;
        let this_t = stamp(i as u64);
        let next_t = stamp(i as u64 + 1);
        let delay = next_t.saturating_sub(this_t).max(1);
        frames.push(VideoFrame {
            width: meta.width,
            height: meta.height,
            pixels: raw[start..start + frame_size].to_vec(),
            delay_ms: delay,
        });
    }
    Ok(frames)
}

/// `true` if both `ffmpeg` and `ffprobe` are on PATH.  Used by tests to
/// skip cleanly in environments without FFmpeg.
pub fn ffmpeg_available() -> bool {
    fn ok(bin: &str) -> bool {
        Command::new(bin)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    ok("ffmpeg") && ok("ffprobe")
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

#[derive(Debug)]
struct Mp4Meta {
    width: u32,
    height: u32,
    fps_numer: u64,
    fps_denom: u64,
}

fn ffprobe(path: &Path) -> Result<Mp4Meta, DecompileError> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate,avg_frame_rate",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            DecompileError::Other(format!(
                "failed to spawn `ffprobe` ({e}). Install FFmpeg (e.g. `brew install ffmpeg` \
                 on macOS, `apt install ffmpeg` on Debian/Ubuntu) and ensure it's on PATH."
            ))
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(DecompileError::Other(format!(
            "ffprobe failed (exit {}): {stderr}",
            out.status
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut r_rate: Option<(u64, u64)> = None;
    let mut avg_rate: Option<(u64, u64)> = None;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "width" => width = v.trim().parse().ok(),
                "height" => height = v.trim().parse().ok(),
                "r_frame_rate" => r_rate = parse_rate(v.trim()),
                "avg_frame_rate" => avg_rate = parse_rate(v.trim()),
                _ => {}
            }
        }
    }
    let width = width.ok_or_else(|| DecompileError::Other("ffprobe: missing width".into()))?;
    let height = height.ok_or_else(|| DecompileError::Other("ffprobe: missing height".into()))?;
    // Prefer r_frame_rate (the encoded base rate); fall back to avg.
    let (fps_numer, fps_denom) = r_rate
        .or(avg_rate)
        .ok_or_else(|| DecompileError::Other("ffprobe: no frame-rate".into()))?;
    Ok(Mp4Meta {
        width,
        height,
        fps_numer,
        fps_denom,
    })
}

fn parse_rate(s: &str) -> Option<(u64, u64)> {
    if let Some((n, d)) = s.split_once('/') {
        let n = n.parse().ok()?;
        let d = d.parse().ok()?;
        if d == 0 {
            return None;
        }
        Some((n, d))
    } else {
        Some((s.parse().ok()?, 1))
    }
}

fn run_ffmpeg_rawvideo(path: &Path) -> Result<Vec<u8>, DecompileError> {
    let out = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            // Forbid implicit colour-space conversion that could happen
            // with -vsync settings; we want exactly one rawvideo frame per
            // input frame.
            "-vsync",
            "passthrough",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            DecompileError::Other(format!(
                "failed to spawn `ffmpeg` ({e}). Install FFmpeg and ensure it's on PATH."
            ))
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(DecompileError::Other(format!(
            "ffmpeg decode failed (exit {}): {stderr}",
            out.status
        )));
    }
    Ok(out.stdout)
}
