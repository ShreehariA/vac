//! `pac` — Picture As Code CLI.
//!
//! Mirrors the `vac` CLI's "unzip-like" ergonomics:
//!
//! ```text
//! pac decompile in.png  out.pac
//! pac compile   in.pac  out.png
//! ```

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use pac_decompiler::{decompile_image_with_stats, DecompileOptions};

#[derive(Parser, Debug)]
#[command(name = "pac", about = "Picture As Code: decompile images to editable text, recompile to PNG")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Decompile a still image (PNG / JPG / WebP / GIF) into a `.pac` source file.
    Decompile {
        /// Input image path.
        input: PathBuf,
        /// Output `.pac` path.
        output: PathBuf,
        /// v0.2: adaptive palette floor.  K-means starts here and
        /// grows until the per-pixel residual drops below the target
        /// or `--k-max` is reached.  Set `--k` equal to `--k-max` to
        /// lock K.
        #[arg(long, default_value_t = 6)]
        k: usize,
        /// v0.2: adaptive palette ceiling.
        #[arg(long, default_value_t = 24)]
        k_max: usize,
        /// v0.2: per-channel mean squared error target for auto-K.
        /// Lower = finer palette (more shapes), higher = coarser.
        #[arg(long, default_value_t = 80.0)]
        target_residual: f32,
        /// Drop regions smaller than this many pixels.
        #[arg(long, default_value_t = 400)]
        min_area: u32,
        /// RDP simplification epsilon (pixels).
        #[arg(long, default_value_t = 2.0)]
        epsilon: f32,
        /// Maximum input edge length (0 = no downscale).
        #[arg(long, default_value_t = 768)]
        max_edge: u32,
        /// v0.2: emit raw `polygon(...)` outlines instead of Bezier `path(...)`.
        /// Default behaviour fits cubic Beziers (Schneider 1990) for smoother edges.
        #[arg(long, default_value_t = false)]
        no_smooth: bool,
        /// Schneider curve-fitting tolerance in pixels (lower = more segments).
        #[arg(long, default_value_t = 1.5)]
        curve_eps: f32,
        /// v0.2: disable linear-gradient detection.  Default is to
        /// try a least-squares plane fit per region and emit
        /// `gradient(linear, ...)` when the gradient model
        /// substantially out-explains a flat fill.
        #[arg(long, default_value_t = false)]
        no_gradients: bool,
    },
    /// Render a `.pac` source file to a PNG.
    Compile {
        /// Input `.pac` path.
        input: PathBuf,
        /// Output PNG path.
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Decompile {
            input,
            output,
            k,
            k_max,
            target_residual,
            min_area,
            epsilon,
            max_edge,
            no_smooth,
            curve_eps,
            no_gradients,
        } => {
            let opts = DecompileOptions {
                palette_size_min: k,
                palette_size_max: k_max,
                target_residual,
                min_region_area: min_area,
                rdp_epsilon: epsilon,
                max_edge,
                kmeans_max_iters: 16,
                smooth_curves: !no_smooth,
                curve_eps,
                detect_gradients: !no_gradients,
                gradient_improvement: 0.30,
                gradient_min_delta: 20.0,
            };
            let in_str = input.to_str().context("input path must be UTF-8")?;
            // Load + decompile via the stats-aware entry point so we
            // can print K and residual to the user.
            let img = pac_decompiler::image_io::load_rgba(in_str, opts.max_edge)
                .with_context(|| format!("load {in_str}"))?;
            let (doc, stats) = decompile_image_with_stats(&img, &opts)
                .with_context(|| format!("decompile {in_str}"))?;
            let text = pac_format::write(&doc);
            std::fs::write(&output, text)
                .with_context(|| format!("write {}", output.display()))?;
            println!(
                "✓ wrote {} ({}x{}, K={}, {} shapes [{} gradient], residual={:.1})",
                output.display(),
                doc.canvas.width,
                doc.canvas.height,
                stats.palette_size,
                stats.region_count,
                stats.gradient_count,
                stats.residual,
            );
            // Report the three regions whose fill agrees least with
            // the source pixels.  These are candidates for a future
            // `raster(...)` escape-hatch in PAC v0.3.  Threshold of
            // 100 = "on average each channel is off by >10 / 255".
            let mut ranked: Vec<(usize, f32)> = stats.region_residuals
                .iter().copied().enumerate().collect();
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let worst: Vec<_> = ranked.into_iter()
                .filter(|(_, r)| *r > 100.0)
                .take(3)
                .collect();
            if !worst.is_empty() {
                let mut bits = Vec::with_capacity(worst.len());
                for (idx, resid) in worst {
                    let name = doc.shapes.get(idx).map(|s| s.name.as_str()).unwrap_or("?");
                    bits.push(format!("{name}={resid:.0}"));
                }
                println!("  weakest-fit regions: {}", bits.join(", "));
            }
        }
        Cmd::Compile { input, output } => {
            let src = std::fs::read_to_string(&input)
                .with_context(|| format!("read {}", input.display()))?;
            let doc = pac_format::parse(&src)
                .with_context(|| format!("parse {}", input.display()))?;
            let out_str = output.to_str().context("output path must be UTF-8")?;
            pac_compiler::render_to_png(&doc, out_str)
                .with_context(|| format!("render {}", output.display()))?;
            println!("✓ wrote {}", output.display());
        }
    }
    Ok(())
}
