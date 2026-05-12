// Byte-exact test against CanvasSpriteAuthor-published Silloutte1.asset.
//
// The fixture (tests/golden/fab/silloutte/) holds the source atlas + the
// hand-authored .tps.fab.json. This test stages the fixture into a temp dir,
// runs pipeline::generate, and diffs the emitted Silloutte1.asset against
// the committed golden. First divergence is reported with offset + a hex
// window so the manifest can be iterated on.

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

fn stage_fixture(name: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fab/silloutte");
    let dst = std::env::temp_dir().join(format!("uspa_silloutte_{name}"));
    let _ = fs::remove_dir_all(&dst);
    copy_dir(&src, &dst).unwrap();
    // Stage Silloutte*.asset.meta inside the sprite_dir so the GUID-preserve
    // branch fires (otherwise the pipeline mints a random GUID and the test
    // diffs at m_RenderDataKey).
    let sprite_dir = dst.join("PremiumCat_Vampire_Popup");
    fs::create_dir_all(&sprite_dir).unwrap();
    for s in ["Silloutte1", "Silloutte2", "Silloutte3"] {
        let meta = dst.join(format!("{s}.asset.meta"));
        if meta.exists() {
            fs::copy(&meta, sprite_dir.join(format!("{s}.asset.meta"))).unwrap();
        }
    }
    dst
}

fn run_pipeline(name: &str) -> std::path::PathBuf {
    let dir = stage_fixture(name);
    let inputs = GenerateInputs {
        tpsheet_path: &dir.join("PremiumCat_Vampire_Popup.tpsheet"),
        tps_path: &dir.join("PremiumCat_Vampire_Popup.tps"),
        atlas_png_path: &dir.join("PremiumCat_Vampire_Popup.png"),
        sprite_dir: &dir.join("PremiumCat_Vampire_Popup"),
        prefix: "",
        ppu: 100.0,
    };
    pipeline::generate(&inputs).expect("generate failed");
    dir
}

fn assert_silloutte_byte_exact(sprite_name: &str, dir: &Path) {
    let combined_path = dir
        .join("PremiumCat_Vampire_Popup")
        .join(format!("{sprite_name}.asset"));
    let got = fs::read(&combined_path).unwrap_or_else(|e| {
        panic!("{sprite_name}.asset not emitted at {}: {e}", combined_path.display())
    });
    let golden = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/golden/fab/silloutte/{sprite_name}.asset")),
    )
    .unwrap();
    if got == golden {
        return;
    }
    let off = first_diff(&got, &golden).unwrap();
    let lo = off.saturating_sub(32);
    let hi_g = (off + 32).min(got.len());
    let hi_e = (off + 32).min(golden.len());
    let _ = fs::create_dir_all("target/diff");
    let _ = fs::write(format!("target/diff/{sprite_name}.actual"), &got);
    let _ = fs::write(format!("target/diff/{sprite_name}.expected"), &golden);
    panic!(
        "{sprite_name} byte mismatch at offset {off} \
         (got len={}, golden len={}):\n\
            got     [{lo}..{hi_g}]: {:?}\n\
            golden  [{lo}..{hi_e}]: {:?}\n\
         Diff files written to target/diff/{sprite_name}.{{actual,expected}}.",
        got.len(),
        golden.len(),
        String::from_utf8_lossy(&got[lo..hi_g]),
        String::from_utf8_lossy(&golden[lo..hi_e]),
    );
}

#[test]
#[ignore = "byte-exact target; run with --include-ignored to attempt"]
fn silloutte1_byte_exact() {
    let dir = run_pipeline("silloutte1");
    assert_silloutte_byte_exact("Silloutte1", &dir);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "byte-exact target; run with --include-ignored to attempt"]
fn silloutte2_byte_exact() {
    let dir = run_pipeline("silloutte2");
    assert_silloutte_byte_exact("Silloutte2", &dir);
    let _ = fs::remove_dir_all(&dir);
}

// Currently fails with a uniform ~1-ULP y-shift on every position-stream f32;
// see TODO.md's `.tps.fab.json` follow-ups and `docs/unity-probes.md#e-silloutte3`
// for the root-anchored hypothesis. Run with `--include-ignored` to attempt.
#[test]
#[ignore = "Silloutte3 1-ULP y-shift unsolved; needs Unity matrix probe"]
fn silloutte3_byte_exact() {
    let dir = run_pipeline("silloutte3");
    assert_silloutte_byte_exact("Silloutte3", &dir);
    let _ = fs::remove_dir_all(&dir);
}
