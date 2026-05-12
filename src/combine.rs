// Geometry builder for fabricated combined sprites. Walks a manifest's
// `Combined.parts` in declared order and produces a single (verts, uvs, tris)
// triple that downstream emit::SpriteAsset consumes.
//
// Phase 2 ships the polygon path only. Atlas-sprite parts + slice methods
// arrive in later phases (see docs/fab.md).

use crate::fab::{Affine, Part};
use crate::tpsheet::Rect;
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

pub struct AtlasSize {
    pub width: u32,
    pub height: u32,
}

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

/// Helper that dispatches over `Part` and returns the part's mesh. Phase 2
/// handles only the polygon variant; atlas-sprite parts panic. Phase 3 fills
/// in the atlas-sprite branch.
pub fn part_mesh(part: &Part, polygon_sprite_rect: Rect, atlas: AtlasSize) -> PartMesh {
    match part {
        Part::Polygon { vertices, affine, .. } => {
            polygon_mesh(vertices, *affine, polygon_sprite_rect, atlas)
        }
        Part::AtlasSprite { .. } => {
            unimplemented!("atlas-sprite parts land in phase 3 (see docs/fab.md)")
        }
    }
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
