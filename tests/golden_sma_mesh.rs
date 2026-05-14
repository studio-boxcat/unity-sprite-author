// Phase 2b foundation test for `unity_sprite_author::mesh_emit`. Currently
// `#[ignore]`-d because the emit stub is unimplemented — running it with
// `--ignored` returns `unimplemented!()` until Phase 2b lands.
//
// The fixture `tests/golden/sma/box_29_ghost/Box_29_Ghost.asset` is a real
// SMA-published multi-mesh asset (32 Mesh sub-objects, 6165 lines). Once
// `emit_mesh_asset` is functional, this test will load the fixture, reverse-
// parse it into `MeshAsset` records, re-emit, and assert byte-exact
// equality. That round-trip catches every channel layout, every empty-field
// shape, and the multi-sub-asset header sequence in one pass.

use std::fs;
use std::path::Path;

use unity_sprite_author::mesh_emit::{self, MeshAsset};

#[test]
#[ignore = "Phase 2b not implemented — mesh_emit::emit_mesh_asset is a stub"]
fn box_29_ghost_roundtrip() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/sma/box_29_ghost/Box_29_Ghost.asset");
    let _golden = fs::read(&path).expect("read fixture");

    // TODO: parse golden → Vec<MeshAsset>, re-emit, compare.
    // For now the empty input asserts emit can be invoked once it lands.
    let _bytes = mesh_emit::emit_mesh_asset(&[] as &[MeshAsset]);
    unreachable!("emit_mesh_asset returned — implementation has landed; finish the round-trip");
}
