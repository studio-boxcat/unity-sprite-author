// Phase 1b *diagnostic* test — `PA_InfinitePencil_Clock.asset` does NOT
// match byte-exact against the committed golden because the regenerated
// `.tpsheet` (TexturePacker output today) differs from the historical
// `.tpsheet` that emitted the committed `_sprite.asset` bytes.
//
// Per-tpsheet emit of the same atlas's constituent sprites also diverges
// from their committed `.asset` files (Clock1.asset, Clock2.asset,
// Color_FFEBBE.asset — see TODO.md "CSA migration — Phase 1b" entry for
// the bit-level trace). So this isn't a Rust pipeline bug: same code +
// same tpsheet = bit-stable output, and Silloutte oracle (3/3 byte-exact)
// proves the pipeline against an aligned tpsheet.
//
// Pinned `#[ignore]` so the diagnostic isn't lost. The proper resolution
// is a corpus regen (TexturePackerCLI on every atlas → re-emit every
// sprite from the new tpsheet → commit the resulting diff). After that
// regen this test should pass byte-exact and become unignored. Until
// then, `cargo test --test golden_fab_pa_clock -- --ignored` reproduces
// the historical drift signature.
//
// Signature when failing: `m_Rect.height` 540.66345 → 540.66296,
// cascading into `m_Offset.y` (−8.262054 → −8.262299) and `m_Pivot.y`
// (0.48471868 → 0.4847182).

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
#[ignore = "Phase 1b tpsheet drift — committed sprite was emitted from a historical tpsheet; see TODO.md"]
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
