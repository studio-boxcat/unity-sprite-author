// End-to-end byte-exactness for the fab path on the Silloutte fixture.
//
// Stages the committed atlas + `.tps.fab.json` into a temp dir, runs
// `pipeline::generate`, and byte-diffs each `Silloutte*.asset` against the
// committed golden. Covers parse → bridge (manifest::to_fab_combined) →
// combine::build_combined → emit, end-to-end.

use std::fs;
use std::path::{Path, PathBuf};

use unity_sprite_author::pipeline::{self, GenerateInputs};

#[test]
fn silloutte_byte_exact() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fab/silloutte");
    let dst: PathBuf = std::env::temp_dir().join("uspa_silloutte");
    let _ = fs::remove_dir_all(&dst);
    fs::create_dir_all(&dst).unwrap();

    for entry in fs::read_dir(&src).unwrap().flatten() {
        let from = entry.path();
        if from.is_file() {
            fs::copy(&from, dst.join(entry.file_name())).unwrap();
        }
    }

    // Stage Silloutte*.asset.meta inside the sprite_dir so the
    // GUID-preserve branch fires.
    let sprite_dir = dst.join("PremiumCat_Vampire_Popup");
    fs::create_dir_all(&sprite_dir).unwrap();
    for s in ["Silloutte1", "Silloutte2"] {
        let meta = dst.join(format!("{s}.asset.meta"));
        if meta.exists() {
            fs::copy(&meta, sprite_dir.join(format!("{s}.asset.meta"))).unwrap();
        }
    }

    pipeline::generate(&GenerateInputs {
        tpsheet_path: &dst.join("PremiumCat_Vampire_Popup.tpsheet"),
        tps_path: &dst.join("PremiumCat_Vampire_Popup.tps"),
        atlas_png_path: &dst.join("PremiumCat_Vampire_Popup.png"),
        sprite_dir: &sprite_dir,
        prefix: "",
        ppu: 100.0,
    })
    .expect("pipeline");

    for sprite in ["Silloutte1", "Silloutte2"] {
        let got = fs::read(sprite_dir.join(format!("{sprite}.asset"))).unwrap();
        let golden = fs::read(src.join(format!("{sprite}.asset"))).unwrap();
        assert_eq!(got, golden, "{sprite}.asset bytes differ");
    }
}
