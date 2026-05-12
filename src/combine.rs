// Geometry builder for fabricated combined sprites. Walks a manifest's
// `Combined.parts` in declared order and produces a single (verts, uvs, tris)
// triple that downstream emit::SpriteAsset consumes.
//
// Phase 2 ships the polygon path only. Atlas-sprite parts + slice methods
// arrive in later phases (see docs/fab.md).

use std::fmt;

use crate::fab::{self, Affine, Method, Part};
use crate::tpsheet::{Rect, SpriteEntry};
use crate::triangulator;

#[derive(Debug, Clone, PartialEq)]
pub struct PartMesh {
    // Verts in world units (post-PPU, post-affine).
    pub verts: Vec<[f32; 2]>,
    // UVs in atlas-normalized space (0..1 on each axis).
    pub uvs: Vec<[f32; 2]>,
    // Index buffer, u16, CCW triangles.
    pub tris: Vec<u16>,
}

#[derive(Debug, Clone)]
pub struct CombinedMesh {
    pub verts: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub tris: Vec<u16>,
    // AABB on the atlas in pixels — the union of every part's atlas rect.
    // Used as the combined Sprite's m_Rect.
    pub atlas_rect: Rect,
}

#[derive(Debug, Clone, Copy)]
pub struct AtlasSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub enum CombineError {
    SpriteNotFound { combined: String, sprite: String },
    MethodUnimplemented { combined: String, sprite: String, method: Method },
}

impl fmt::Display for CombineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpriteNotFound { combined, sprite } => write!(
                f, "combined {combined:?}: sprite {sprite:?} not found in tpsheet",
            ),
            Self::MethodUnimplemented { combined, sprite, method } => write!(
                f, "combined {combined:?} part {sprite:?}: method {method} not implemented yet",
            ),
        }
    }
}

impl std::error::Error for CombineError {}

/// Build the mesh for a single polygon part.
///
/// - `vertices` come from the manifest in world units. The polygon is
///   triangulated via ear-clip (auto-handles winding).
/// - All UVs sample the center pixel of `polygon_sprite_rect`, normalized
///   against the atlas size — matches `SolidUVCache.Get` in meow-tower.
/// - The affine `T · R · S` is applied to each vert *after* triangulation.
///
/// Triangulator output preserves the relative ordering of input verts; the
/// crate emits the polygon's own verts (not a re-triangulated subset), so the
/// returned `verts.len() == vertices.len()`.
pub fn polygon_mesh(
    vertices: &[[f32; 2]],
    affine: Affine,
    polygon_sprite_rect: Rect,
    atlas: AtlasSize,
) -> PartMesh {
    let tris = triangulator::triangulate(vertices);

    let verts: Vec<[f32; 2]> = vertices.iter().map(|v| apply_affine(*v, affine)).collect();

    let cx = (polygon_sprite_rect.x as f32 + polygon_sprite_rect.w as f32 * 0.5) / atlas.width as f32;
    let cy = (polygon_sprite_rect.y as f32 + polygon_sprite_rect.h as f32 * 0.5) / atlas.height as f32;
    let uvs: Vec<[f32; 2]> = vec![[cx, cy]; vertices.len()];

    PartMesh { verts, uvs, tris }
}

/// Build the mesh for a single atlas-sprite part under method `ID`.
///
/// Source verts in the tpsheet entry are atlas-pixel, sprite-rect-relative
/// (px ∈ [0, w]). Convert each to the part's local frame via
/// `(px - w·pivotX, py - h·pivotY) / ppu`, then apply the affine.
/// UVs are atlas-normalized from the sprite's atlas rect + the pixel verts.
///
/// Non-ID methods (MX/MY/MXY/slice/tile) arrive in later phases; this
/// function panics on them so the dispatcher (`build_combined`) is the only
/// place that decides supported-method policy.
pub fn atlas_sprite_mesh(
    entry: &SpriteEntry,
    method: Method,
    affine: Affine,
    atlas: AtlasSize,
    ppu: f32,
) -> PartMesh {
    assert!(matches!(method, Method::Id), "phase 3 supports method Id only");

    let pw = entry.rect.w as f32;
    let ph = entry.rect.h as f32;
    let pivot_px = (pw * entry.pivot.x, ph * entry.pivot.y);
    let aw = atlas.width as f32;
    let ah = atlas.height as f32;
    let rx = entry.rect.x as f32;
    let ry = entry.rect.y as f32;

    let verts: Vec<[f32; 2]> = entry
        .geometry
        .vertices
        .iter()
        .map(|v| {
            let local = [(v.x - pivot_px.0) / ppu, (v.y - pivot_px.1) / ppu];
            apply_affine(local, affine)
        })
        .collect();
    let uvs: Vec<[f32; 2]> = entry
        .geometry
        .vertices
        .iter()
        .map(|v| [(rx + v.x) / aw, (ry + v.y) / ah])
        .collect();
    let tris = entry.geometry.triangles.clone();

    PartMesh { verts, uvs, tris }
}

