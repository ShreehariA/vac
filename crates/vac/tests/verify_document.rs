//! `vac verify --vac FILE`: measure *any* `.vac` against a video — a
//! hand-edited source, or one an older version wrote. This is the half of
//! "editable" that makes an edit measurable.

use std::path::PathBuf;

use vac::DecompileOptions;

fn examples_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

#[test]
fn verifying_the_decompilers_own_output_matches_verify() {
    let doc = vac::read(examples_dir().join("two_balls.vac")).unwrap();
    let dir = std::env::temp_dir().join(format!("vac-verify-doc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gif = dir.join("two_balls.gif");
    vac::write_video(&doc, &gif).unwrap();

    let auto = vac::verify(&gif, &DecompileOptions::default()).unwrap();
    let again = vac::verify_document(&gif, auto.document.clone()).unwrap();
    assert_eq!(auto.fidelity.residual, again.fidelity.residual);
    assert_eq!(auto.fidelity.ink_iou, again.fidelity.ink_iou);
    assert_eq!(auto.vac_bytes, again.vac_bytes);

    // A hand edit that moves the recovered shapes 40 px down measurably
    // costs fidelity. (The hand-written source is not the reference here:
    // GIF stores delays in centiseconds, so the 30 fps source plays at
    // 33.3 fps once encoded, and only the decompiled document matches the
    // GIF's real timing.)
    let mut moved = auto.document.clone();
    for scene in &mut moved.scenes {
        for st in &mut scene.statements {
            match st {
                vac::Statement::Let { name, shape: vac::Shape::Ellipse { cy, .. } } if name != "bg" => *cy += 40.0,
                vac::Statement::Animate(a) => {
                    for kf in &mut a.keyframes {
                        for t in &mut kf.transforms {
                            if let vac::Transform::Position(_, y) = t {
                                *y += 40.0;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let worse = vac::verify_document(&gif, moved).unwrap();
    assert!(
        worse.fidelity.ink_iou < auto.fidelity.ink_iou - 0.3,
        "moving every shape 40 px should cost ink overlap: {:.4} vs {:.4}",
        worse.fidelity.ink_iou,
        auto.fidelity.ink_iou
    );
    let _ = std::fs::remove_dir_all(&dir);
}
