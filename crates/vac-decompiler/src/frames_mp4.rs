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
///
/// Frames are read off ffmpeg's pipe one at a time (v1.16): through v1.15
/// the whole decoded clip was buffered and then copied frame by frame,
/// so loading alone needed twice the clip's memory.
pub fn load_mp4(path: &Path) -> Result<Vec<VideoFrame>, DecompileError> {
    let frames: Vec<VideoFrame> = stream_mp4(path, None)?.collect::<Result<_, _>>()?;
    if frames.is_empty() {
        return Err(DecompileError::Other("no frames decoded from MP4".into()));
    }
    Ok(frames)
}

/// What a clip is, before decoding it: size and frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp4Info {
    pub width: u32,
    pub height: u32,
    pub fps_numer: u64,
    pub fps_denom: u64,
    /// The container's frame count, when it records one.
    pub frames: Option<u64>,
}

/// Probe a clip's size and frame rate.
pub fn probe_mp4(path: &Path) -> Result<Mp4Info, DecompileError> {
    let m = ffprobe(path)?;
    Ok(Mp4Info {
        width: m.width,
        height: m.height,
        fps_numer: m.fps_numer,
        fps_denom: m.fps_denom,
        frames: m.nb_frames,
    })
}

/// The frames of an MP4, decoded one at a time as they are read (v1.16) —
/// optionally shrunk by ffmpeg to `width` pixels wide (height following
/// the aspect, kept even) for a cheap first look. Each frame's `delay_ms`
/// comes from the rational frame rate, exactly as [`load_mp4`]'s always
/// has.
pub fn stream_mp4(path: &Path, width: Option<u32>) -> Result<Mp4Stream, DecompileError> {
    let meta = ffprobe(path)?;
    let (w, h) = match width {
        Some(tw) if tw < meta.width => {
            let th = ((meta.height as u64 * tw as u64 / meta.width.max(1) as u64) as u32 / 2 * 2).max(2);
            (tw / 2 * 2, th)
        }
        _ => (meta.width, meta.height),
    };
    if w == 0 || h == 0 {
        return Err(DecompileError::Other(format!(
            "ffprobe reported zero-area video: {}x{}",
            meta.width, meta.height
        )));
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-loglevel", "error", "-i"]).arg(path);
    if (w, h) != (meta.width, meta.height) {
        cmd.args(["-vf", &format!("scale={w}:{h}:flags=area")]);
    }
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-vsync", "passthrough", "-"]);
    let child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            DecompileError::Other(format!(
                "failed to spawn `ffmpeg` ({e}). Install FFmpeg and ensure it's on PATH."
            ))
        })?;
    Ok(Mp4Stream {
        child,
        width: w,
        height: h,
        fps_numer: meta.fps_numer.max(1),
        fps_denom: meta.fps_denom.max(1),
        index: 0,
        done: false,
    })
}

/// See [`stream_mp4`].
pub struct Mp4Stream {
    child: std::process::Child,
    width: u32,
    height: u32,
    fps_numer: u64,
    fps_denom: u64,
    index: u64,
    done: bool,
}

impl Iterator for Mp4Stream {
    type Item = Result<VideoFrame, DecompileError>;

    fn next(&mut self) -> Option<Self::Item> {
        use std::io::Read;
        if self.done {
            return None;
        }
        let size = (self.width as usize) * (self.height as usize) * 4;
        let mut pixels = vec![0u8; size];
        let out = self.child.stdout.as_mut().expect("piped stdout");
        let mut filled = 0usize;
        while filled < size {
            match out.read(&mut pixels[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.done = true;
                    return Some(Err(DecompileError::Io(e)));
                }
            }
        }
        if filled < size {
            self.done = true;
            let status = self.child.wait();
            if filled > 0 {
                return Some(Err(DecompileError::Other(format!(
                    "ffmpeg ended mid-frame: {filled} of {size} bytes"
                ))));
            }
            return match status {
                Ok(s) if s.success() => None,
                Ok(s) => Some(Err(DecompileError::Other(format!("ffmpeg decode failed (exit {s})")))),
                Err(e) => Some(Err(DecompileError::Io(e))),
            };
        }
        // `delay_ms` from the rational frame rate, so accumulated time
        // lands on each frame's exact wall-clock time: 33, 34, 33, 33, 34
        // … at 30 fps, not a flat 33 that drifts.
        let stamp = |i: u64| -> u64 { i.saturating_mul(1000).saturating_mul(self.fps_denom) / self.fps_numer };
        let delay = (stamp(self.index + 1) - stamp(self.index)).max(1) as u32;
        self.index += 1;
        Some(Ok(VideoFrame {
            width: self.width,
            height: self.height,
            pixels,
            delay_ms: delay,
        }))
    }
}

impl Drop for Mp4Stream {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
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
    nb_frames: Option<u64>,
}

fn ffprobe(path: &Path) -> Result<Mp4Meta, DecompileError> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate,avg_frame_rate,nb_frames",
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
    let mut nb_frames: Option<u64> = None;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "width" => width = v.trim().parse().ok(),
                "height" => height = v.trim().parse().ok(),
                "r_frame_rate" => r_rate = parse_rate(v.trim()),
                "avg_frame_rate" => avg_rate = parse_rate(v.trim()),
                "nb_frames" => nb_frames = v.trim().parse().ok(),
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
        nb_frames,
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
