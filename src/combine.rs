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

fn is_method_supported(m: Method) -> bool {
    matches!(m, Method::Id | Method::Mx | Method::My | Method::Mxy)
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

/// Build the mesh for a single atlas-sprite part under the given method.
///
/// Source verts in the tpsheet entry are atlas-pixel, sprite-rect-relative
/// (px ∈ [0, w]). They get converted to the part's local frame (pivot-relative
/// world units) via `(px - w·pivotX, py - h·pivotY) / ppu`, then the
/// per-method slice/mirror math transforms into the target-rect frame, and
/// finally the per-part affine is applied.
///
/// Unimplemented methods panic — the dispatcher (`build_combined`) is the
/// only place that decides supported-method policy.
pub fn atlas_sprite_mesh(
    entry: &SpriteEntry,
    method: Method,
    size: Option<(f32, f32)>,
    part_pivot: [f32; 2],
    affine: Affine,
    atlas: AtlasSize,
    ppu: f32,
) -> PartMesh {
    let src = SrcMesh {
        verts: local_src_verts(entry, ppu),
        uvs: atlas_uvs(entry, atlas),
        tris: entry.geometry.triangles.clone(),
    };
    let ctx = SliceCtx {
        sprite_pivot_norm: (entry.pivot.x, entry.pivot.y),
        sprite_bound_size: (entry.rect.w as f32 / ppu, entry.rect.h as f32 / ppu),
        part_pivot: (part_pivot[0], part_pivot[1]),
        affine,
    };
    match method {
        Method::Id => {
            let verts = src.verts.iter().map(|v| apply_affine(*v, affine)).collect();
            PartMesh { verts, uvs: src.uvs, tris: src.tris }
        }
        // MX/MY/MXY: with size → slice-fitted (UISliceMeshGen).
        //            without size → native-scale duplicate (UIIconMeshGen).
        Method::Mx  => match size {
            Some(sz) => slice_mirror(&src, &ctx, sz, MirrorAxis::X),
            None     => icon_mirror(&src, &ctx, MirrorAxis::X),
        },
        Method::My  => match size {
            Some(sz) => slice_mirror(&src, &ctx, sz, MirrorAxis::Y),
            None     => icon_mirror(&src, &ctx, MirrorAxis::Y),
        },
        Method::Mxy => match size {
            Some(sz) => slice_mirror(&src, &ctx, sz, MirrorAxis::Xy),
            None     => icon_mirror(&src, &ctx, MirrorAxis::Xy),
        },
        _ => panic!("method {method} not implemented"),
    }
}

struct SrcMesh {
    verts: Vec<[f32; 2]>,
    uvs: Vec<[f32; 2]>,
    tris: Vec<u16>,
}

struct SliceCtx {
    sprite_pivot_norm: (f32, f32),
    sprite_bound_size: (f32, f32),
    part_pivot: (f32, f32),
    affine: Affine,
}

// (px, py) → pivot-relative world units (matches Unity's Sprite.vertices).
fn local_src_verts(entry: &SpriteEntry, ppu: f32) -> Vec<[f32; 2]> {
    let pw = entry.rect.w as f32;
    let ph = entry.rect.h as f32;
    let pivot_px = (pw * entry.pivot.x, ph * entry.pivot.y);
    entry.geometry.vertices.iter()
        .map(|v| [(v.x - pivot_px.0) / ppu, (v.y - pivot_px.1) / ppu])
        .collect()
}

fn atlas_uvs(entry: &SpriteEntry, atlas: AtlasSize) -> Vec<[f32; 2]> {
    let aw = atlas.width as f32;
    let ah = atlas.height as f32;
    let rx = entry.rect.x as f32;
    let ry = entry.rect.y as f32;
    entry.geometry.vertices.iter()
        .map(|v| [(rx + v.x) / aw, (ry + v.y) / ah])
        .collect()
}

// --- slice-translation primitive --------------------------------------------
// Direct port of UISliceMeshGen.GetSliceVertexTranslation. `target_size` is
// the part's declared (width, height) in world units; `rect_pivot` is the
// target rect's pivot (hardcoded (0.5, 0.5) in v1 — see docs/fab.md).

struct SliceXform { scale: (f32, f32), translation: (f32, f32), offset: (f32, f32) }

fn slice_vertex_translation(
    target_size: (f32, f32),
    rect_pivot: (f32, f32),
    sprite_pivot_norm: (f32, f32),
    sprite_bound_size: (f32, f32),
    slice: (f32, f32),
    mirror_pivot: (f32, f32),
) -> SliceXform {
    let slice_size = (target_size.0 * slice.0, target_size.1 * slice.1);
    let scale = (slice_size.0 / sprite_bound_size.0, slice_size.1 / sprite_bound_size.1);
    let translation = (sprite_pivot_norm.0 * slice_size.0, sprite_pivot_norm.1 * slice_size.1);
    let offset = (
        (mirror_pivot.0 - rect_pivot.0) * target_size.0,
        (mirror_pivot.1 - rect_pivot.1) * target_size.1,
    );
    SliceXform { scale, translation, offset }
}

enum MirrorAxis { X, Y, Xy }

// Native-scale mirror duplication, matching UIIconMeshGen.{MX, MY, MXY}.
// No target rect: each src vert is taken at its native pivot-relative
// world-unit position, mirrored about the origin, and the affine is applied.
// Layout = [copy0, copy1, ...] same as slice_mirror.
fn icon_mirror(src: &SrcMesh, ctx: &SliceCtx, axis: MirrorAxis) -> PartMesh {
    let signs: &[(f32, f32)] = match axis {
        MirrorAxis::X  => &[(1.0, 1.0), (-1.0, 1.0)],
        MirrorAxis::Y  => &[(1.0, 1.0), (1.0, -1.0)],
        MirrorAxis::Xy => &[(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)],
    };
    let copies = signs.len();
    let n = src.verts.len();

    let mut verts = Vec::with_capacity(n * copies);
    for (sx, sy) in signs {
        for v in &src.verts {
            verts.push(apply_affine([v[0] * sx, v[1] * sy], ctx.affine));
        }
    }

    let mut uvs = Vec::with_capacity(n * copies);
    for _ in 0..copies {
        uvs.extend_from_slice(&src.uvs);
    }

    let mut tris = Vec::with_capacity(src.tris.len() * copies);
    for c in 0..copies {
        let off = (c * n) as u16;
        tris.extend(src.tris.iter().map(|i| i + off));
    }

    PartMesh { verts, uvs, tris }
}

// MX / MY / MXY: place 2 or 4 mirrored copies of src into the target rect.
// Source layout (in target-rect frame) per axis:
//   MX  : 2 copies [X+, X-], slice = (0.5, 1)  , mirror_pivot = (0.5, 0)
//   MY  : 2 copies [Y+, Y-], slice = (1, 0.5)  , mirror_pivot = (0, 0.5)
//   MXY : 4 copies clockwise from X+Y+,
//         slice = (0.5, 0.5), mirror_pivot = (0.5, 0.5)
fn slice_mirror(src: &SrcMesh, ctx: &SliceCtx, target_size: (f32, f32), axis: MirrorAxis) -> PartMesh {
    let (slice, mirror_pivot, signs) = match axis {
        MirrorAxis::X  => ((0.5, 1.0), (0.5, 0.0), &[(1.0, 1.0), (-1.0, 1.0)][..]),
        MirrorAxis::Y  => ((1.0, 0.5), (0.0, 0.5), &[(1.0, 1.0), (1.0, -1.0)][..]),
        MirrorAxis::Xy => ((0.5, 0.5), (0.5, 0.5),
                           &[(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)][..]),
    };
    let x = slice_vertex_translation(
        target_size, ctx.part_pivot, ctx.sprite_pivot_norm, ctx.sprite_bound_size,
        slice, mirror_pivot,
    );

    let copies = signs.len();
    let n = src.verts.len();

    // Translate each src vert into the slice's frame once.
    let base: Vec<(f32, f32)> = src.verts.iter()
        .map(|v| (v[0] * x.scale.0 + x.translation.0, v[1] * x.scale.1 + x.translation.1))
        .collect();

    // Layout = [copy0_verts..., copy1_verts..., ...]; matches C# index math
    // in UISliceMeshGen.{MX,MY,MXY}.
    let mut verts = Vec::with_capacity(n * copies);
    for (sx, sy) in signs {
        for b in &base {
            let p = [b.0 * sx + x.offset.0, b.1 * sy + x.offset.1];
            verts.push(apply_affine(p, ctx.affine));
        }
    }

    // UVs repeated verbatim per copy (UV mirroring = the same texel is
    // sampled from each copy's geometrically-mirrored verts).
    let mut uvs = Vec::with_capacity(n * copies);
    for _ in 0..copies {
        uvs.extend_from_slice(&src.uvs);
    }

    // Tris duplicated, offset by n per copy.
    let mut tris = Vec::with_capacity(src.tris.len() * copies);
    for c in 0..copies {
        let off = (c * n) as u16;
        tris.extend(src.tris.iter().map(|i| i + off));
    }

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
            Part::AtlasSprite { method, size, part_pivot, affine, .. } => {
                if !is_method_supported(*method) {
                    return Err(CombineError::MethodUnimplemented {
                        combined: combined.name.clone(),
                        sprite: source_name.clone(),
                        method: *method,
                    });
                }
                atlas_sprite_mesh(&entry, *method, *size, *part_pivot, *affine, atlas, ppu)
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
            &entry, Method::Id, None, [0.5, 0.5], Affine::default(),
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
            &entry, Method::Id, None, [0.5, 0.5],
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

    // --- mirror methods (phase 4) ---

    #[test]
    fn mx_doubles_verts_and_indices() {
        // 2×2 sprite, PPU 1, pivot center. Target rect 4×2 → MX produces two
        // 2×2 halves sitting side-by-side, target rect centered at origin.
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mx, Some((4.0, 2.0)), [0.5, 0.5], Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0,
        );
        assert_eq!(m.verts.len(), 8, "2× source verts");
        assert_eq!(m.tris.len(), 12, "2× indices");
        assert_eq!(m.uvs.len(), 8);
        // Second copy's tris are offset by 4.
        assert_eq!(m.tris[..6], [0, 2, 1, 1, 2, 3]);
        assert_eq!(m.tris[6..], [4, 6, 5, 5, 6, 7]);
    }

    #[test]
    fn mx_first_copy_in_positive_x_half_second_in_negative_x_half() {
        // Symmetric square sprite (verts at corners of [0,2]×[0,2], pivot
        // center). Target 4×2. The first copy should land at x ∈ [0, 2],
        // second copy at x ∈ [-2, 0]. Both at y ∈ [-1, 1].
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mx, Some((4.0, 2.0)), [0.5, 0.5], Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0,
        );
        // First copy bottom-left: src (0,0) → pivot-rel (-1,-1) → scale (1,1) → translate (1,1) → slice (0,0) → offset (0,-1) → (0,-1).
        assert_eq!(m.verts[0], [0.0, -1.0]);
        // First copy top-right: src (2,2) → pivot-rel (1,1) → scale (1,1) → translate (1,1) → slice (2,2) → offset (0,-1) → (2,1).
        assert_eq!(m.verts[3], [2.0, 1.0]);
        // Second copy mirrors X around slice origin (0): (0,-1) → (0,-1), (2,1) → (-2,1).
        assert_eq!(m.verts[4], [0.0, -1.0]);
        assert_eq!(m.verts[7], [-2.0, 1.0]);
    }

    #[test]
    fn mx_uvs_repeated_verbatim() {
        let entry = quad_entry(10, 20, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mx, Some((4.0, 2.0)), [0.5, 0.5], Affine::default(),
            AtlasSize { width: 100, height: 100 }, 1.0,
        );
        assert_eq!(m.uvs[..4], m.uvs[4..]);
    }

    #[test]
    fn my_doubles_along_y() {
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::My, Some((2.0, 4.0)), [0.5, 0.5], Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0,
        );
        // First copy (Y+): src (0,0) → (-1,-1) * (1,1) + (1,1) = (0,0); + offset (-1,0) → (-1, 0). Wait, mirror_pivot = (0, 0.5), rect_pivot = (0.5, 0.5).
        // offset = (0 - 0.5, 0.5 - 0.5) * (2, 4) = (-1, 0).
        // src bottom-left (0,0) → (0,0)+offset = (-1, 0).
        assert_eq!(m.verts[0], [-1.0, 0.0]);
        // src top-right (2,2) → slice (2, 2) → +offset → (1, 2).
        assert_eq!(m.verts[3], [1.0, 2.0]);
        // Second copy mirrors Y around slice origin: (1, 2) → (1, -2) → +offset (-1, 0)? No, that's wrong logic. Let me re-derive.
        // After translate, slice frame: src (0,0) → (0,0); src(2,2) → (2,2). Then v.y = -v.y for second copy: (0,0) → (0,0); (2,2) → (2,-2).
        // Then +offset (-1, 0): first copy (0,0) → (-1,0); first copy (2,2) → (1,2); second copy (0,0) → (-1,0); second copy (2,-2) → (1,-2).
        assert_eq!(m.verts[4], [-1.0, 0.0]);
        assert_eq!(m.verts[7], [1.0, -2.0]);
    }

    #[test]
    fn mxy_quadruples_verts() {
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mxy, Some((4.0, 4.0)), [0.5, 0.5], Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0,
        );
        assert_eq!(m.verts.len(), 16, "4× source");
        assert_eq!(m.tris.len(), 24);
        // Target 4×4, sprite 2×2 PPU 1. slice = sliceSize = (2, 2), scale = (1, 1),
        // translation = (1, 1), offset = (0, 0).
        // src (2,2) → pivot-rel (1,1) → scale (1,1) → translate (2, 2) → offset (0,0) → (2, 2).
        // Copies sign-multiply X, Y around slice origin (then add offset):
        //   Copy0 (1,1)→(2,2), Copy1 (-1,1)→(-2,2),
        //   Copy2 (-1,-1)→(-2,-2), Copy3 (1,-1)→(2,-2).
        assert_eq!(m.verts[3],  [2.0, 2.0]);
        assert_eq!(m.verts[7],  [-2.0, 2.0]);
        assert_eq!(m.verts[11], [-2.0, -2.0]);
        assert_eq!(m.verts[15], [2.0, -2.0]);
    }

    #[test]
    fn icon_mx_duplicates_with_native_origin_mirror() {
        // 2×2 sprite at pivot (0.5, 0.5), PPU 1, no target size.
        // Source verts (pivot-relative): (-1,-1), (1,-1), (-1,1), (1,1).
        // First copy: same. Second copy: X-flipped about origin (0,0).
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mx, None, [0.5, 0.5], Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0,
        );
        assert_eq!(m.verts.len(), 8);
        assert_eq!(m.tris.len(), 12);
        // First copy is the native sprite verts.
        assert_eq!(m.verts[..4], [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]]);
        // Second copy mirrors X about (0,0).
        assert_eq!(m.verts[4..], [[1.0, -1.0], [-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]);
        // UVs repeated verbatim.
        assert_eq!(m.uvs[..4], m.uvs[4..]);
    }

    #[test]
    fn icon_my_mirrors_y_axis() {
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::My, None, [0.5, 0.5], Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0,
        );
        assert_eq!(m.verts.len(), 8);
        assert_eq!(m.verts[4..], [[-1.0, 1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]]);
    }

    #[test]
    fn icon_mxy_quadruples_about_origin() {
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mxy, None, [0.5, 0.5], Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0,
        );
        assert_eq!(m.verts.len(), 16);
        assert_eq!(m.tris.len(), 24);
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
            part_pivot: [0.5, 0.5],
            border_mult: 1.0,
            affine,
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
        // Use a method that hasn't landed yet (Tx is phase 5).
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(),
            method: Method::Tx,
            size: Some((4.0, 1.0)),
            part_pivot: [0.5, 0.5],
            border_mult: 1.0,
            affine: Affine::default(),
        }]);
        let a = quad_entry(0, 0, 1, 1, (0.5, 0.5));
        let err = build_combined(
            &combined,
            |_| Some(a.clone()),
            AtlasSize { width: 16, height: 16 },
            1.0,
        ).unwrap_err();
        assert!(matches!(err, CombineError::MethodUnimplemented { method: Method::Tx, .. }), "{err:?}");
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
