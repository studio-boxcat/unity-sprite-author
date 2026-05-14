// End-to-end byte-exactness for the v3 manifest path on the Silloutte fixture.
//
// The plain `golden_fab_silloutte` test already pins v1 → byte-exact emit for
// `Silloutte{1,2,3}.asset`. This test:
//   1. Parses the v3 fixture (`PremiumCat_Vampire_Popup.tps.fab.v3.json`).
//   2. Bridges each `Tree` through `manifest::to_fab_combined`.
//   3. Asserts the resulting `fab::Combined` is equivalent to the v1
//      manifest's parsed Combined — same name, scale, root_anchored, and
//      part-for-part identical AtlasSprite/Polygon fields.
//
// If this passes, v3 reproduces v1's typed AST and therefore inherits v1's
// byte-exact `.asset` emit through `combine::build_combined`.

use std::fs;
use std::path::Path;

use unity_sprite_author::{fab, manifest};

#[test]
fn v3_silloutte_bridges_to_v1_equivalent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fab/silloutte");
    let v1 = fab::parse(
        &fs::read_to_string(root.join("PremiumCat_Vampire_Popup.tps.fab.json"))
            .expect("v1 fixture"),
    )
    .expect("v1 parse");
    let v3 = manifest::parse(
        &fs::read_to_string(root.join("PremiumCat_Vampire_Popup.tps.fab.v3.json"))
            .expect("v3 fixture"),
    )
    .expect("v3 parse");

    assert_eq!(
        v1.combined.len(),
        v3.trees.len(),
        "tree count mismatch"
    );

    for (v1c, tree) in v1.combined.iter().zip(v3.trees.iter()) {
        let bridged = manifest::to_fab_combined(tree).expect("bridge");
        assert_combined_eq(v1c, &bridged);
    }
}

#[test]
fn v3_silloutte_byte_exact() {
    // Stage the Silloutte fixture into a temp dir, swap v1 fab.json for
    // v3 form (renamed onto the canonical path the pipeline discovers),
    // run `pipeline::generate`, byte-diff each Silloutte*.asset against
    // the committed golden. Proves v3 → byte-exact `.asset` end-to-end.

    use std::path::PathBuf;
    use unity_sprite_author::pipeline::{self, GenerateInputs};

    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fab/silloutte");
    let dst: PathBuf = std::env::temp_dir().join("uspa_silloutte_v3");
    let _ = fs::remove_dir_all(&dst);
    fs::create_dir_all(&dst).unwrap();

    // Copy atlas inputs (everything except the v1 fab.json).
    for entry in fs::read_dir(&src).unwrap().flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s == "PremiumCat_Vampire_Popup.tps.fab.json" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_file() {
            fs::copy(&from, &to).unwrap();
        }
    }
    // Drop v3 fab.json onto the canonical fab.json path.
    fs::copy(
        src.join("PremiumCat_Vampire_Popup.tps.fab.v3.json"),
        dst.join("PremiumCat_Vampire_Popup.tps.fab.json"),
    )
    .unwrap();

    // Stage Silloutte*.asset.meta inside the sprite_dir so the GUID-preserve
    // branch fires (mirrors the v1 byte-exact test).
    let sprite_dir = dst.join("PremiumCat_Vampire_Popup");
    fs::create_dir_all(&sprite_dir).unwrap();
    for s in ["Silloutte1", "Silloutte2", "Silloutte3"] {
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

    for sprite in ["Silloutte1", "Silloutte2", "Silloutte3"] {
        let got = fs::read(sprite_dir.join(format!("{sprite}.asset"))).unwrap();
        let golden = fs::read(src.join(format!("{sprite}.asset"))).unwrap();
        assert_eq!(got, golden, "{sprite}.asset bytes differ (v3 ≠ v1 emit)");
    }
}

fn assert_combined_eq(a: &fab::Combined, b: &fab::Combined) {
    assert_eq!(a.name, b.name, "name");
    assert_eq!(a.canvas_scale, b.canvas_scale, "canvas_scale on {}", a.name);
    assert_eq!(
        a.root_anchored, b.root_anchored,
        "root_anchored on {}",
        a.name
    );
    assert_eq!(a.parts.len(), b.parts.len(), "parts count on {}", a.name);
    for (i, (ap, bp)) in a.parts.iter().zip(b.parts.iter()).enumerate() {
        assert_eq!(ap, bp, "part {i} of {}", a.name);
    }
}
