// v2 fab.json schema tests — landed before the implementation change.
//
// v2 collapses three multiplicative scale fields (Tree.scale,
// Leaf.uiScale, Node.scale flip) into one per-node `Node.scale` vec2.
// Sign carries flip; magnitude carries the composed
// `old uiScale × old canvasScale`. The canvas factor becomes
// mode-implicit (Csa = 0.01, Sma = 1.0) and is applied at exactly one
// seam in the bridge: `Part.offset` is pre-multiplied so the runtime
// per-vert chain drops the `× canvas_scale` step entirely.
//
// See [[fab.md]] (post-cutover) for the schema.

use unity_sprite_author::fab;
use unity_sprite_author::manifest::{
    parse, to_fab_combined, BridgeError, Graphic, ManifestError, Output,
};

// ---------------------------------------------------------------------------
// Group 1 — schema parse rejects legacy v1 fields.

#[test]
fn parse_rejects_tree_level_scale_field() {
    // v1 carried `scale` at the tree level (root canvasScale). v2 drops
    // it entirely — deny_unknown_fields surfaces "unknown field `scale`"
    // with the offending path, which is a clearer migration prompt than a
    // version-bump error would be.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui", "scale": 0.01,
              "children":[{"type":"sprite","sprite":"a"}]
            }]}"#,
    );
    let err = m.unwrap_err();
    let msg = format!("{err}");
    assert!(
        matches!(err, ManifestError::Json(_)),
        "expected JSON unknown-field error, got {err:?}"
    );
    assert!(
        msg.contains("scale"),
        "error must name the offending `scale` field, got: {msg}"
    );
}

#[test]
fn parse_rejects_leaf_ui_scale_field() {
    // v1 carried `uiScale` on sprite leaves (UIIcon._scaleFactor).
    // v2 folds it into the leaf's `scale` magnitude at JSON-emit time.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{"type":"sprite","sprite":"a","uiScale": 53.125}]
            }]}"#,
    );
    let err = m.unwrap_err();
    let msg = format!("{err}");
    assert!(matches!(err, ManifestError::Json(_)));
    assert!(
        msg.contains("uiScale"),
        "error must name the offending `uiScale` field, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Group 2 — mode-implicit canvas_scale: one well-defined seam.

#[test]
fn csa_offset_pre_scaled_by_one_hundredth_at_bridge() {
    // CSA mode → canvas_scale_implicit = 0.01. The bridge pre-multiplies
    // the leaf's anchored position (canvas-pixel units) by 0.01 so it
    // lands in world units on the Part. Runtime sees no canvas_scale.
    //
    // Concrete: a sprite leaf at pos [100, -50] under a CSA tree gives
    // Part.offset = [1.0, -0.5]. SMA leaf at the same pos would give
    // Part.offset = [100.0, -50.0] (×1.0). See `sma_offset_unscaled`.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[
                {"pos":[100, -50], "type":"sprite", "sprite":"a"}
              ]
            }]}"#,
    )
    .unwrap();
    let c = to_fab_combined(&m.trees[0]).unwrap();
    let offset = match &c.parts[0] {
        fab::Part::AtlasSprite { offset, .. } => *offset,
        _ => panic!("expected AtlasSprite"),
    };
    assert_eq!(offset, [1.0, -0.5]);
}

#[test]
fn sma_offset_unscaled_at_bridge() {
    // SMA mode → canvas_scale_implicit = 1.0; offset passes through.
    // SMA trees go through `to_mesh_combined` (not the fab bridge), so
    // exercise that path to confirm the mode-implicit constant is wired
    // into the right adapter too.
    use unity_sprite_author::manifest::to_mesh_combined;
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":-1, "outputPath":"o.asset",
              "children":[
                {"pos":[100, -50], "type":"spriteRenderer", "sprite":"a"}
              ]
            }]}"#,
    )
    .unwrap();
    let mc = to_mesh_combined(&m.trees[0]).unwrap();
    // localToRoot row-major: [m00, m01, _, m03, m10, m11, _, m13].
    // m03/m13 carry the world translation (= offset × 1.0 in SMA).
    let l2r = mc.renderers[0].local_to_root;
    assert_eq!(l2r[3], 100.0, "m03 should equal pos.x at canvas_scale=1");
    assert_eq!(l2r[7], -50.0, "m13 should equal pos.y at canvas_scale=1");
}

// ---------------------------------------------------------------------------
// Group 3 — Node.scale is now the composed leaf magnitude
//           (was: per-axis flip only; ui_scale carried magnitude).

#[test]
fn leaf_scale_magnitude_flows_into_affine_unmodified() {
    // A CSA sprite leaf with `scale: 0.53125` (the v2 carry-over of
    // old uiScale=53.125 × canvasScale=0.01) must surface on the part's
    // Affine.sx/sy as 0.53125 — no remaining ×100 or ×0.01 anywhere in
    // the bridge or runtime. The "one scale source per node" rule.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[
                {"scale": 0.53125, "type":"sprite", "sprite":"a"}
              ]
            }]}"#,
    )
    .unwrap();
    let c = to_fab_combined(&m.trees[0]).unwrap();
    let affine = match &c.parts[0] {
        fab::Part::AtlasSprite { affine, .. } => *affine,
        _ => panic!(),
    };
    assert_eq!(affine.sx, 0.53125);
    assert_eq!(affine.sy, 0.53125);
}

