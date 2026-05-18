//! Editor IR round-trip against the committed `crates/core/tests/golden/fab/`
//! fixtures. Parses each .tps.fab.json via `manifest::parse`, re-serializes
//! through the editor's `serialize::serialize`, re-parses the result, and
//! asserts the two `Manifest`s are equal. Catches regressions in the
//! serializer (default-skipping, scale collapse, color-prefix round-trip).

use std::path::PathBuf;
use unity_sprite_author::manifest::parse;

#[path = "../src/serialize.rs"]
mod serialize_module;

fn fab_paths() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../core/tests/golden/fab/silloutte/PremiumCat_Vampire_Popup.tps.fab.json"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../core/tests/golden/fab/treasure_trove/TreasureTrove.tps.fab.json"),
    ];
    roots.retain(|p| p.exists());
    roots
}

#[test]
fn editor_serialize_round_trips_all_goldens() {
    let paths = fab_paths();
    assert!(!paths.is_empty(), "no golden fab.json fixtures found");
    for path in paths {
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let original = parse(&src)
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let re_emitted = serialize_module::serialize(&original);
        let reparsed = parse(&re_emitted)
            .unwrap_or_else(|e| panic!("reparse {}: {e}\n--- emitted ---\n{re_emitted}", path.display()));
        assert_eq!(original, reparsed, "round-trip drift on {}", path.display());
    }
}
