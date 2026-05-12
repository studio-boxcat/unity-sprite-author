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

#[test]
#[ignore = "byte-exact target; run with --include-ignored to attempt"]
fn silloutte1_byte_exact() {
    let dir = stage_fixture("byte_exact");

    let inputs = GenerateInputs {
        tpsheet_path: &dir.join("PremiumCat_Vampire_Popup.tpsheet"),
        tps_path: &dir.join("PremiumCat_Vampire_Popup.tps"),
        atlas_png_path: &dir.join("PremiumCat_Vampire_Popup.png"),
        sprite_dir: &dir.join("PremiumCat_Vampire_Popup"),
        prefix: "",
        ppu: 100.0,
    };
    let out = pipeline::generate(&inputs).expect("generate failed");

    let combined_path = dir.join("PremiumCat_Vampire_Popup/Silloutte1.asset");
    assert!(
        out.written_paths.iter().any(|p| p == &combined_path),
        "Silloutte1.asset not in written set",
    );

    let got = fs::read(&combined_path).unwrap();
    let golden = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/fab/silloutte/Silloutte1.asset"),
    )
    .unwrap();

    if got == golden {
        let _ = fs::remove_dir_all(&dir);
        return; // byte-exact match
    }

    // Mismatch — dump the first divergence with surrounding context.
    let off = first_diff(&got, &golden).unwrap();
    let lo = off.saturating_sub(32);
    let hi_g = (off + 32).min(got.len());
    let hi_e = (off + 32).min(golden.len());
    let _ = fs::create_dir_all("target/diff");
    let _ = fs::write("target/diff/Silloutte1.actual", &got);
    let _ = fs::write("target/diff/Silloutte1.expected", &golden);
    panic!(
        "Silloutte1 byte mismatch at offset {off} \
         (got len={}, golden len={}):\n\
            got     [{lo}..{hi_g}]: {:?}\n\
            golden  [{lo}..{hi_e}]: {:?}\n\
         Diff files written to target/diff/Silloutte1.{{actual,expected}}.",
        got.len(),
        golden.len(),
        String::from_utf8_lossy(&got[lo..hi_g]),
        String::from_utf8_lossy(&golden[lo..hi_e]),
    );
}