/// Walk every part of a combined entry, in declared order, and return the
/// merged mesh + atlas-rect AABB. Order is preserved verbatim — the
/// resulting `tris` runs back-to-front by part.
pub fn build_combined<F>(
    combined: &fab::Combined,
    mut resolve: F,
    atlas: AtlasSize,
    ppu: f32,
) -> Result<CombinedMesh, CombineError>
where
    F: FnMut(&str) -> Option<SpriteEntry>,
{
    let mut all_verts: Vec<[f32; 2]> = Vec::new();
    let mut all_uvs: Vec<[f32; 2]> = Vec::new();
    let mut all_tris: Vec<u16> = Vec::new();
    let mut aabb: Option<(u32, u32, u32, u32)> = None; // (minx, miny, maxx, maxy)

    for part in &combined.parts {
        let source_name = match part {
            Part::AtlasSprite { sprite, .. } => sprite,
            Part::Polygon { polygon_sprite, .. } => polygon_sprite,
        };
        let entry = resolve(source_name).ok_or_else(|| CombineError::SpriteNotFound {
            combined: combined.name.clone(),
            sprite: source_name.clone(),
        })?;

        let r = entry.rect;
        aabb = Some(match aabb {
            None => (r.x, r.y, r.x + r.w, r.y + r.h),
            Some((minx, miny, maxx, maxy)) => (
                minx.min(r.x), miny.min(r.y),
                maxx.max(r.x + r.w), maxy.max(r.y + r.h),
            ),
        });

        let part_mesh = match part {
            Part::AtlasSprite { method, affine, .. } => {
                if !matches!(method, Method::Id) {
                    return Err(CombineError::MethodUnimplemented {
                        combined: combined.name.clone(),
                        sprite: source_name.clone(),
                        method: *method,
                    });
                }
                atlas_sprite_mesh(&entry, *method, *affine, atlas, ppu)
            }
            Part::Polygon { vertices, affine, .. } => {
                polygon_mesh(vertices, *affine, entry.rect, atlas)
            }
        };

        let base = all_verts.len() as u16;
        all_verts.extend(part_mesh.verts);
        all_uvs.extend(part_mesh.uvs);
        all_tris.extend(part_mesh.tris.iter().map(|i| i + base));
    }

    let (minx, miny, maxx, maxy) = aabb.expect("fab::Combined.parts is non-empty (parse-time)");
    Ok(CombinedMesh {
        verts: all_verts,
        uvs: all_uvs,
        tris: all_tris,
        atlas_rect: Rect { x: minx, y: miny, w: maxx - minx, h: maxy - miny },
    })
}

