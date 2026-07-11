//! End-to-end PAC round-trip integration test using the library API.
//!
//! Generates a synthetic two-colour test image, runs
//! `pac_decompiler::decompile_image` → `pac_format::write` →
//! `pac_format::parse` → `pac_compiler::render`, and checks the
//! output: shape count, palette colours, and pixel-level fidelity at
//! the centre of the foreground block.

use pac_decompiler::{decompile_image, DecompileOptions};
use pac_decompiler::image_io::RgbaImage;

fn make_two_color_image() -> RgbaImage {
    let w = 80u32;
    let h = 60u32;
    let mut pix = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let inside = (20..60).contains(&x) && (15..45).contains(&y);
            let c: [u8; 4] = if inside { [0xff, 0x00, 0x00, 0xff] } else { [0x1a, 0x1a, 0x2e, 0xff] };
            pix.extend_from_slice(&c);
        }
    }
    RgbaImage { width: w, height: h, pixels: pix }
}

#[test]
fn round_trip_two_color_rectangle_preserves_block() {
    let img = make_two_color_image();
    let opts = DecompileOptions {
        palette_size_min: 2,
        palette_size_max: 2,
        target_residual: 1000.0,
        min_region_area: 10,
        rdp_epsilon: 1.0,
        max_edge: 0,
        kmeans_max_iters: 16,
        // Force polygon output here so the test asserts pixel-level
        // round-trip without any Bezier smoothing slack.
        smooth_curves: false,
        curve_eps: 1.5,
        // Two flat-coloured regions: no real gradient to find.
        detect_gradients: false,
        gradient_improvement: 0.30,
        gradient_min_delta: 20.0,
    };

    let doc = decompile_image(&img, &opts).expect("decompile");
    assert_eq!(doc.canvas.width, 80);
    assert_eq!(doc.canvas.height, 60);
    assert!(doc.background.is_some(), "background expected");
    assert_eq!(doc.shapes.len(), 1, "expected exactly one foreground polygon, got {}", doc.shapes.len());

    // Round-trip the text format.
    let text = pac_format::write(&doc);
    let reparsed = pac_format::parse(&text).expect("reparse");
    assert_eq!(reparsed, doc, "round-trip mismatch:\n{text}");

    // Render and check the centre / corner pixels survive.
    let render = pac_compiler::render(&reparsed).expect("render");
    let centre = render.get_pixel(40, 30);
    assert!(
        centre.0[0] > 200 && centre.0[1] < 60 && centre.0[2] < 60,
        "centre pixel not red: {:?}",
        centre.0
    );
    let corner = render.get_pixel(2, 2);
    assert!(
        corner.0[0] < 60 && corner.0[1] < 60 && corner.0[2] > 30,
        "corner pixel not navy: {:?}",
        corner.0
    );
}

#[test]
fn parses_hello_pac_example() {
    // The committed example must always parse + render cleanly.
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let example = manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .join("examples/hello.pac");
    let src = std::fs::read_to_string(&example).unwrap_or_else(|e| {
        panic!("failed to read {example:?}: {e}");
    });
    let doc = pac_format::parse(&src).expect("parse hello.pac");
    let render = pac_compiler::render(&doc).expect("render hello.pac");
    assert_eq!(render.width(), 320);
    assert_eq!(render.height(), 200);
}
