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
    // None against the Unity RectTransform default (0.5, 0.5) in
    // build_combined. (Earlier design defaulted to the sprite's tps
    // pivotPoint; reverted because GO RectTransform.pivot and sprite
    // tps pivotPoint are semantically distinct — conflating them broke
    // CSA prefabs that set a non-centered RectTransform.pivot for
    // size-fitted methods. See PA_InfinitePencil_Clock for the
    // asymmetric-mirror pattern that relies on this distinction.)
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
fn ui_icon_vs_ui_slice_method_dispatch_pinned_by_size_presence() {
    // The schema doesn't carry a UIIcon-vs-UISlice flag — both produce
    // `type: "sprite"` with `method: "MX/MY/MXY"`. The runtime
    // distinguishes them by the `size` field:
    //   - size: None → icon_mirror (matches UIIcon, native scale)
    //   - size: Some → slice_mirror (matches UISlice, stretch-to-rect)
    //
    // This test pins the dispatch so the migrator's invariant — UIIcon
    // leaves never emit `size`, UISlice leaves always do — translates to
    // the expected runtime path. A regression that auto-defaulted size
    // for size==None sprite leaves with MX/MY/MXY would silently flip
    // every UIIcon leaf into the slice path, breaking the 50+ UIIcon
    // combined sprites in meow-tower.
    use unity_sprite_author::combine::{self, AtlasSize};
    use unity_sprite_author::fab::{Affine, Combined, Method, Part};
    use unity_sprite_author::tpsheet::{Geometry, Pivot, Rect, SpriteAlignment, SpriteEntry, Vertex};

    let entry = SpriteEntry {
        name: "synthetic".into(),
        rect: Rect { x: 0, y: 0, w: 100, h: 100 },
        border: Default::default(),
        pivot: Pivot { x: 0.5, y: 0.5 },
        alignment: SpriteAlignment::Center,
        geometry: Geometry {
            vertices: vec![
                Vertex { x: 0.0,   y: 0.0   },
                Vertex { x: 100.0, y: 0.0   },
                Vertex { x: 100.0, y: 100.0 },
                Vertex { x: 0.0,   y: 100.0 },
            ],
            triangles: vec![0, 1, 2, 0, 2, 3],
        },
    };
    let entry_arc = std::sync::Arc::new(entry);

    // UIIcon path (size=None) — MXY produces 4 mirrored copies; AABB
    // spans 2× native on each axis around the sprite pivot.
    let combined_icon = Combined {
        name: "X".into(), pivot: [0.5, 0.5], border: [0.0; 4],
        parts: vec![Part::AtlasSprite {
            sprite: "synthetic".into(),
            method: Method::Mxy,
            size: None,                    // ← UIIcon: no size
            part_pivot: None,
            border_mult: 1.0,
            affine: Affine::default(),
            offset: [0.0, 0.0],
        }],
    };
    let resolve = {
        let e = entry_arc.clone();
        move |_: &str| Some(((*e).clone(), 1.0))
    };
    let m_icon = combine::build_combined(
        &combined_icon, resolve.clone(), AtlasSize { width: 128, height: 128 }, 100.0,
    ).unwrap();
    assert_eq!(m_icon.verts.len(), 16, "icon_mirror MXY = 4 copies × 4 verts");

    // UISlice path (size=Some) — slice_mirror stretches the source rect to
    // fit the target size; vert count depends on slice topology (4 corners
    // × 4 quadrants = 16 for a simple MXY slice).
    let combined_slice = Combined {
        name: "X".into(), pivot: [0.5, 0.5], border: [0.0; 4],
        parts: vec![Part::AtlasSprite {
            sprite: "synthetic".into(),
            method: Method::Mxy,
            size: Some((2.0, 2.0)),        // ← UISlice: explicit target rect
            part_pivot: None,
            border_mult: 1.0,
            affine: Affine::default(),
            offset: [0.0, 0.0],
        }],
    };
    let m_slice = combine::build_combined(
        &combined_slice, resolve, AtlasSize { width: 128, height: 128 }, 100.0,
    ).unwrap();
    // Vert layouts diverge — a single number can't pin slice topology
    // robustly without a fixture, but presence of a non-zero output proves
    // the path was taken. The critical pin is that the two paths produce
    // DIFFERENT vertex layouts, which we assert by comparing vert counts:
    // a regression collapsing both paths into one would equalize them.
    assert!(
        m_icon.verts != m_slice.verts,
        "UIIcon (icon_mirror) and UISlice (slice_mirror) must produce \
         different meshes for the same sprite + method — the size field is \
         the path discriminator. If they're equal, the dispatch collapsed."
    );
}