fn apply_affine(v: [f32; 2], a: Affine) -> [f32; 2] {
    // T · R · S applied to v: scale → rotate → translate.
    let sx = v[0] * a.sx;
    let sy = v[1] * a.sy;
    let (rs, rc) = a.rot_deg.to_radians().sin_cos();
    let rx = sx * rc - sy * rs;
    let ry = sx * rs + sy * rc;
    [rx + a.tx, ry + a.ty]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATLAS: AtlasSize = AtlasSize { width: 1024, height: 1024 };

    fn rect(x: u32, y: u32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn polygon_quad_produces_two_triangles() {
        let verts = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
        let m = polygon_mesh(&verts, Affine::default(), rect(0, 0, 2, 2), ATLAS);
        assert_eq!(m.tris.len(), 6);
        assert_eq!(m.verts.len(), 4);
        assert_eq!(m.uvs.len(), 4);
    }

    #[test]
    fn polygon_uvs_all_sample_atlas_center_pixel() {
        // Atlas rect = (100, 200, 4, 6). Center pixel = (102, 203).
        // Normalized against 1024 = (102/1024, 203/1024).
        let verts = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let m = polygon_mesh(&verts, Affine::default(), rect(100, 200, 4, 6), ATLAS);
        let want = [102.0 / 1024.0, 203.0 / 1024.0];
        for uv in &m.uvs {
            assert_eq!(uv, &want);
        }
    }

    #[test]
    fn polygon_identity_affine_is_passthrough() {
        let verts = [[0.0, 0.0], [3.0, 0.0], [0.0, 4.0]];
        let m = polygon_mesh(&verts, Affine::default(), rect(0, 0, 1, 1), ATLAS);
        for (out, src) in m.verts.iter().zip(verts.iter()) {
            assert!((out[0] - src[0]).abs() < 1e-6);
            assert!((out[1] - src[1]).abs() < 1e-6);
        }
    }

    #[test]
    fn polygon_translation_only() {
        let verts = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let a = Affine { tx: 10.0, ty: 20.0, ..Affine::default() };
        let m = polygon_mesh(&verts, a, rect(0, 0, 1, 1), ATLAS);
        assert_eq!(m.verts, vec![[10.0, 20.0], [11.0, 20.0], [10.0, 21.0]]);
    }

    #[test]
    fn polygon_scale_only() {
        let verts = [[1.0, 1.0], [2.0, 1.0], [1.0, 3.0]];
        let a = Affine { sx: 2.0, sy: 0.5, ..Affine::default() };
        let m = polygon_mesh(&verts, a, rect(0, 0, 1, 1), ATLAS);
        assert_eq!(m.verts, vec![[2.0, 0.5], [4.0, 0.5], [2.0, 1.5]]);
    }

    #[test]
    fn polygon_negative_scale_flips_axis() {
        let verts = [[1.0, 2.0], [3.0, 2.0], [1.0, 4.0]];
        let a = Affine { sx: -1.0, ..Affine::default() };
        let m = polygon_mesh(&verts, a, rect(0, 0, 1, 1), ATLAS);
        assert_eq!(m.verts, vec![[-1.0, 2.0], [-3.0, 2.0], [-1.0, 4.0]]);
    }

    #[test]
    fn polygon_rotation_90deg() {
        let verts = [[1.0, 0.0]];
        let a = Affine { rot_deg: 90.0, ..Affine::default() };
        let m = polygon_mesh_one(&verts[0], a);
        assert!((m[0] - 0.0).abs() < 1e-5, "{:?}", m);
        assert!((m[1] - 1.0).abs() < 1e-5, "{:?}", m);
    }

    fn polygon_mesh_one(v: &[f32; 2], a: Affine) -> [f32; 2] {
        apply_affine(*v, a)
    }

    // --- atlas-sprite ID method ---

    fn quad_entry(rx: u32, ry: u32, w: u32, h: u32, pivot: (f32, f32)) -> SpriteEntry {
        use crate::tpsheet::*;
        SpriteEntry {
            name: "Q".into(),
            rect: Rect { x: rx, y: ry, w, h },
            pivot: Pivot { x: pivot.0, y: pivot.1 },
            alignment: SpriteAlignment::Custom,
            border: Border::default(),
            geometry: Geometry {
                vertices: vec![
                    Vertex { x: 0.0, y: 0.0 },
                    Vertex { x: w as f32, y: 0.0 },
                    Vertex { x: 0.0, y: h as f32 },
                    Vertex { x: w as f32, y: h as f32 },
                ],
                triangles: vec![0, 2, 1, 1, 2, 3],
            },
        }
    }

    #[test]
    fn atlas_sprite_id_emits_pivot_relative_world_units() {
        // 2×4 sprite at atlas (10, 20), pivot (0.5, 0.5), PPU 100.
        // pixel pivot = (1, 2). local verts = (pixel − pivot)/PPU.
        let entry = quad_entry(10, 20, 2, 4, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Id, Affine::default(),
            AtlasSize { width: 100, height: 100 },
            100.0,
        );
        assert_eq!(m.verts, vec![
            [-0.01, -0.02], [0.01, -0.02], [-0.01, 0.02], [0.01, 0.02],
        ]);
        assert_eq!(m.uvs, vec![
            [0.10, 0.20], [0.12, 0.20], [0.10, 0.24], [0.12, 0.24],
        ]);
        assert_eq!(m.tris, vec![0, 2, 1, 1, 2, 3]);
    }

    #[test]
    fn atlas_sprite_id_with_translation() {
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Id,
            Affine { tx: 5.0, ty: 7.0, ..Affine::default() },
            AtlasSize { width: 16, height: 16 },
            1.0,
        );
        // pivot pixel = (1, 1). local verts pre-translate:
        //   (-1, -1), (1, -1), (-1, 1), (1, 1).
        // After translate:
        assert_eq!(m.verts, vec![
            [4.0, 6.0], [6.0, 6.0], [4.0, 8.0], [6.0, 8.0],
        ]);
    }

    // --- multi-part combine ---

    fn make_combined(name: &str, parts: Vec<Part>) -> fab::Combined {
        fab::Combined { name: name.into(), pivot: [0.5, 0.5], border: [0.0; 4], parts }
    }

    fn id_part(sprite: &str, affine: Affine) -> Part {
        Part::AtlasSprite {
            sprite: sprite.into(),
            method: Method::Id,
            size: None,
            border_mult: 1.0,
            affine,
            mirror_x: false,
            mirror_y: false,
        }
    }

    #[test]
    fn build_combined_concatenates_parts_with_index_offset() {
        let a = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let b = quad_entry(4, 4, 2, 2, (0.5, 0.5));
        let combined = make_combined("BX", vec![
            id_part("A", Affine::default()),
            id_part("B", Affine { tx: 10.0, ..Affine::default() }),
        ]);
        let m = build_combined(
            &combined,
            |n| match n { "A" => Some(a.clone()), "B" => Some(b.clone()), _ => None },
            AtlasSize { width: 16, height: 16 },
            1.0,
        ).unwrap();
        assert_eq!(m.verts.len(), 8, "4 + 4 verts");
        assert_eq!(m.tris.len(), 12, "6 + 6 indices");
        // Second part's indices offset by 4.
        assert!(m.tris[6..].iter().all(|&i| i >= 4));
        assert_eq!(m.tris[6..], [4, 6, 5, 5, 6, 7]);
    }

    #[test]
    fn build_combined_aabb_is_atlas_rect_union() {
        let a = quad_entry(10, 20, 4, 4, (0.5, 0.5));   // (10..14, 20..24)
        let b = quad_entry(50, 8, 6, 2, (0.5, 0.5));    // (50..56, 8..10)
        let combined = make_combined("BX", vec![
            id_part("A", Affine::default()),
            id_part("B", Affine::default()),
        ]);
        let m = build_combined(
            &combined,
            |n| match n { "A" => Some(a.clone()), "B" => Some(b.clone()), _ => None },
            AtlasSize { width: 64, height: 64 },
            1.0,
        ).unwrap();
        // Union: x ∈ [10, 56), y ∈ [8, 24) ⇒ rect (10, 8, 46, 16).
        assert_eq!(m.atlas_rect, Rect { x: 10, y: 8, w: 46, h: 16 });
    }

    #[test]
    fn build_combined_propagates_part_order() {
        let a = quad_entry(0, 0, 1, 1, (0.5, 0.5));
        let combined = make_combined("BX", vec![
            id_part("Z", Affine { tx: 100.0, ..Affine::default() }),
            id_part("Z", Affine::default()),
        ]);
        let m = build_combined(
            &combined,
            |_| Some(a.clone()),
            AtlasSize { width: 16, height: 16 },
            1.0,
        ).unwrap();
        // First part's verts get tx=100; second part's stay near origin.
        // Confirms parts are NOT deduped by source name.
        assert!(m.verts[0][0] > 50.0, "{:?}", m.verts);
        assert!(m.verts[4][0] < 5.0, "{:?}", m.verts);
    }

    #[test]
    fn build_combined_errors_on_unresolved_sprite() {
        let combined = make_combined("BX", vec![id_part("MISSING", Affine::default())]);
        let err = build_combined(
            &combined,
            |_| None,
            AtlasSize { width: 16, height: 16 },
            1.0,
        ).unwrap_err();
        assert!(matches!(err, CombineError::SpriteNotFound { ref sprite, .. } if sprite == "MISSING"), "{err:?}");
    }

    #[test]
    fn build_combined_errors_on_unimplemented_method() {
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(),
            method: Method::Mx,
            size: Some((1.0, 1.0)),
            border_mult: 1.0,
            affine: Affine::default(),
            mirror_x: false, mirror_y: false,
        }]);
        let a = quad_entry(0, 0, 1, 1, (0.5, 0.5));
        let err = build_combined(
            &combined,
            |_| Some(a.clone()),
            AtlasSize { width: 16, height: 16 },
            1.0,
        ).unwrap_err();
        assert!(matches!(err, CombineError::MethodUnimplemented { method: Method::Mx, .. }), "{err:?}");
    }

    #[test]
    fn polygon_combined_transform_order_is_t_after_rs() {
        // Order: scale, then rotate, then translate. Check that (1,0)
        // under (sx=2, rot=90, tx=10, ty=0) lands at (10, 2).
        //   scale  → (2, 0)
        //   rotate → (0, 2)
        //   trans  → (10, 2)
        let a = Affine { tx: 10.0, ty: 0.0, sx: 2.0, sy: 1.0, rot_deg: 90.0 };
        let out = apply_affine([1.0, 0.0], a);
        assert!((out[0] - 10.0).abs() < 1e-5, "{out:?}");
        assert!((out[1] - 2.0).abs() < 1e-5, "{out:?}");
    }
}
