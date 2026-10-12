//! Tier 4 — corpus fidelity against a committed baseline.
//!
//! This is the only tier that measures *progress*. The hermetic tier
//! (`hermetic.rs`) feeds the decompiler the renderer's own flat-shaded
//! output and scores 0.95–0.9997 on everything, which makes it a very
//! sensitive regression detector and no evidence at all about real
//! video. These numbers are the evidence.
//!
//! # Why this cannot gate CI
//!
//! `examples/wild/inputs/` is gitignored in its entirety. Five inputs are
//! regenerated locally by ffmpeg from committed scripts and filters; five
//! are re-downloaded from Wikimedia Commons by
//! `examples/wild/scripts/fetch_real_gifs.sh`. So a checkout does not
//! have them, CI never fetches them, and an upstream re-encode would turn
//! a green build red for reasons having nothing to do with this code.
//!
//! The design that follows from that:
//!
//! - **Missing input → skip, loudly.** Never fail.
//! - **Input content changed → skip, loudly**, naming both hashes. Each
//!   baseline row records an FNV-1a hash of the input bytes, so "the file
//!   moved under us" is reported as exactly that rather than as a
//!   fidelity regression.
//! - **Tolerance bands, never equality.** `tiny-skia` selects SIMD
//!   backends by target feature, so f32 rasterisation is not guaranteed
//!   bit-identical between an x86_64 CI runner and an aarch64 laptop.
//!   Bit-exactness is the right assertion *within* one machine — that is
//!   what `properties.rs` asserts — and the wrong one across machines.
//! - **Recording is separate from gating.** `VAC_UPDATE_BASELINES=1`
//!   rewrites the file; nothing rewrites it implicitly. A baseline that
//!   silently self-updates measures nothing.
//!
//! ```text
//! cargo test --release -p vac --test corpus -- --nocapture
//! VAC_UPDATE_BASELINES=1 cargo test --release -p vac --test corpus -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use vac::DecompileOptions;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn inputs_dir() -> PathBuf {
    repo_root().join("examples/wild/inputs")
}

fn baseline_path() -> PathBuf {
    repo_root().join("benches/corpus-baseline.tsv")
}

/// Ink IoU may drift this much between platforms before it counts as a
/// regression. Chosen to absorb SIMD rasteriser differences, not real
/// change.
const IOU_TOL: f64 = 0.01;
/// Per-channel MSE tolerance, as a fraction of the recorded value.
const RESIDUAL_TOL_FRAC: f64 = 0.05;
/// How far the single worst frame's ink IoU may fall. Looser than the
/// mean's tolerance (one frame is noisier than ninety), but a collapse is
/// exactly what a mean hides: in v1.3's first draft a mis-tracked colour
/// channel dropped `mandelbrot.mp4`'s worst frame 0.40 → 0.09 while its
/// mean moved 0.005 and passed.
const WORST_IOU_TOL: f64 = 0.05;

#[derive(Debug, Clone, PartialEq)]
struct Row {
    input: String,
    hash: u64,
    frames: usize,
    ink_iou: f64,
    worst_iou: f64,
    residual: f64,
    psnr_db: f64,
    missing_ink: f64,
    spurious_ink: f64,
    shapes: usize,
    animate_blocks: usize,
    keyframes: usize,
    vac_bytes: usize,
}

const HEADER: &str = "input\thash\tframes\tink_iou\tworst_iou\tresidual\tpsnr_db\tmissing_ink\tspurious_ink\tshapes\tanimate\tkeyframes\tvac_bytes";

impl Row {
    fn to_tsv(&self) -> String {
        format!(
            "{}\t{:016x}\t{}\t{:.4}\t{:.4}\t{:.2}\t{:.2}\t{:.4}\t{:.4}\t{}\t{}\t{}\t{}",
            self.input,
            self.hash,
            self.frames,
            self.ink_iou,
            self.worst_iou,
            self.residual,
            self.psnr_db,
            self.missing_ink,
            self.spurious_ink,
            self.shapes,
            self.animate_blocks,
            self.keyframes,
            self.vac_bytes
        )
    }

    fn from_tsv(line: &str) -> Option<Row> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 13 {
            return None;
        }
        Some(Row {
            input: f[0].to_string(),
            hash: u64::from_str_radix(f[1], 16).ok()?,
            frames: f[2].parse().ok()?,
            ink_iou: f[3].parse().ok()?,
            worst_iou: f[4].parse().ok()?,
            residual: f[5].parse().ok()?,
            psnr_db: f[6].parse().ok()?,
            missing_ink: f[7].parse().ok()?,
            spurious_ink: f[8].parse().ok()?,
            shapes: f[9].parse().ok()?,
            animate_blocks: f[10].parse().ok()?,
            keyframes: f[11].parse().ok()?,
            vac_bytes: f[12].parse().ok()?,
        })
    }
}

/// FNV-1a over the file bytes. Enough to notice an upstream re-encode,
/// and it needs no dependency.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn measure_one(path: &Path) -> Option<Row> {
    let bytes = std::fs::read(path).ok()?;
    let v = match vac::verify(path, &DecompileOptions::default()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  !! {}: {e}", path.display());
            return None;
        }
    };
    let f = &v.fidelity;
    Some(Row {
        input: path.file_name()?.to_string_lossy().into_owned(),
        hash: fnv1a(&bytes),
        frames: f.frames_compared,
        ink_iou: f.ink_iou,
        worst_iou: f.worst_iou,
        residual: f.residual,
        psnr_db: if f.psnr_db.is_finite() { f.psnr_db } else { 99.0 },
        missing_ink: f.missing_ink,
        spurious_ink: f.spurious_ink,
        shapes: v.shapes,
        animate_blocks: v.animate_blocks,
        keyframes: v.keyframes,
        vac_bytes: v.vac_bytes,
    })
}

