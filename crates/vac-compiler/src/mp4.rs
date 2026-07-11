//! MP4 / H.264 encoder backed by the system `ffmpeg` binary.
//!
//! Deliberately implemented as a subprocess rather than via
//! `ffmpeg-next` bindings: the bindings track FFmpeg 6/7, while
//! recent toolchains (FFmpeg 8+) ship without an up-to-date binding
//! crate.  Subprocess works against any FFmpeg version, has zero
//! build-time dependencies (no pkg-config, no clang, no FFI), and
//! gives us the exact-FPS guarantee that v0.4 promises.
//!
//! Pipeline:
//!
//! ```text
//! RGBA frames in memory   --pipe-->   ffmpeg
//!                                       rawvideo (RGBA, WxH, FPS)
//!                                       -> libx264 (yuv420p, CRF 18)
//!                                       -> MP4 with +faststart
//! ```
//!
//! Settings:
//! - `-crf 18` is "visually lossless" — keeps colour drift well within
//!   v0.3's L1≤12 bound while file sizes stay reasonable.
//! - `-pix_fmt yuv420p` is required for MP4/H.264 compatibility with
//!   downstream tools and players.
//! - `-movflags +faststart` puts the moov atom up-front so the file
//!   plays without buffering when streamed.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::render::RenderedFrame;
use crate::CompileError;

/// Encode a sequence of RGBA frames as an H.264/MP4 file at exactly
/// `fps` frames per second.
pub fn encode_mp4(frames: &[RenderedFrame], fps: u32, out: &Path) -> Result<(), CompileError> {
    if frames.is_empty() {
        return Err(CompileError::Encode("no frames to encode".into()));
    }
    let w = frames[0].width;
    let h = frames[0].height;
    let fps = fps.max(1);

    // libx264 + yuv420p requires even dimensions. Instead of bailing on odd
    // canvases (a real-world snag — e.g. Wikimedia's 658×689 double-pendulum
    // GIF), insert a `pad` filter that rounds W/H up to the next even number
    // and fills the new strip with the colour of the first frame's top-left
    // pixel — which is the background corner sample the decompiler uses, so
    // the padding is visually invisible at canvas edges where shapes don't
    // normally live.
    let needs_pad = (w % 2 != 0) || (h % 2 != 0);
    let pad_filter = if needs_pad {
        let r = frames[0].rgba.first().copied().unwrap_or(0);
        let g = frames[0].rgba.get(1).copied().unwrap_or(0);
        let b = frames[0].rgba.get(2).copied().unwrap_or(0);
        Some(format!(
            "pad=ceil(iw/2)*2:ceil(ih/2)*2:0:0:color=0x{r:02X}{g:02X}{b:02X}"
        ))
    } else {
        None
    };

    let mut args: Vec<String> = [
        "-y",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &format!("{w}x{h}"),
        "-r",
        &fps.to_string(),
        "-i",
        "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(pf) = &pad_filter {
        args.push("-vf".to_string());
        args.push(pf.clone());
    }
    args.extend(
        [
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "18",
            "-preset",
            "medium",
            "-movflags",
            "+faststart",
        ]
        .iter()
        .map(|s| s.to_string()),
    );

    let mut child = Command::new("ffmpeg")
        .args(&args)
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            CompileError::Encode(format!(
                "failed to spawn `ffmpeg` ({e}). Install it (e.g. `brew install ffmpeg` \
                 on macOS, `apt install ffmpeg` on Debian/Ubuntu) and ensure it's on PATH."
            ))
        })?;

    // Pipe every frame's raw RGBA bytes into ffmpeg's stdin. If ffmpeg
    // exits early (bad args etc.) the write fails with EPIPE — keep the
    // io error for later but always wait on the child so we get its
    // stderr to diagnose with.
    let pipe_result: std::io::Result<()> = (|| {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        for f in frames {
            stdin.write_all(&f.rgba)?;
        }
        Ok(())
    })();

    let output = child
        .wait_with_output()
        .map_err(|e| CompileError::Encode(format!("waiting on ffmpeg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CompileError::Encode(format!(
            "ffmpeg failed (exit {}): {stderr}",
            output.status
        )));
    }
    pipe_result.map_err(|e| CompileError::Encode(format!("piping frames to ffmpeg: {e}")))?;
    Ok(())
}

/// `true` if a usable `ffmpeg` binary is available on the current PATH.
/// Used by tests to skip cleanly in environments without ffmpeg.
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
