// Byte-exact test for `PA_InfinitePencil_Clock.asset` — a CSA-published
// combined sprite that surfaces a 5-ULP Y-axis residual in the fab emit
// path. Pinned `#[ignore]` so it lives as a fast oracle for the next
// debug iteration without breaking `cargo test`.
//
// Repro: `cargo test --test golden_fab_pa_clock -- --ignored`.
// Diff dump: `target/diff/PA_InfinitePencil_Clock.{actual,expected}`.
//
// Signature: `m_Rect.height` 540.66345 → 540.66296, cascading into
// `m_Offset.y` (−8.262054 → −8.262299) and `m_Pivot.y` (0.48471868 →
// 0.4847182). X-axis fields match byte-exact. Combined parts include
// 1 UISolid color polygon + 6 UIIcons (ID / FX-desugared / MXY).

use std::fs;
use std::path::{Path, PathBuf};

use unity_sprite_author::pipeline::{self, GenerateInputs};

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x != y).or({
        if a.len() != b.len() { Some(a.len().min(b.len())) } else { None }
    })
}

#[test]
#[ignore = "Phase 1b residual — see TODO.md and docs/sma-migration.md"]
fn pa_clock_byte_exact() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fab/pa_clock");
    let dst: PathBuf = std::env::temp_dir().join("uspa_pa_clock");
    let _ = fs::remove_dir_all(&dst);
    copy_dir(&src, &dst).unwrap();

    // GUID-preserve stage: the committed .asset.meta must sit inside the
    // output sprite_dir so the pipeline reads it instead of minting fresh.
    let sprite_dir = dst.join("PencilAdPopup_InfinitePencil");
    fs::create_dir_all(&sprite_dir).unwrap();
    fs::copy(
        dst.join("PA_InfinitePencil_Clock.asset.meta"),
        sprite_dir.join("PA_InfinitePencil_Clock.asset.meta"),
    )
    .unwrap();

    let inputs = GenerateInputs {
        tpsheet_path: &dst.join("PencilAdPopup_InfinitePencil.tpsheet"),
        tps_path: &dst.join("PencilAdPopup_InfinitePencil.tps"),
        atlas_png_path: &dst.join("PencilAdPopup_InfinitePencil.png"),
        sprite_dir: &sprite_dir,
        prefix: "",
        ppu: 100.0,
    };
    pipeline::generate(&inputs).expect("generate failed");

    let got = fs::read(sprite_dir.join("PA_InfinitePencil_Clock.asset")).unwrap();
    let golden = fs::read(src.join("PA_InfinitePencil_Clock.asset")).unwrap();
    if got == golden {
        let _ = fs::remove_dir_all(&dst);
        return;
    }
    let off = first_diff(&got, &golden).unwrap();
    let _ = fs::create_dir_all("target/diff");
    let _ = fs::write("target/diff/PA_InfinitePencil_Clock.actual", &got);
    let _ = fs::write("target/diff/PA_InfinitePencil_Clock.expected", &golden);
    panic!(
        "PA_InfinitePencil_Clock byte mismatch at offset {off} \
         (got len={}, golden len={}); diff files in target/diff/",
        got.len(),
        golden.len()
    );
}