/// Corpus inputs, in a stable order. The `ai_*` stills are excluded:
/// they are single-frame videos built for PAC's still-image matrices and
/// measure nothing temporal.
fn corpus_inputs() -> Vec<PathBuf> {
    let dir = inputs_dir();
    let mut out: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| {
                let p = e.ok()?.path();
                let name = p.file_name()?.to_string_lossy().into_owned();
                if name.starts_with("ai_") {
                    return None;
                }
                let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
                (ext == "gif" || ext == "mp4").then_some(p)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

fn load_baseline() -> BTreeMap<String, Row> {
    let text = match std::fs::read_to_string(baseline_path()) {
        Ok(t) => t,
        Err(_) => return BTreeMap::new(),
    };
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("input\t") && !l.trim().is_empty())
        .filter_map(Row::from_tsv)
        .map(|r| (r.input.clone(), r))
        .collect()
}

#[test]
fn corpus_fidelity_matches_the_committed_baseline() {
    let update = std::env::var("VAC_UPDATE_BASELINES").is_ok();
    let inputs = corpus_inputs();

    if inputs.is_empty() {
        eprintln!(
            "SKIPPED: no corpus inputs under {}.\n\
             They are gitignored by design — regenerate with \
             `examples/wild/scripts/fetch_real_gifs.sh` and the ffmpeg lines in \
             docs/v0.4-capability-matrix.md.",
            inputs_dir().display()
        );
        return;
    }

    let baseline = load_baseline();
    let mut measured: Vec<Row> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut skipped = 0usize;

    println!(
        "\n{:<28} {:>7} {:>8} {:>9} {:>8} {:>9} {:>7}",
        "input", "IoU", "worst", "residual", "psnr dB", "spurious", "shapes"
    );
    println!("{}", "-".repeat(82));

    for path in &inputs {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(row) = measure_one(path) else {
            eprintln!("SKIPPED {name}: could not be measured");
            skipped += 1;
            continue;
        };

        println!(
            "{:<28} {:>7.4} {:>8.4} {:>9.2} {:>8.2} {:>8.1}% {:>7}",
            name,
            row.ink_iou,
            row.worst_iou,
            row.residual,
            row.psnr_db,
            row.spurious_ink * 100.0,
            row.shapes
        );

        if let Some(base) = baseline.get(&name) {
            if base.hash != row.hash {
                eprintln!(
                    "SKIPPED {name}: input bytes changed (baseline {:016x}, found {:016x}). \
                     Re-fetch or re-record; this is not a fidelity regression.",
                    base.hash, row.hash
                );
                skipped += 1;
                measured.push(row);
                continue;
            }
            if row.ink_iou < base.ink_iou - IOU_TOL {
                failures.push(format!(
                    "{name}: ink IoU regressed {:.4} → {:.4} (tolerance {IOU_TOL})",
                    base.ink_iou, row.ink_iou
                ));
            }
            if row.worst_iou < base.worst_iou - WORST_IOU_TOL {
                failures.push(format!(
                    "{name}: worst-frame ink IoU collapsed {:.4} → {:.4} (tolerance {WORST_IOU_TOL})",
                    base.worst_iou, row.worst_iou
                ));
            }
            let ceiling = base.residual * (1.0 + RESIDUAL_TOL_FRAC) + 1.0;
            if row.residual > ceiling {
                failures.push(format!(
                    "{name}: residual regressed {:.2} → {:.2} (ceiling {ceiling:.2})",
                    base.residual, row.residual
                ));
            }
        } else if !update {
            eprintln!("NOTE {name}: no baseline row yet — run with VAC_UPDATE_BASELINES=1");
        }
        measured.push(row);
    }
    println!();

    if update {
        let mut out = String::new();
        out.push_str(
            "# VAC corpus fidelity baseline.\n\
             #\n\
             # Regenerate:  VAC_UPDATE_BASELINES=1 cargo test --release -p vac --test corpus\n\
             #\n\
             # `hash` is FNV-1a over the input file's bytes. The inputs themselves are\n\
             # gitignored and re-fetched, so a hash mismatch means the source material\n\
             # changed — the row is skipped rather than reported as a regression.\n\
             #\n\
             # ink_iou / worst_iou / residual / psnr_db are FIDELITY: how much of the\n\
             # picture survived. shapes / animate / keyframes / vac_bytes are STRUCTURE:\n\
             # what it cost. Only the first group says whether VAC is getting better.\n",
        );
        out.push_str(HEADER);
        out.push('\n');
        for r in &measured {
            out.push_str(&r.to_tsv());
            out.push('\n');
        }
        std::fs::create_dir_all(baseline_path().parent().unwrap()).unwrap();
        std::fs::write(baseline_path(), out).unwrap();
        println!(
            "wrote {} rows to {}",
            measured.len(),
            baseline_path().display()
        );
        return;
    }

    if skipped > 0 {
        eprintln!("{skipped} input(s) skipped — see notes above.");
    }
    assert!(
        failures.is_empty(),
        "corpus fidelity regressed:\n  {}",
        failures.join("\n  ")
    );
}