#[test]
fn build_combined_resolves_missing_part_pivot_to_centered_default() {
    // End-to-end: a Part with `part_pivot: None` and a slice method
    // (R3C3 requires part_pivot for target-rect anchoring) renders the
    // target rect centered around the leaf origin — proving the default
    // is (0.5, 0.5), NOT the sprite's tps pivotPoint (which is (0, 1)
    // for this fixture). A regression to "default = tps pivot" would
    // shift the rect to a corner-anchored layout and the test would
    // catch it via the AABB centre being far from the leaf origin.
    use unity_sprite_author::combine::{self, AtlasSize};
    use unity_sprite_author::fab::{Affine, Combined, Method, Part};
    use unity_sprite_author::tpsheet::{Geometry, Pivot, Rect, SpriteAlignment, SpriteEntry, Vertex};

    // Synthetic sprite with non-centered tps pivot (0, 1) — top-left corner.
    let entry = SpriteEntry {
        name: "synthetic".into(),
        rect: Rect { x: 0, y: 0, w: 100, h: 100 },
        border: Default::default(),
        pivot: Pivot { x: 0.0, y: 1.0 },
        alignment: SpriteAlignment::TopLeft,
        geometry: Geometry {
            vertices: vec![
                Vertex { x: 0.0,   y: 0.0   },
                Vertex { x: 100.0, y: 0.0   },
                Vertex { x: 100.0, y: 100.0 },
                Vertex { x: 0.0,   y: 100.0 },
            ],
            triangles: vec![0, 1, 2, 0, 2, 3],
        },
    };
    let combined = Combined {
        name: "X".into(),
        pivot: [0.5, 0.5],
        border: [0.0; 4],
        parts: vec![Part::AtlasSprite {
            sprite: "synthetic".into(),
            method: Method::R3c3,
            size: Some((1.0, 1.0)),       // 100 canvas px × 0.01 = 1.0 world units
            part_pivot: None,             // ← KEY: default should resolve to (0.5, 0.5)
            border_mult: 1.0,
            affine: Affine::default(),
            offset: [0.0, 0.0],
        }],
    };

    // sprite_bound = (100/100, 100/100) = (1, 1) world units; sized to match
    // target (1, 1) → scale = 1, no resize. Centered around (0, 0) means verts
    // span (-0.5, 0.5) on both axes. A tps-pivot-default regression would
    // shift to (-1.0, 0.0)..(0.0, 1.0) (anchored top-left).
    let entry = std::sync::Arc::new(entry);
    let m = combine::build_combined(
        &combined,
        |_| Some(((*entry).clone(), 1.0)),
        AtlasSize { width: 128, height: 128 },
        100.0,
    )
    .unwrap();
    // R3c3 with border=0 collapses to a 1-quad strip; assert AABB is centered.
    let (min_x, max_x, min_y, max_y) = m.verts.iter().fold(
        (f32::MAX, f32::MIN, f32::MAX, f32::MIN),
        |(mnx, mxx, mny, mxy), v| (mnx.min(v[0]), mxx.max(v[0]), mny.min(v[1]), mxy.max(v[1])),
    );
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;
    assert!(
        cx.abs() < 1e-5 && cy.abs() < 1e-5,
        "centered default → AABB centred at origin; got ({cx}, {cy}) from x:[{min_x}, {max_x}] y:[{min_y}, {max_y}]. \
         A regression to tps-default would shift the centre by ~(±target/2, ±target/2).",
    );
}

#[test]
fn icon_mirror_mesh_is_independent_of_part_pivot() {
    // UIIconMeshGen.MX/MY/MXY hardcodes the mirror axis at the sprite's
    // natural (tps) pivot — RectTransform.pivot doesn't enter the math.
    // This pins that property so any future "harmonize defaults" refactor
    // that auto-threads rect_pivot through every mesh-gen path can't
    // silently shift the 80+ UIIcon leaves in the meow-tower corpus that
    // carry a non-default RectTransform.pivot.
    //
    // Test shape: build the same MX leaf twice with different part_pivot
    // values (default-None vs. explicit-tps) and assert vertex equality.
    use unity_sprite_author::combine::{self, AtlasSize};
    use unity_sprite_author::fab::{Affine, Combined, Method, Part};
    use unity_sprite_author::tpsheet::{Geometry, Pivot, Rect, SpriteAlignment, SpriteEntry, Vertex};

    // Non-centered tps pivot at (0, 1) — top-left. If icon_mirror ever
    // consumed rect_pivot, swapping None ↔ Some([0, 1]) would shift verts.
    let entry = SpriteEntry {
        name: "synthetic".into(),
        rect: Rect { x: 0, y: 0, w: 100, h: 100 },
        border: Default::default(),
        pivot: Pivot { x: 0.0, y: 1.0 },
        alignment: SpriteAlignment::TopLeft,
        geometry: Geometry {
            vertices: vec![
                Vertex { x: 0.0,   y: 0.0   },
                Vertex { x: 100.0, y: 0.0   },
                Vertex { x: 100.0, y: 100.0 },
                Vertex { x: 0.0,   y: 100.0 },
            ],
            triangles: vec![0, 1, 2, 0, 2, 3],
        },
    };
    let entry_arc = std::sync::Arc::new(entry);
    let resolve = {
        let e = entry_arc.clone();
        move |_: &str| Some(((*e).clone(), 1.0))
    };

    let make = |part_pivot: Option<[f32; 2]>| Combined {
        name: "X".into(), pivot: [0.5, 0.5], border: [0.0; 4],
        parts: vec![Part::AtlasSprite {
            sprite: "synthetic".into(),
            method: Method::Mx,
            size: None,              // ← UIIcon: no size
            part_pivot,
            border_mult: 1.0,
            affine: Affine::default(),
            offset: [0.0, 0.0],
        }],
    };

    let m_default = combine::build_combined(
        &make(None), resolve.clone(), AtlasSize { width: 128, height: 128 }, 100.0,
    ).unwrap();
    let m_explicit = combine::build_combined(
        &make(Some([0.0, 1.0])), resolve, AtlasSize { width: 128, height: 128 }, 100.0,
    ).unwrap();

    assert_eq!(
        m_default.verts, m_explicit.verts,
        "icon_mirror (MX/MY/MXY, size=None) must be independent of part_pivot — \
         UIIconMeshGen hardcodes the mirror axis at the sprite's tps pivot. \
         If verts diverge, the icon_mirror path is now consuming rect_pivot, \
         which silently shifts 80+ UIIcon leaves with non-default RectTransform.pivot."
    );
}

#[test]
fn sprite_leaf_explicit_pivot_overrides_centered_default() {
    // Explicit `pivot` survives as Some(_), overriding the runtime's
    // (0.5, 0.5) default — required for size-fitted leaves that
    // intentionally diverge from center (e.g. asymmetric-pivot mirror).
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