#[test]
fn leaf_scale_composes_through_deep_container_chain() {
    // Mirror the GiftShop GS_Door2 shape (max depth observed in the
    // meow-tower corpus): root → container(scale=[-1,1] flip) →
    // container(no scale) → leaf(scale=0.5). Composed leaf affine
    // must be sx=-0.5, sy=0.5. Verifies the walker still multiplies
    // node.scale per-axis across arbitrary depth under v2 semantics.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{
                "scale":[-1, 1],
                "children":[{
                  "children":[{
                    "scale": 0.5,
                    "type":"sprite", "sprite":"deep"
                  }]
                }]
              }]
            }]}"#,
    )
    .unwrap();
    let c = to_fab_combined(&m.trees[0]).unwrap();
    let affine = match &c.parts[0] {
        fab::Part::AtlasSprite { affine, .. } => *affine,
        _ => panic!(),
    };
    assert_eq!(affine.sx, -0.5);
    assert_eq!(affine.sy, 0.5);
}

// ---------------------------------------------------------------------------
// Group 4 — pivot defaults to the sprite's own tps pivotPoint
//           (not [0.5, 0.5]) when JSON omits it.

#[test]
fn sprite_leaf_omitting_pivot_inherits_none_from_bridge() {
    // No `pivot` in JSON → Part.part_pivot = None. The runtime resolves
    // None against the tps sprite-entry's pivotPoint in build_combined, so
    // a leaf authored without `pivot` automatically lines up with the
    // sprite's natural anchor.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{"type":"sprite", "sprite":"a"}]
            }]}"#,
    )
    .unwrap();
    let c = to_fab_combined(&m.trees[0]).unwrap();
    match &c.parts[0] {
        fab::Part::AtlasSprite { part_pivot, .. } => assert_eq!(*part_pivot, None),
        _ => panic!(),
    }
}

#[test]
fn sprite_leaf_explicit_pivot_overrides_tps_default() {
    // Explicit `pivot` survives as Some(_), overriding the runtime's
    // tps-default lookup — required for leaves that intentionally
    // diverge from the sprite's natural anchor.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{"type":"sprite", "sprite":"a", "pivot":[0.0, 1.0]}]
            }]}"#,
    )
    .unwrap();
    let c = to_fab_combined(&m.trees[0]).unwrap();
    match &c.parts[0] {
        fab::Part::AtlasSprite { part_pivot, .. } => assert_eq!(*part_pivot, Some([0.0, 1.0])),
        _ => panic!(),
    }
}

// ---------------------------------------------------------------------------
// Group 5 — schema renames: trees → combined, sizeDelta → size,
//           rotDeg → rotDegCCW. Old names hard-error.

#[test]
fn parse_rejects_legacy_trees_key() {
    // Old top-level key was `trees`. New schema requires `combined`.
    let m = parse(r#"{"version":1, "trees":[]}"#);
    assert!(matches!(m.unwrap_err(), ManifestError::Json(_)));
}

#[test]
fn parse_rejects_legacy_size_delta_key() {
    // Old per-node key was `sizeDelta`. New schema uses `size`.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{"type":"sprite","sprite":"a","sizeDelta":[10,10],"method":"MX"}]
            }]}"#,
    );
    let err = m.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("sizeDelta"), "got: {msg}");
}

#[test]
fn parse_rejects_legacy_rot_deg_key() {
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{"type":"sprite","sprite":"a","rotDeg":45}]
            }]}"#,
    );
    let err = m.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("rotDeg"), "got: {msg}");
}

#[test]
fn parse_accepts_new_canonical_field_names() {
    parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{"type":"sprite","sprite":"a","size":[10,10],"rotDegCCW":45,"method":"MX"}]
            }]}"#,
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Group 6 — `size` on size-fitted sprite leaves defaults to the sprite's
//           natural rect size when omitted (mirrors the pivot-tps default).

#[test]
fn size_fitted_sprite_leaf_omitting_size_passes_through_none() {
    // No `size` for a method that requires it → Part.AtlasSprite.size = None.
    // build_combined resolves None against the sprite entry's pixel rect.
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X", "mode":"ui",
              "children":[{"type":"sprite","sprite":"a","method":"R3C3"}]
            }]}"#,
    )
    .unwrap();
    let c = to_fab_combined(&m.trees[0]).unwrap();
    match &c.parts[0] {
        fab::Part::AtlasSprite { size, .. } => assert_eq!(*size, None),
        _ => panic!(),
    }
}

// ---------------------------------------------------------------------------
// Group 7 — bridge errors still surface; CSA-output guard intact.

#[test]
fn sma_tree_into_fab_adapter_still_errors() {
    let m = parse(
        r#"{ "version":1, "combined":[{
              "name":"X",
              "mode":"sma-canvas", "fileId":1, "outputPath":"o.asset",
              "children":[{"type":"spriteRenderer","sprite":"a"}]
            }]}"#,
    )
    .unwrap();
    assert!(matches!(
        to_fab_combined(&m.trees[0]).unwrap_err(),
        BridgeError::OutputMismatch { .. }
    ));
}
