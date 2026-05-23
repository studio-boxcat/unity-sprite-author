// Geometry builder for fabricated combined sprites. Walks a manifest's
// `Combined.parts` in declared order and produces a single (verts, uvs, tris)
// triple that downstream emit::SpriteAsset consumes.
//
// Supports both shapes from `docs/fab.md`: polygon parts (ear-clipped or
// with an explicit `triangles` override) and atlas-sprite parts under the
// full `Method` enum — `ID` / `MX` / `MY` / `MXY` / `TX` / `TY` / `TX_MC3`
// / the slice-grid family (`R*`, `MX_R*`, `MY_R*`, `MXY_R*`). Each part is
// then composed through `apply_transform` to reproduce Unity's
// `CanvasSpriteAuthor` matrix chain bit-for-bit (see the "Per-part
// transform" section in fab.md).

use std::fmt;

use crate::fab::{self, Affine, Method, Part};
use crate::tpsheet::{Rect, SpriteEntry};
use crate::triangulator;

/// Output of a single part-builder (`polygon_mesh` /
/// `atlas_sprite_mesh`) — the per-part `(verts, uvs, tris)` triple in a
/// frame ready for [`build_combined`] to splice into the combined buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct PartMesh {
    /// Verts in world units (post-PPU, post-affine / canvas chain).
    pub verts: Vec<[f32; 2]>,
    /// UVs in atlas-normalized space (0..1 on each axis).
    pub uvs: Vec<[f32; 2]>,
    /// Index buffer, `u16`, CCW triangles.
    pub tris: Vec<u16>,
}

/// Output of [`build_combined`]: the merged `(verts, uvs, tris)` across
/// every part of a `Combined` entry, plus the atlas-rect AABB used to
/// derive `m_Rect` of the fabricated Sprite.
#[derive(Debug, Clone)]
pub struct CombinedMesh {
    pub verts: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub tris: Vec<u16>,
    /// AABB on the atlas in pixels — the union of every part's atlas
    /// rect. Used as the combined Sprite's `m_Rect`.
    pub atlas_rect: Rect,
}

/// Atlas texture dimensions (pixels). Passed through the build path so
/// UV coordinates can be normalized against the texture's actual extent.
#[derive(Debug, Clone, Copy)]
pub struct AtlasSize {
    pub width: u32,
    pub height: u32,
}

/// Errors raised by [`build_combined`] — slice-method source-rect
/// constraint violations and missing sprite references. Each `Display`
/// impl names the combined entry + the offending part so the message
/// is self-contained.
#[derive(Debug)]
pub enum CombineError {
    SpriteNotFound { combined: String, sprite: String },
    SliceConstraint { combined: String, sprite: String, method: Method, reason: &'static str },
}

impl fmt::Display for CombineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpriteNotFound { combined, sprite } => write!(
                f, "combined {combined:?}: sprite {sprite:?} not found in tpsheet",
            ),
            Self::SliceConstraint { combined, sprite, method, reason } => write!(
                f, "combined {combined:?} part {sprite:?} (method {method}): {reason}",
            ),
        }
    }
}

impl std::error::Error for CombineError {}

/// Derived `m_Rect.{w, h}` (pixels, f32) and `m_Pivot` (normalized 0..1) for
/// a fabricated sprite, computed from the combined mesh's vertex AABB in
/// world units. Port of meow-tower's
/// `SpriteFactory.CalcRectAndPivot(vertices, ppu)`:
///
/// ```text
/// (x0, y0, x1, y1) = AABB(verts)
/// rect = (0, 0, (x1 - x0) * ppu, (y1 - y0) * ppu)
/// pivot = (-x0 / (x1 - x0), -y0 / (y1 - y0))
/// ```
///
/// Empty input is a programming error (the parser rejects empty `parts`).
pub fn calc_rect_and_pivot(verts: &[[f32; 2]], ppu: f32) -> ((f32, f32), (f32, f32)) {
    assert!(!verts.is_empty(), "calc_rect_and_pivot called with no verts");
    let mut x0 = f32::INFINITY;
    let mut y0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for v in verts {
        if v[0] < x0 { x0 = v[0]; }
        if v[0] > x1 { x1 = v[0]; }
        if v[1] < y0 { y0 = v[1]; }
        if v[1] > y1 { y1 = v[1]; }
    }
    let w = x1 - x0;
    let h = y1 - y0;
    // pivot = (x0 / (x0 - x1), y0 / (y0 - y1)) — but Mono/IL2CPP on ARM64
    // macOS evaluates each f32 arithmetic op through f64 intermediates and
    // rounds at the end (verified against CSA emit on PA_InfinitePencil_Clock:
    // pure-f32 gives 0x3EF82CFC, CSA gives 0x3EF82CFD which matches the
    // f64-throughout-then-cast path). Both the subtraction *and* the
    // division have to widen to f64 — folding the f32 subtraction first
    // and only widening the result for the divide reproduces the f32 bug.
    let x0d = x0 as f64;
    let x1d = x1 as f64;
    let y0d = y0 as f64;
    let y1d = y1 as f64;
    let pivot_x = (x0d / (x0d - x1d)) as f32;
    let pivot_y = (y0d / (y0d - y1d)) as f32;
    ((w * ppu, h * ppu), (pivot_x, pivot_y))
}

// Source-rect / border constraints asserted by each slice method in
// UISliceMeshGen.cs + Tiling.cs. The fab.rs parser has no tpsheet access,
// so these checks fire at build time instead — same `fail loud` outcome,
// surfaced through CombineError::SliceConstraint.
fn check_method_constraints(
    method: Method,
    entry: &SpriteEntry,
    combined: &str,
    sprite: &str,
) -> Result<(), CombineError> {
    let err = |reason| CombineError::SliceConstraint {
        combined: combined.to_string(),
        sprite: sprite.to_string(),
        method,
        reason,
    };
    // ID, MX/MY/MXY, TX, TY have no source-rect constraints. The clauses
    // below cover TX_MC3 (an asymmetric tiler) and the slice-grid family
    // (R1C3 / MX_R1C4 / MX_R3C2 / … MXY_R3C3_NF) — each method asserts
    // the border layout it expects.
    if matches!(method, Method::TxMc3) {
        if entry.border.left != 0 {
            return Err(err("left border must be 0 (TX_MC3 expects mirrored edges)"));
        }
        if entry.border.right <= 0 {
            return Err(err("right border must be > 0 (TX_MC3 needs edge width)"));
        }
    }
    if matches!(method, Method::R1c3) {
        // Per UISliceMeshGen.R1C3: borderSumX == sprite_rect_w. The centre
        // column collapses to a single U coordinate (UV-stretching a 0-width
        // atlas region).
        if entry.border.left + entry.border.right != entry.rect.w as i32 {
            return Err(err("R1C3 requires border.left + border.right == sprite.rect.width"));
        }
    }
    if matches!(method, Method::MxR1c4) {
        // border: left=0, right>0, bottom=0, top=0.
        if entry.border.left != 0 || entry.border.bottom != 0 || entry.border.top != 0 {
            return Err(err("MX_R1C4 requires border.{left, bottom, top} == 0"));
        }
        if entry.border.right <= 0 {
            return Err(err("MX_R1C4 requires border.right > 0"));
        }
    }
    if matches!(method, Method::MxR3c2) {
        // X: left=0, right=width. Y: top+bottom == height.
        if entry.border.left != 0 || entry.border.right as u32 != entry.rect.w {
            return Err(err("MX_R3C2 requires border.left == 0 AND border.right == sprite.rect.width"));
        }
        if entry.border.bottom + entry.border.top != entry.rect.h as i32 {
            return Err(err("MX_R3C2 requires border.bottom + border.top == sprite.rect.height"));
        }
    }
    if matches!(method, Method::MxR3c4) {
        // X: left=0, right>0. Y: free (uses bottom/top as 9-slice).
        if entry.border.left != 0 || entry.border.right <= 0 {
            return Err(err("MX_R3C4 requires border.left == 0 AND border.right > 0"));
        }
    }
    if matches!(method, Method::MxR3c6) {
        // No explicit C# asserts; we still need a sensible default. Require
        // positive right border so x1/x5 don't collapse.
        if entry.border.right <= 0 {
            return Err(err("MX_R3C6 requires border.right > 0"));
        }
    }
    if matches!(method, Method::MxR1c3 | Method::MxR3c3) {
        // Both mirror-X slice variants assert left == 0 and right == width.
        if entry.border.left != 0 {
            return Err(err("MX_R1C3 / MX_R3C3 require border.left == 0"));
        }
        if entry.border.right as u32 != entry.rect.w {
            return Err(err("MX_R1C3 / MX_R3C3 require border.right == sprite.rect.width"));
        }
    }
    if matches!(method, Method::MyR3c3) {
        // Mirror-Y 9-slice: bottom == 0, top == height.
        if entry.border.bottom != 0 {
            return Err(err("MY_R3C3 requires border.bottom == 0"));
        }
        if entry.border.top as u32 != entry.rect.h {
            return Err(err("MY_R3C3 requires border.top == sprite.rect.height"));
        }
    }
    if matches!(method, Method::MyR3c1) {
        // border.x == 0 AND border.z == 0, border.y == 0, border.w == height.
        if entry.border.left != 0 || entry.border.right != 0 {
            return Err(err("MY_R3C1 requires border.left == 0 AND border.right == 0"));
        }
        if entry.border.bottom != 0 || entry.border.top as u32 != entry.rect.h {
            return Err(err("MY_R3C1 requires border.bottom == 0 AND border.top == sprite.rect.height"));
        }
    }
    if matches!(method, Method::MyR3c2 | Method::MyR2c2) {
        // X: left == 0, right == width. Y: bottom == 0, top == height.
        if entry.border.left != 0 || entry.border.right as u32 != entry.rect.w {
            return Err(err("requires border.left == 0 AND border.right == sprite.rect.width"));
        }
        if entry.border.bottom != 0 || entry.border.top as u32 != entry.rect.h {
            return Err(err("requires border.bottom == 0 AND border.top == sprite.rect.height"));
        }
    }
    if matches!(method, Method::MyR2c3) {
        // X: left + right == width. Y: bottom == 0, top == height.
        if entry.border.left + entry.border.right != entry.rect.w as i32 {
            return Err(err("MY_R2C3 requires border.left + border.right == sprite.rect.width"));
        }
        if entry.border.bottom != 0 || entry.border.top as u32 != entry.rect.h {
            return Err(err("MY_R2C3 requires border.bottom == 0 AND border.top == sprite.rect.height"));
        }
    }
    if matches!(method, Method::MxyR3c3 | Method::MxyR3c3Nf) {
        // Mirror-XY: left/bottom == 0; right/top > 0. C# allows
        // border.x < 2 / border.y < 2 to absorb borderMult quantization;
        // we use the strict 0 form (callers can opt-out with borderMult
        // adjustments before authoring).
        if entry.border.left != 0 {
            return Err(err("MXY_R3C3 / MXY_R3C3_NF require border.left == 0"));
        }
        if entry.border.right <= 0 {
            return Err(err("MXY_R3C3 / MXY_R3C3_NF require border.right > 0"));
        }
        if entry.border.bottom != 0 {
            return Err(err("MXY_R3C3 / MXY_R3C3_NF require border.bottom == 0"));
        }
        if entry.border.top <= 0 {
            return Err(err("MXY_R3C3 / MXY_R3C3_NF require border.top > 0"));
        }
    }
    Ok(())
}

/// Build the mesh for a single polygon part with identity `offset` — the
/// SpriteRenderer / Box-prefab path. CSA callers go through `build_combined`,
/// which feeds the bridge-precomputed per-part offset into
/// `polygon_mesh_with_tris` directly.
///
/// - `vertices` are caller-supplied (interpreted in whatever frame the
///   caller wants — world units for the default path, canvas pixels under
///   the fab/Canvas path). The polygon is triangulated via ear-clip
///   (auto-handles winding).
/// - All UVs sample the center pixel of `polygon_sprite_rect`, normalized
///   against the atlas size — matches `SolidUVCache.Get` in meow-tower.
/// - `apply_transform` runs each vert through the affine chain *after*
///   triangulation. With `offset = (0, 0)` it's plain `T · R · S`.
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
    polygon_mesh_with_tris(vertices, None, affine, polygon_sprite_rect, atlas, (0.0, 0.0))
}

/// True when the 4-vert input matches `MeshBuilder.SetUp_Quad`'s emission
/// order: vert[0]=(x_min, y_min) BL, [1]=(x_max, y_min) BR,
/// [2]=(x_min, y_max) TL, [3]=(x_max, y_max) TR. The cheap check is
/// enough — the layout is machine-generated by UISolid / its sibling
/// primitives, never authored by hand.
///
/// Callers (CSA `polygon_mesh_with_tris`, SMA `mesh_emit::build_polygon_mesh`)
/// emit the canonical `[0, 2, 3, 3, 1, 0]` triangle list on a true return
/// to dodge ear-clip's signed-area cancellation on the BL/BR/TL/TR ring.
pub fn is_quad_layout(v: &[[f32; 2]]) -> bool {
    v.len() == 4
        && v[0][1] == v[1][1]
        && v[2][1] == v[3][1]
        && v[0][0] == v[2][0]
        && v[1][0] == v[3][0]
        && v[0][0] < v[1][0]
        && v[0][1] < v[2][1]
}

// Polygon-part mesh with optional explicit triangles. When `tris_override` is
// `Some`, those indices are used verbatim (caller-validated for length and
// range upstream). Otherwise we ear-clip via the triangulator — *unless* the
// input is a 4-vert SetUp_Quad layout, where ear-clip's signed-area cancels
// to zero on the bowtie ring `[BL, BR, TL, TR]`. UISolid's MonoBehaviour
// path stamps that same layout with `QuadIndexCache.Single = [0,2,3,3,1,0]`,
// so fab.json can omit the override for color quads and we derive it here.
//
// `offset` is the part's world-unit translation (bridge already applied the
// mode-implicit canvas factor). For SpriteRenderer / Box prefabs the caller
// passes `offset = (0, 0)` and the chain collapses to the plain affine.
fn polygon_mesh_with_tris(
    vertices: &[[f32; 2]],
    tris_override: Option<&[u16]>,
    affine: Affine,
    polygon_sprite_rect: Rect,
    atlas: AtlasSize,
    offset: (f32, f32),
) -> PartMesh {
    let tris = match tris_override {
        Some(t) => t.to_vec(),
        None if is_quad_layout(vertices) => vec![0, 2, 3, 3, 1, 0],
        None => triangulator::triangulate(vertices),
    };

    let ctx = SliceCtx {
        sprite_pivot_norm: (0.5, 0.5),
        sprite_bound_size: (1.0, 1.0),
        part_pivot: (0.5, 0.5),
        border_mult: 1.0,
        affine,
        offset,
    };
    let verts: Vec<[f32; 2]> = vertices.iter().map(|v| apply_transform(*v, &ctx)).collect();

    let [cx, cy] = polygon_uv_center(polygon_sprite_rect, atlas);
    let uvs: Vec<[f32; 2]> = vec![[cx, cy]; vertices.len()];

    PartMesh { verts, uvs, tris }
}

/// UV coordinates sampled at the center pixel of a `Color_*` atlas rect.
///
/// Multiplies by the reciprocal of atlas width/height to match Unity's
/// `SolidUVCache.Get`, which averages `DataUtility.GetInnerUV`'s
/// already-multiplied `innerUV.x` / `innerUV.z`. The algebraically
/// equivalent `(rect.x + rect.w*0.5) / atlas.width` loses 1 ULP. Shared
/// between the CSA polygon path ([`polygon_mesh`]) and the SMA polygon
/// path (`mesh_emit::build_mesh`).
pub fn polygon_uv_center(rect: Rect, atlas: AtlasSize) -> [f32; 2] {
    let inv_w = 1.0_f32 / atlas.width as f32;
    let inv_h = 1.0_f32 / atlas.height as f32;
    let cx = (rect.x as f32 * inv_w + (rect.x + rect.w) as f32 * inv_w) * 0.5;
    let cy = (rect.y as f32 * inv_h + (rect.y + rect.h) as f32 * inv_h) * 0.5;
    [cx, cy]
}


/// Build the mesh for a single atlas-sprite part under the given method.
///
/// Source verts in the tpsheet entry are atlas-pixel, sprite-rect-relative
/// (px ∈ [0, w]). They get converted to the part's local frame (pivot-relative
/// world units) via `(px - w·pivotX, py - h·pivotY) * (1/ppu)` — multiply by
/// a precomputed reciprocal, not direct division, to match Unity's f32 op
/// order for `Sprite.vertices` (see `local_src_verts`). The per-method
/// slice/mirror/tile math then transforms into the target-rect frame, and
/// `apply_transform` runs each vert through the per-part affine then adds
/// the bridge-precomputed world-unit `offset`.
///
/// Every `Method` variant is wired (ID / MX / MY / MXY plus the slice-grid
/// family and the TX/TY/TX_MC3 tilers). FX/FY/FXY are rejected at parse
/// time in favor of negative `sx`/`sy`.
#[allow(clippy::too_many_arguments)] // public dispatch surface — each arg is meaningful
pub fn atlas_sprite_mesh(
    entry: &SpriteEntry,
    method: Method,
    size: Option<(f32, f32)>,
    part_pivot: [f32; 2],
    border_mult: f32,
    affine: Affine,
    atlas: AtlasSize,
    ppu: f32,
    invert_scale: f32,
    offset: (f32, f32),
) -> PartMesh {
    // The sprite's effective PPU = ppu / invert_scale = ppu * spriteScale.
    // Sprite.vertices in Unity are stored in world units = pixel /
    // effective_ppu; we have to mirror that so verts match byte-for-byte.
    // (TexturePacker's per-sprite spriteScale, parsed in tps.rs, is the
    //  source of `invert_scale` = 1 / spriteScale.)
    let effective_ppu = ppu / invert_scale;
    let src = SrcMesh {
        verts: local_src_verts(entry, effective_ppu),
        uvs: atlas_uvs(entry, atlas),
        tris: entry.geometry.triangles.clone(),
    };
    let ctx = SliceCtx {
        sprite_pivot_norm: (entry.pivot.x, entry.pivot.y),
        sprite_bound_size: (entry.rect.w as f32 / effective_ppu, entry.rect.h as f32 / effective_ppu),
        part_pivot: (part_pivot[0], part_pivot[1]),
        border_mult,
        affine,
        offset,
    };
    match method {
        // ID:
        //   - size=None  → native-scale (UIIconMeshGen.Identity): src.verts
        //                  pass through unmodified.
        //   - size=Some  → stretch-to-rect (UISliceMeshGen.Identity): src.verts
        //                  scaled to fill the target rect, anchored at the
        //                  part_pivot. Used by Color_* solid bars in CSA.
        Method::Id => match size {
            Some(sz) => slice_identity(&src, &ctx, sz),
            None     => {
                let verts = src.verts.iter().map(|v| apply_transform(*v, &ctx)).collect();
                PartMesh { verts, uvs: src.uvs, tris: src.tris }
            }
        },
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
        Method::Tx  => tile_axis(entry, atlas, ppu, &ctx, size.expect("tx size"), TileAxis::X),
        Method::Ty  => tile_axis(entry, atlas, ppu, &ctx, size.expect("ty size"), TileAxis::Y),
        Method::TxMc3 => tile_x_mc3(entry, atlas, ppu, &ctx, size.expect("tx_mc3 size")),
        Method::R1c3 => slice_r1c3(entry, atlas, ppu, &ctx, size.expect("r1c3 size")),
        Method::R3c3 => slice_r3c3(entry, atlas, ppu, &ctx, size.expect("r3c3 size"), false),
        Method::R3c3Nf => slice_r3c3(entry, atlas, ppu, &ctx, size.expect("r3c3_nf size"), true),
        Method::MxR1c3 => slice_mx_r1c3(entry, atlas, ppu, &ctx, size.expect("mx_r1c3 size")),
        Method::MxR1c4 => slice_mx_r1c4(entry, atlas, ppu, &ctx, size.expect("mx_r1c4 size")),
        Method::MxR3c2 => slice_mx_r3c2(entry, atlas, ppu, &ctx, size.expect("mx_r3c2 size")),
        Method::MxR3c3 => slice_mx_r3c3(entry, atlas, ppu, &ctx, size.expect("mx_r3c3 size")),
        Method::MxR3c4 => slice_mx_r3c4(entry, atlas, ppu, &ctx, size.expect("mx_r3c4 size")),
        Method::MxR3c6 => slice_mx_r3c6(entry, atlas, ppu, &ctx, size.expect("mx_r3c6 size")),
        Method::MyR3c1 => slice_my_r3c1(entry, atlas, ppu, &ctx, size.expect("my_r3c1 size")),
        Method::MyR2c2 => slice_my_r2c2(entry, atlas, ppu, &ctx, size.expect("my_r2c2 size")),
        Method::MyR2c3 => slice_my_r2c3(entry, atlas, ppu, &ctx, size.expect("my_r2c3 size")),
        Method::MyR3c2 => slice_my_r3c2(entry, atlas, ppu, &ctx, size.expect("my_r3c2 size")),
        Method::MyR3c3 => slice_my_r3c3(entry, atlas, ppu, &ctx, size.expect("my_r3c3 size")),
        Method::MxyR3c3 => slice_mxy_r3c3(entry, atlas, ppu, &ctx, size.expect("mxy_r3c3 size"), false),
        Method::MxyR3c3Nf => slice_mxy_r3c3(entry, atlas, ppu, &ctx, size.expect("mxy_r3c3_nf size"), true),
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
    border_mult: f32,
    affine: Affine,
    // Per-part world-unit offset. Already canvas-scaled by the bridge
    // (`Output::canvas_scale_implicit`), so the per-vert chain is a single
    // add: `v_world = affine·v + offset`. For SpriteMeshAuthor / Box prefabs
    // `offset = (0, 0)` and the chain collapses to the affine-only path.
    offset: (f32, f32),
}

// (px, py) → pivot-relative world units (matches Unity's Sprite.vertices).
//
// Critical: Unity stores `pixel * (1/ppu)` (multiplication by a precomputed
// f32 reciprocal), NOT direct `pixel / ppu`. The two differ by 1 ULP on
// common inputs — verified via llm-bridge:
//   18.0f / 80.0f      = 0x3e666666  (Rust default; standard IEEE division)
//   18.0f * (1.0f/80f) = 0x3e666667  (what Unity's Sprite.vertices stores)
// Bit-exact matching of the typelessdata block requires reproducing this
// op order. Likely Unity's native sprite-creator precomputes `1/ppu` once
// for SIMD efficiency.
fn local_src_verts(entry: &SpriteEntry, ppu: f32) -> Vec<[f32; 2]> {
    let pw = entry.rect.w as f32;
    let ph = entry.rect.h as f32;
    let pivot_px = (pw * entry.pivot.x, ph * entry.pivot.y);
    let inv_ppu = 1.0_f32 / ppu;
    entry.geometry.vertices.iter()
        .map(|v| [(v.x - pivot_px.0) * inv_ppu, (v.y - pivot_px.1) * inv_ppu])
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
// target rect's pivot (defaults to (0.5, 0.5) but threads through from the
// per-part `partPivot` field — Box prefabs leave it default; UI hierarchies
// like CanvasSpriteAuthor carry mixed pivots such as (0, 0.5)). See
// docs/fab.md "Schema" for the field; callers pass `ctx.part_pivot` here.

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

// --- tiling (TX / TY) — port of meow-tower's Tiling.cs ----------------------

#[derive(Clone, Copy)]
enum TileAxis { X, Y }

struct TileLayout {
    tail_fract: f32,
    has_tail: bool,
    cols_half: usize,  // tile columns per side, excluding the centre seam
    vert_half: usize,  // verts per side (cols_half * 2 + 1)
    vert_count: usize, // total verts (vert_half * 2)
}

const TAIL_EPSILON: f32 = 0.0001;

fn calc_tile_layout(tile_w: f32, rect_w: f32) -> TileLayout {
    // `tiles` is per-side; we ping-pong outward from the centre seam.
    let tiles = (rect_w / tile_w) * 0.5;
    let half_tiles = tiles.floor() as usize;
    let tail_fract = tiles - half_tiles as f32;
    let has_tail = tail_fract > TAIL_EPSILON;
    let cols_half = half_tiles + if has_tail { 1 } else { 0 };
    let vert_half = cols_half * 2 + 1;
    let vert_count = vert_half * 2;
    TileLayout { tail_fract, has_tail, cols_half, vert_half, vert_count }
}

// Lerp matching C# Mathf.Lerp's clamped semantics. Used to sample partial-tail UVs.
fn sample_partial_uv(min: f32, max: f32, tail_fract: f32, forward: bool) -> f32 {
    let t = tail_fract.clamp(0.0, 1.0);
    if forward { max + (min - max) * t } else { min + (max - min) * t }
}

// TX/TY produce a strap of axis-aligned quads in the target rect, with the
// source sprite ping-ponging outward from the rect centre. The opposite side
// is a mirror of the forward side (geometry + UVs both).
//
// Layout (per BuildStrapIndices in Tiling.cs): vert pairs go
//   [centre, forward_1, forward_2, ..., forward_colsHalf,
//    backward_1, backward_2, ..., backward_colsHalf]
// with `quadCount = colsHalf * 2` quads stitched outward then around the
// centre seam.
fn tile_axis(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
    axis: TileAxis,
) -> PartMesh {
    let (target_w, target_h) = target;
    let part_pivot = ctx.part_pivot;

    // Rect bounds in target-rect local space (origin at part pivot point).
    let x_min = -target_w * part_pivot.0;
    let x_max = target_w * (1.0 - part_pivot.0);
    let y_min = -target_h * part_pivot.1;
    let y_max = target_h * (1.0 - part_pivot.1);

    // Outer UV bounds for the source sprite (atlas-normalized).
    let aw = atlas.width as f32;
    let ah = atlas.height as f32;
    let u_min = entry.rect.x as f32 / aw;
    let u_max = (entry.rect.x + entry.rect.w) as f32 / aw;
    let v_min = entry.rect.y as f32 / ah;
    let v_max = (entry.rect.y + entry.rect.h) as f32 / ah;

    // Tile size + rect size along the tiling axis. Both in world units.
    let (tile_size, rect_size, axis_min, axis_max) = match axis {
        TileAxis::X => (entry.rect.w as f32 / ppu, target_w, x_min, x_max),
        TileAxis::Y => (entry.rect.h as f32 / ppu, target_h, y_min, y_max),
    };
    let layout = calc_tile_layout(tile_size, rect_size);

    let mut verts = vec![[0.0f32; 2]; layout.vert_count];
    let mut uvs = vec![[0.0f32; 2]; layout.vert_count];

    // Fixed (non-axis) coordinate + UV — the cross-axis edges of every column.
    // C# TileX runs with poses[i*2].y = yMin, poses[i*2+1].y = yMax.
    // TileY swaps roles (X is the cross-axis, with x_max/x_min winding flipped
    // to keep CCW). We mirror that here.
    let (cross_lo, cross_hi, cross_uv_lo, cross_uv_hi) = match axis {
        TileAxis::X => (y_min, y_max, v_min, v_max),
        TileAxis::Y => (x_max, x_min, u_min, u_max),
    };
    for i in 0..layout.vert_half {
        let idx0 = i * 2;
        let idx1 = idx0 + 1;
        write_cross(&mut verts[idx0], axis, cross_lo);
        write_cross(&mut verts[idx1], axis, cross_hi);
        uvs[idx0] = uv_set_cross(axis, cross_uv_lo);
        uvs[idx1] = uv_set_cross(axis, cross_uv_hi);
    }

    // Centre seam: position at the rect midpoint, UV at sprite uMin/vMin.
    let mid = (axis_min + axis_max) * 0.5;
    let centre_uv = match axis { TileAxis::X => u_min, TileAxis::Y => v_min };
    write_axis(&mut verts[0], axis, mid);
    write_axis(&mut verts[1], axis, mid);
    uvs[0] = uv_set_axis(axis, centre_uv, uvs[0]);
    uvs[1] = uv_set_axis(axis, centre_uv, uvs[1]);

    // Forward side (rises from centre toward axis_max).
    let mut vp = 2;
    let mut cursor = mid;
    let (uv_lo, uv_hi) = match axis {
        TileAxis::X => (u_min, u_max),
        TileAxis::Y => (v_min, v_max),
    };
    for col in 1..=layout.cols_half {
        let flow_forward = col % 2 == 0; // C# .IsEven() on col index
        let is_partial = col == layout.cols_half && layout.has_tail;
        let (delta, u) = if is_partial {
            (
                layout.tail_fract * tile_size,
                sample_partial_uv(uv_lo, uv_hi, layout.tail_fract, flow_forward),
            )
        } else {
            (tile_size, if flow_forward { uv_lo } else { uv_hi })
        };
        cursor += delta;
        write_axis(&mut verts[vp], axis, cursor);
        write_axis(&mut verts[vp + 1], axis, cursor);
        uvs[vp] = uv_set_axis(axis, u, uvs[vp]);
        uvs[vp + 1] = uv_set_axis(axis, u, uvs[vp + 1]);
        vp += 2;
    }

    // Backward side: positions mirror around mid; UVs mirror by copying
    // forward UV pairs starting at offset 2.
    let pos_offset = mid * 2.0;
    for i in 1..=layout.cols_half {
        let src_pos = match axis { TileAxis::X => verts[i * 2][0], TileAxis::Y => verts[i * 2][1] };
        let dst = pos_offset - src_pos;
        write_axis(&mut verts[vp], axis, dst);
        write_axis(&mut verts[vp + 1], axis, dst);
        // C# MirrorUV: copy forward UVs (indices 2..vert_half) verbatim into
        // the backward block. Same texel is sampled — geometry handles the flip.
        uvs[vp]     = uvs[i * 2];
        uvs[vp + 1] = uvs[i * 2 + 1];
        vp += 2;
    }

    // Triangulate: forward strap + centre-seam quad + backward strap.
    let tris = build_strap_indices(layout.cols_half);

    // Apply affine after all positions are laid out.
    let verts: Vec<[f32; 2]> = verts.iter().map(|v| apply_transform(*v, ctx)).collect();

    PartMesh { verts, uvs, tris }
}

fn write_axis(v: &mut [f32; 2], axis: TileAxis, value: f32) {
    match axis {
        TileAxis::X => v[0] = value,
        TileAxis::Y => v[1] = value,
    }
}

fn write_cross(v: &mut [f32; 2], axis: TileAxis, value: f32) {
    match axis {
        TileAxis::X => v[1] = value,
        TileAxis::Y => v[0] = value,
    }
}

fn uv_set_axis(axis: TileAxis, value: f32, existing: [f32; 2]) -> [f32; 2] {
    match axis {
        TileAxis::X => [value, existing[1]],
        TileAxis::Y => [existing[0], value],
    }
}

fn uv_set_cross(axis: TileAxis, value: f32) -> [f32; 2] {
    match axis {
        TileAxis::X => [0.0, value],
        TileAxis::Y => [value, 0.0],
    }
}

fn build_strap_indices(cols_half: usize) -> Vec<u16> {
    // Quads laid out per BuildStrapIndices in Tiling.cs: forward strap, centre
    // join, backward strap. 6 indices per quad.
    let mut tris: Vec<u16> = Vec::with_capacity(cols_half * 2 * 6);

    // Forward strap.
    for i in 0..cols_half {
        let a = (i * 2) as u16;
        let b = a + 1;
        let c = a + 2;
        let d = a + 3;
        push_quad(&mut tris, a, b, c, d);
    }

    // Centre-seam quad (joins the last forward column back to the centre).
    let offset = (cols_half * 2 + 2) as u16;
    push_quad(&mut tris, offset, offset + 1, 0, 1);

    // Backward strap.
    for i in 1..cols_half {
        let a = offset + (i * 2) as u16;
        let b = a + 1;
        let c = a - 2;
        let d = a - 1;
        push_quad(&mut tris, a, b, c, d);
    }

    tris
}

fn push_quad(tris: &mut Vec<u16>, a: u16, b: u16, c: u16, d: u16) {
    tris.extend_from_slice(&[a, b, d, a, d, c]);
}

// --- slice grids (R3C3 and friends) — ports of UISliceMeshGen.cs ----------

// 4×4 UV grid for a bordered sprite. Mirrors meow-tower's UVMatrix, which
// wraps Unity's DataUtility.GetOuter/InnerUV. The inner border bounds are
// the atlas-normalized inset by `border` pixels on each side of the sprite's
// outer rect (post-borderMult scaling).
#[derive(Debug, Clone, Copy)]
struct UvGrid {
    u: [f32; 4], // u[0..4] = outer-min, inner-min, inner-max, outer-max
    v: [f32; 4],
}

impl UvGrid {
    fn for_entry(entry: &SpriteEntry, atlas: AtlasSize, border_mult: f32) -> Self {
        let aw = atlas.width as f32;
        let ah = atlas.height as f32;
        let r = entry.rect;
        let u_min = r.x as f32 / aw;
        let u_max = (r.x + r.w) as f32 / aw;
        let v_min = r.y as f32 / ah;
        let v_max = (r.y + r.h) as f32 / ah;
        // Border inset in atlas-normalized space, scaled by borderMult.
        let bl = entry.border.left as f32 * border_mult / aw;
        let br = entry.border.right as f32 * border_mult / aw;
        let bb = entry.border.bottom as f32 * border_mult / ah;
        let bt = entry.border.top as f32 * border_mult / ah;
        UvGrid {
            u: [u_min, u_min + bl, u_max - br, u_max],
            v: [v_min, v_min + bb, v_max - bt, v_max],
        }
    }
    fn uv(&self, col: usize, row: usize) -> [f32; 2] { [self.u[col], self.v[row]] }
}

// Grid index buffer for a `rows × cols` quad grid (CW winding to match
// CreateIndices in GridIndex.cs).
fn create_grid_indices(rows: usize, cols: usize) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::with_capacity(rows * cols * 6);
    let mut vr: u16 = 0;
    for _ in 0..rows {
        for x in 0..cols as u16 {
            let vl = vr + x;
            let vu = vl + cols as u16 + 1;
            // Upper-left triangle, then lower-right (CW).
            out.extend_from_slice(&[vl, vu, vu + 1, vu + 1, vl + 1, vl]);
        }
        vr += cols as u16 + 1;
    }
    out
}

// 16 verts on a 4×4 grid laid out row-major: (x0,y0),(x1,y0),(x2,y0),(x3,y0),
// (x0,y1),..., (x3,y3). Matches GridPos.SetUp_R3C3.
fn r3c3_positions(
    target: (f32, f32),
    part_pivot: (f32, f32),
    border: (f32, f32, f32, f32), // L, B, R, T in world units
) -> Vec<[f32; 2]> {
    let (target_w, target_h) = target;
    let x_min = -target_w * part_pivot.0;
    let x_max = target_w * (1.0 - part_pivot.0);
    let y_min = -target_h * part_pivot.1;
    let y_max = target_h * (1.0 - part_pivot.1);
    let x = [x_min, x_min + border.0, x_max - border.2, x_max];
    let y = [y_min, y_min + border.1, y_max - border.3, y_max];
    let mut out = Vec::with_capacity(16);
    for &yi in &y {
        for &xi in &x {
            out.push([xi, yi]);
        }
    }
    out
}

fn r3c3_uvs(grid: &UvGrid) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(16);
    for row in 0..4 {
        for col in 0..4 {
            out.push(grid.uv(col, row));
        }
    }
    out
}

// R1C3: 1 row × 3 cols slice. Centre column collapses to a single U
// coordinate (border.left + border.right == sprite.rect.width — enforced by
// check_method_constraints).
fn slice_r1c3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);

    let bl = entry.border.left as f32 * ctx.border_mult / ppu;
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let x = [x_min, x_min + bl, x_max - br, x_max];
    let y = [y_min, y_max];

    let mut verts_local: Vec<[f32; 2]> = Vec::with_capacity(8);
    for &yi in &y {
        for &xi in &x {
            verts_local.push([xi, yi]);
        }
    }

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Inner verts share inner-min U (centre column collapses). UVs map y[0]
    // → outer-min V (=v[0]); y[1] → outer-max V (=v[3]).
    let uvs = vec![
        [grid.u[0], grid.v[0]],
        [grid.u[1], grid.v[0]],
        [grid.u[1], grid.v[0]], // <- same U as vert 1 (centre share)
        [grid.u[3], grid.v[0]],
        [grid.u[0], grid.v[3]],
        [grid.u[1], grid.v[3]],
        [grid.u[1], grid.v[3]], // <- same U as vert 5
        [grid.u[3], grid.v[3]],
    ];

    let tris = create_grid_indices(1, 3);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MX_R1C3: mirror-X variant of R1C3. Outer columns sample u3 (atlas-max U);
// inner columns sample u0 (atlas-min U). Source-rect constraints
// (left==0, right==width) are validated upstream.
//
// Edge width in C# is `sprite.rect.size.x * borderMult` (not border.x). Under
// the constraint border.right == rect.w these are equivalent, but the literal
// form matches the source.
fn slice_mx_r1c3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);

    let b = entry.rect.w as f32 * ctx.border_mult / ppu;
    let x = [x_min, x_min + b, x_max - b, x_max];
    let y = [y_min, y_max];

    let mut verts_local: Vec<[f32; 2]> = Vec::with_capacity(8);
    for &yi in &y {
        for &xi in &x {
            verts_local.push([xi, yi]);
        }
    }

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Mirror-X layout: outer cols → u3; inner cols → u0. Both bottom and top.
    let uvs = vec![
        [grid.u[3], grid.v[0]],
        [grid.u[0], grid.v[0]],
        [grid.u[0], grid.v[0]],
        [grid.u[3], grid.v[0]],
        [grid.u[3], grid.v[3]],
        [grid.u[0], grid.v[3]],
        [grid.u[0], grid.v[3]],
        [grid.u[3], grid.v[3]],
    ];

    let tris = create_grid_indices(1, 3);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MX_R3C3: mirror-X 9-slice. Same vertex grid as R3C3 but UV cells per
// SetUp_MX_R3C3 in GridUV.cs:
//   row 0 (v0): u3, u0, u0, u3
//   row 1 (v1): u3, u0, u0, u3  -- but with v[1] not v[0]; C# uses _31/_01
//   row 2 (v2): _32/_02
//   row 3 (v3): _33/_03
// I.e. outer cols always sample u3, inner cols always sample u0; rows pick
// v0..v3 of UvGrid.
fn slice_mx_r3c3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    // Position grid: x edges use border.right (== rect.w under constraint).
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let bb = entry.border.bottom as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;
    // Pass (left, bottom, right, top); left=right under MX_R3C3.
    let verts_local = r3c3_positions(target, ctx.part_pivot, (br, bb, br, bt));

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(16);
    // Each row picks v[row]; cols are: u3, u0, u0, u3.
    for row in 0..4 {
        uvs.push([grid.u[3], grid.v[row]]);
        uvs.push([grid.u[0], grid.v[row]]);
        uvs.push([grid.u[0], grid.v[row]]);
        uvs.push([grid.u[3], grid.v[row]]);
    }

    let tris = create_grid_indices(3, 3);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MXY_R3C3 / MXY_R3C3_NF: mirror both axes. Source-rect constraints:
// border.left/bottom == 0, border.right/top > 0. After the mirror
// transform, both X edges share border.right and both Y edges share
// border.top.
//
// UV layout from GridUV.SetUp_MXY_R3C3: each cell samples u from
// {u2 (inner), u3 (outer)} and v from {v2 (inner), v3 (outer)} depending
// on which slice the cell sits in. Effectively only the atlas's top-right
// quarter is sampled and reflected to all four corners.
fn slice_mxy_r3c3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
    no_fill: bool,
) -> PartMesh {
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;
    // X borders both = right; Y borders both = top (mirror-XY).
    let verts_local = r3c3_positions(target, ctx.part_pivot, (br, bt, br, bt));

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Per-col U: outer cols (0, 3) → u3, inner cols (1, 2) → u2.
    // Per-row V: outer rows (0, 3) → v3, inner rows (1, 2) → v2.
    let u_table = [grid.u[3], grid.u[2], grid.u[2], grid.u[3]];
    let v_table = [grid.v[3], grid.v[2], grid.v[2], grid.v[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(16);
    for &vv in &v_table {
        for &uu in &u_table {
            uvs.push([uu, vv]);
        }
    }

    let tris = if no_fill { r3c3_nf_indices() } else { create_grid_indices(3, 3) };
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// --- shared slice helpers ---

// Compute the part's target-rect bounds in pivot-relative world units.
// Returns (x_min, y_min, x_max, y_max).
fn target_rect_bounds(target: (f32, f32), part_pivot: (f32, f32)) -> (f32, f32, f32, f32) {
    let (tw, th) = target;
    let x_min = -tw * part_pivot.0;
    let x_max = tw * (1.0 - part_pivot.0);
    let y_min = -th * part_pivot.1;
    let y_max = th * (1.0 - part_pivot.1);
    (x_min, y_min, x_max, y_max)
}

// Row-major vertex grid: outputs (xs.len() * ys.len()) verts, iterating
// y outer, x inner. Matches the layout GridPos.SetUp_R*C* writes.
fn grid_verts(xs: &[f32], ys: &[f32]) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(xs.len() * ys.len());
    for &y in ys {
        for &x in xs {
            out.push([x, y]);
        }
    }
    out
}

// --- MX slice grid ports follow ---

// MX_R1C4: 2 rows × 5 cols, mirror-X with double centre column. 10 verts,
// 4 quads. C# ignores borderMult (hardcodes 1); we honor ctx.border_mult
// for consistency with sibling methods. Constraint: borders all 0 except
// right > 0.
fn slice_mx_r1c4(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let br = entry.border.right as f32 * ctx.border_mult / ppu;

    let xs = [x_min, x_min + br, (x_min + x_max) * 0.5, x_max - br, x_max];
    let ys = [y_min, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Row 0: u3, u2, u0, u2, u3 (mirror about centre at u0).
    // Row 1: same pattern with v3.
    let col_u = [grid.u[3], grid.u[2], grid.u[0], grid.u[2], grid.u[3]];
    let row_v = [grid.v[0], grid.v[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(10);
    for &vv in &row_v {
        for &uu in &col_u {
            uvs.push([uu, vv]);
        }
    }

    let tris = create_grid_indices(1, 4);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MX_R3C2: 4 rows × 3 cols, mirror-X + vertical 9-slice. 12 verts, 6 quads.
// Constraint: X left=0, right=width; Y bottom+top=height.
fn slice_mx_r3c2(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let bb = entry.border.bottom as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;

    let xs = [x_min, (x_min + x_max) * 0.5, x_max];
    let ys = [y_min, y_min + bb, y_max - bt, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Cols (3): outer=u3, centre=u0, outer=u3 (mirror around centre).
    let col_u = [grid.u[3], grid.u[0], grid.u[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(12);
    for row in 0..4 {
        let vv = grid.v[row];
        for &uu in &col_u {
            uvs.push([uu, vv]);
        }
    }

    let tris = create_grid_indices(3, 2);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MX_R3C4: 4 rows × 5 cols, mirror-X + full vertical 9-slice. 20 verts,
// 12 quads. Constraint: X left=0, right>0 (no Y constraint).
fn slice_mx_r3c4(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let bb = entry.border.bottom as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;

    let xs = [x_min, x_min + br, (x_min + x_max) * 0.5, x_max - br, x_max];
    let ys = [y_min, y_min + bb, y_max - bt, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Per SetUp_MX_R3C4: per-row UV pattern (u3, u2, u0, u2, u3) with v
    // running through grid.v[0..3]. Row index → grid.v[row].
    let col_u = [grid.u[3], grid.u[2], grid.u[0], grid.u[2], grid.u[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(20);
    for row in 0..4 {
        let vv = grid.v[row];
        for &uu in &col_u {
            uvs.push([uu, vv]);
        }
    }

    let tris = create_grid_indices(3, 4);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MX_R3C6: 4 rows × 7 cols, double-mirror-X + 9-slice. 28 verts, 18 quads.
// Uses both border.x (inner) and border.z (outer) to split into 6 columns.
fn slice_mx_r3c6(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let bl = entry.border.left as f32 * ctx.border_mult / ppu;
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let bb = entry.border.bottom as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;
    let x_mid = (x_min + x_max) * 0.5;

    let xs = [x_min, x_min + br, x_mid - bl, x_mid, x_mid + bl, x_max - br, x_max];
    let ys = [y_min, y_min + bb, y_max - bt, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Per SetUp_MX_R3C6: per-row UV pattern (u3, u2, u1, u0, u1, u2, u3)
    // with v running through grid.v[0..3].
    let col_u = [grid.u[3], grid.u[2], grid.u[1], grid.u[0], grid.u[1], grid.u[2], grid.u[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(28);
    for row in 0..4 {
        let vv = grid.v[row];
        for &uu in &col_u {
            uvs.push([uu, vv]);
        }
    }

    let tris = create_grid_indices(3, 6);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MY_R3C1: 3 rows × 1 col, mirror-Y. Source-rect: borders all 0 except top
// = height. 8 verts (4 rows × 2 cols), 3 quads.
fn slice_my_r3c1(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;

    let xs = [x_min, x_max];
    let ys = [y_min, y_min + bt, y_max - bt, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Per GridUV.SetUp_MY_R3C1: outer rows → v3; inner rows → v0.
    // U progresses u0..u3 (2 cols), so cols → u0, u3 within each row.
    let row_v = [grid.v[3], grid.v[0], grid.v[0], grid.v[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(8);
    for &vv in &row_v {
        uvs.push([grid.u[0], vv]);
        uvs.push([grid.u[3], vv]);
    }

    let tris = create_grid_indices(3, 1);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MY_R2C2: 2 rows × 2 cols, mirror-Y + mirror-X-edge. 9 verts (3 rows × 3
// cols), 4 quads. Position: x0..x2 (right edge at x_max - br); y0..y2
// (centre row at midpoint).
fn slice_my_r2c2(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let br = entry.border.right as f32 * ctx.border_mult / ppu;

    let xs = [x_min, x_max - br, x_max];
    let ys = [y_min, (y_min + y_max) * 0.5, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // SetUp_MY_R2C2 pattern: cols 0,1 share u0; col 2 = u3.
    // Rows 0,2 → v3; row 1 → v0.
    let row_v = [grid.v[3], grid.v[0], grid.v[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(9);
    for &vv in &row_v {
        uvs.push([grid.u[0], vv]);
        uvs.push([grid.u[0], vv]);
        uvs.push([grid.u[3], vv]);
    }

    let tris = create_grid_indices(2, 2);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MY_R2C3: 2 rows × 3 cols, mirror-Y + horizontal slice. 12 verts (3 rows
// × 4 cols), 6 quads. Constraint: border.left + border.right == rect.w.
fn slice_my_r2c3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let bl = entry.border.left as f32 * ctx.border_mult / ppu;
    let br = entry.border.right as f32 * ctx.border_mult / ppu;

    let xs = [x_min, x_min + bl, x_max - br, x_max];
    let ys = [y_min, (y_min + y_max) * 0.5, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // SetUp_MY_R2C3 pattern:
    //   col 0 → u0; cols 1,2 → u1 (shared, atlas inner-min via border-x);
    //   col 3 → u3.
    // Rows 0,2 → v3; row 1 → v0.
    let row_v = [grid.v[3], grid.v[0], grid.v[3]];
    let col_u = [grid.u[0], grid.u[1], grid.u[1], grid.u[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(12);
    for &vv in &row_v {
        for &uu in &col_u {
            uvs.push([uu, vv]);
        }
    }

    let tris = create_grid_indices(2, 3);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MY_R3C2: 3 rows × 2 cols, mirror-Y + mirror-X-edge. 12 verts (4 rows ×
// 3 cols), 6 quads. Constraints same as MY_R2C2.
fn slice_my_r3c2(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (x_min, y_min, x_max, y_max) = target_rect_bounds(target, ctx.part_pivot);
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;

    let xs = [x_min, x_max - br, x_max];
    let ys = [y_min, y_min + bt, y_max - bt, y_max];
    let verts_local = grid_verts(&xs, &ys);

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // SetUp_MY_R3C2 pattern: cols 0,1 → u0 (shared); col 2 → u3.
    // Rows 0,3 → v3; rows 1,2 → v0.
    let row_v = [grid.v[3], grid.v[0], grid.v[0], grid.v[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(12);
    for &vv in &row_v {
        uvs.push([grid.u[0], vv]);
        uvs.push([grid.u[0], vv]);
        uvs.push([grid.u[3], vv]);
    }

    let tris = create_grid_indices(3, 2);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// MY_R3C3: mirror-Y 9-slice. Source-rect: bottom == 0, top == height.
//
// Vertex layout uses the standard R3C3 grid, but both Y borders are inset by
// the top-border value (the source's only non-zero Y border). UV rows
// mirror about the centre: outer rows sample v3 (atlas-top); inner rows
// sample v0 (atlas-bottom).
fn slice_my_r3c3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let bl = entry.border.left as f32 * ctx.border_mult / ppu;
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;
    // C# uses b.w (top border) for both y1 and y2 insets. We do the same:
    // bottom-inset and top-inset both = bt.
    let verts_local = r3c3_positions(target, ctx.part_pivot, (bl, bt, br, bt));

    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    // Row-major UVs: rows 0,3 → v3 (atlas-top), rows 1,2 → v0 (atlas-bottom).
    // U progresses u0..u3 within each row.
    let row_v = [grid.v[3], grid.v[0], grid.v[0], grid.v[3]];
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(16);
    for &vv in &row_v {
        for col in 0..4 {
            uvs.push([grid.u[col], vv]);
        }
    }

    let tris = create_grid_indices(3, 3);
    let verts: Vec<[f32; 2]> = verts_local.iter().map(|v| apply_transform(*v, ctx)).collect();
    PartMesh { verts, uvs, tris }
}

// R3C3 / R3C3_NF: same vert + UV layout; differ only in whether the centre
// quad (4 indices) is included. Source-rect padding is currently treated as
// zero (TexturePacker atlases produce tight rects in our pipeline).
fn slice_r3c3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
    no_fill: bool,
) -> PartMesh {
    let bl = entry.border.left as f32 * ctx.border_mult / ppu;
    let br = entry.border.right as f32 * ctx.border_mult / ppu;
    let bb = entry.border.bottom as f32 * ctx.border_mult / ppu;
    let bt = entry.border.top as f32 * ctx.border_mult / ppu;
    let verts_local = r3c3_positions(target, ctx.part_pivot, (bl, bb, br, bt));
    let grid = UvGrid::for_entry(entry, atlas, ctx.border_mult);
    let uvs = r3c3_uvs(&grid);
    let tris = if no_fill { r3c3_nf_indices() } else { create_grid_indices(3, 3) };

    let verts: Vec<[f32; 2]> = verts_local.iter()
        .map(|v| apply_transform(*v, ctx))
        .collect();
    PartMesh { verts, uvs, tris }
}

// R3C3_NF: same as R3C3 but the centre quad (verts 5, 6, 9, 10) is omitted.
// Literal from GridIndex.cs.
fn r3c3_nf_indices() -> Vec<u16> {
    vec![
        0, 4, 5, 5, 1, 0,    // BL
        1, 5, 6, 6, 2, 1,    // BC
        2, 6, 7, 7, 3, 2,    // BR
        4, 8, 9, 9, 5, 4,    // ML
        6, 10, 11, 11, 7, 6, // MR
        8, 12, 13, 13, 9, 8, // TL
        9, 13, 14, 14, 10, 9,// TC
        10, 14, 15, 15, 11, 10, // TR
    ]
}

// TX_MC3: 3-section layout along X — mirrored left edge | tiled centre |
// right edge. Port of Tiling.cs TileX_MC3. Source border requirements
// (left == 0, right > 0) are validated by `check_method_constraints`
// before this runs.
fn tile_x_mc3(
    entry: &SpriteEntry,
    atlas: AtlasSize,
    ppu: f32,
    ctx: &SliceCtx,
    target: (f32, f32),
) -> PartMesh {
    let (target_w, target_h) = target;
    let pp = ctx.part_pivot;

    // Target-rect bounds in part-local space.
    let x_min = -target_w * pp.0;
    let x_max = target_w * (1.0 - pp.0);
    let y_min = -target_h * pp.1;
    let y_max = target_h * (1.0 - pp.1);

    // Edge width in world units. C# uses sprite.border.z * borderMult — the
    // border is pixels in our tpsheet, so divide by PPU first.
    let edge_w = (entry.border.right as f32 / ppu) * ctx.border_mult;

    let aw = atlas.width as f32;
    let ah = atlas.height as f32;
    let u_min = entry.rect.x as f32 / aw;
    let u_max = (entry.rect.x + entry.rect.w) as f32 / aw;
    let v_min = entry.rect.y as f32 / ah;
    let v_max = (entry.rect.y + entry.rect.h) as f32 / ah;

    // innerU = lerp(uMin, uMax, (w - borderRight) / w). The "lerp" is f32
    // (uMax - uMin) * t + uMin form to keep f32 ordering close to C#.
    let sprite_w = entry.rect.w as f32;
    let t = (sprite_w - entry.border.right as f32) / sprite_w;
    let inner_u = u_min + (u_max - u_min) * t;

    let x_l = x_min + edge_w;
    let x_r = x_max - edge_w;
    let centre_w = (x_r - x_l).max(0.0);

    // Centre-tile metrics. tile_scale = edgeW / borderRight (world / pixels);
    // tile_size_rect = (spriteW - borderRight) * tile_scale.
    let tile_scale = if entry.border.right > 0 {
        edge_w / (entry.border.right as f32 / ppu)
    } else {
        0.0
    };
    let tile_size_rect = (sprite_w - entry.border.right as f32) / ppu * tile_scale;

    let (tile_cols, tail_fract, has_tail) = if centre_w <= TAIL_EPSILON || tile_size_rect <= TAIL_EPSILON {
        (0usize, 0.0, false)
    } else {
        let tiles = centre_w / tile_size_rect;
        let full_tiles = tiles.floor() as usize;
        let tf = tiles - full_tiles as f32;
        let ht = tf > TAIL_EPSILON;
        // tileCols = fullTiles - 1 + (hasTail ? 1 : 0) per C#. Saturate at 0
        // when fullTiles is 0 to avoid underflow.
        let cols = full_tiles.saturating_sub(1) + if ht { 1 } else { 0 };
        (cols, tf, ht)
    };

    let total_cols = 4 + tile_cols;
    let mut verts: Vec<[f32; 2]> = Vec::with_capacity(total_cols * 2);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(total_cols * 2);

    // Helper: push a column (two verts, bottom + top, at the same x with same u).
    let push_col = |x: f32, u: f32, verts: &mut Vec<[f32; 2]>, uvs: &mut Vec<[f32; 2]>| {
        verts.push([x, y_min]);
        verts.push([x, y_max]);
        uvs.push([u, v_min]);
        uvs.push([u, v_max]);
    };

    // Col 0: left edge outer — mirrored, UV = uMax.
    push_col(x_min, u_max, &mut verts, &mut uvs);
    // Col 1: left edge inner — UV = innerU.
    push_col(x_l, inner_u, &mut verts, &mut uvs);

    // Centre tile columns: x advances; UV ping-pongs between uMin and innerU.
    let mut x = x_l;
    for i in 0..tile_cols {
        let is_last = i == tile_cols - 1 && has_tail;
        let delta = if is_last { tail_fract * tile_size_rect } else { tile_size_rect };
        x += delta;
        // Mirror C#: colIdx = i + 1 (1-based within centre); IsEven picks
        // forward vs back. forward → uMin (or partial sample), back → innerU
        // (or partial sample).
        let col_idx = i + 1;
        let flow_forward = col_idx % 2 == 0;
        let u = match (is_last, flow_forward) {
            (false, true)  => u_min,
            (false, false) => inner_u,
            (true,  true)  => sample_partial_uv(inner_u, u_min, tail_fract, true),
            (true,  false) => sample_partial_uv(u_min, inner_u, tail_fract, false),
        };
        push_col(x, u, &mut verts, &mut uvs);
    }

    // Col N-1: right edge inner.
    push_col(x_r, inner_u, &mut verts, &mut uvs);
    // Col N: right edge outer — UV = uMax (not mirrored on the right).
    push_col(x_max, u_max, &mut verts, &mut uvs);

    // Indices: simple linear quads.
    let quad_count = total_cols.saturating_sub(1);
    let mut tris: Vec<u16> = Vec::with_capacity(quad_count * 6);
    for i in 0..quad_count {
        let a = (i * 2) as u16;
        push_quad(&mut tris, a, a + 1, a + 2, a + 3);
    }

    // Apply affine.
    let verts: Vec<[f32; 2]> = verts.iter().map(|v| apply_transform(*v, ctx)).collect();

    PartMesh { verts, uvs, tris }
}

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
            verts.push(apply_transform([v[0] * sx, v[1] * sy], ctx));
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
// UISliceMeshGen.Identity (UISliceMethod=9): stretch the full source sprite
// to the target rect, anchored at part_pivot. No mirror, no border. Used by
// CSA "Color_*" 1×1 color sprites under UISlice to render solid-color bars.
//
// Formula (matches slice_mirror's per-copy math with one copy, sign=(1,1)):
//   v_local = v_src × scale + translation + offset
//     scale       = target / sprite_bound   (via slice=(1,1) full coverage)
//     translation = sprite_pivot × target   (shift sprite-pivot origin to
//                                            (sprite_pivot × target))
//     offset      = (mirror_pivot - rect_pivot) × target
//
// For Identity we want the sprite's pivot point to land at rect's pivot
// point — i.e., verts centered around (0, 0) when both pivots are (0.5,
// 0.5). Setting mirror_pivot = (0, 0) cancels the translation term:
//   v_local = v_src × scale + sprite_pivot × target - rect_pivot × target
//          = v_src × scale + (sprite_pivot - rect_pivot) × target
fn slice_identity(src: &SrcMesh, ctx: &SliceCtx, target_size: (f32, f32)) -> PartMesh {
    let x = slice_vertex_translation(
        target_size, ctx.part_pivot, ctx.sprite_pivot_norm, ctx.sprite_bound_size,
        (1.0, 1.0), (0.0, 0.0),
    );
    let verts: Vec<[f32; 2]> = src.verts.iter()
        .map(|v| {
            let p = [
                v[0] * x.scale.0 + x.translation.0 + x.offset.0,
                v[1] * x.scale.1 + x.translation.1 + x.offset.1,
            ];
            apply_transform(p, ctx)
        })
        .collect();
    PartMesh { verts, uvs: src.uvs.clone(), tris: src.tris.clone() }
}

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
            verts.push(apply_transform(p, ctx));
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
    resolve: F,
    atlas: AtlasSize,
    ppu: f32,
) -> Result<CombinedMesh, CombineError>
where
    // Returns (sprite_entry, invert_scale = 1 / .tps spriteScale) for each
    // referenced part name. Both pieces come from the same pipeline tier
    // (tpsheet + tps), so the caller can resolve them together.
    F: FnMut(&str) -> Option<(SpriteEntry, f32)>,
{
    build_combined_with_ranges(combined, resolve, atlas, ppu).map(|o| o.mesh)
}

/// Output of [`build_combined_with_ranges`]: the merged mesh plus the
/// `[start, end)` index range each input part occupies in the merged
/// `verts` / `uvs` arrays. The editor's preview canvas uses these ranges
/// for per-part picking, outlining, and vertex-color overrides without
/// re-running the build per part.
#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub mesh: CombinedMesh,
    /// Same length as `combined.parts`. `(start, end)` indexes into
    /// `mesh.verts` / `mesh.uvs`. The triangle list isn't split — callers
    /// filter `mesh.tris` by checking each triangle's first index against
    /// the range.
    pub part_ranges: Vec<(usize, usize)>,
}

/// Like [`build_combined`] but also returns per-part vertex-range info.
pub fn build_combined_with_ranges<F>(
    combined: &fab::Combined,
    mut resolve: F,
    atlas: AtlasSize,
    ppu: f32,
) -> Result<BuildOutput, CombineError>
where
    F: FnMut(&str) -> Option<(SpriteEntry, f32)>,
{
    let mut all_verts: Vec<[f32; 2]> = Vec::new();
    let mut all_uvs: Vec<[f32; 2]> = Vec::new();
    let mut all_tris: Vec<u16> = Vec::new();
    let mut aabb: Option<(u32, u32, u32, u32)> = None;
    let mut part_ranges: Vec<(usize, usize)> = Vec::with_capacity(combined.parts.len());

    for part in &combined.parts {
        let source_name = match part {
            Part::AtlasSprite { sprite, .. } => sprite,
            Part::Polygon { polygon_sprite, .. } => polygon_sprite,
        };
        let (entry, invert_scale) = resolve(source_name).ok_or_else(|| CombineError::SpriteNotFound {
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
            Part::AtlasSprite { method, size, part_pivot, border_mult, affine, offset, .. } => {
                check_method_constraints(*method, &entry, &combined.name, source_name)?;
                let resolved_pivot = part_pivot.unwrap_or([0.5, 0.5]);
                let effective_ppu = ppu / invert_scale;
                let resolved_size = if size.is_none() && method.requires_size() {
                    Some((entry.rect.w as f32 / effective_ppu, entry.rect.h as f32 / effective_ppu))
                } else {
                    *size
                };
                atlas_sprite_mesh(
                    &entry, *method, resolved_size, resolved_pivot, *border_mult, *affine,
                    atlas, ppu, invert_scale,
                    (offset[0], offset[1]),
                )
            }
            Part::Polygon { vertices, triangles, affine, offset, .. } => {
                polygon_mesh_with_tris(
                    vertices,
                    triangles.as_deref(),
                    *affine,
                    entry.rect,
                    atlas,
                    (offset[0], offset[1]),
                )
            }
        };

        let base = all_verts.len();
        let count = part_mesh.verts.len();
        all_verts.extend(part_mesh.verts);
        all_uvs.extend(part_mesh.uvs);
        all_tris.extend(part_mesh.tris.iter().map(|i| i + base as u16));
        part_ranges.push((base, base + count));
    }

    let (minx, miny, maxx, maxy) = aabb.expect("fab::Combined.parts is non-empty (parse-time)");
    Ok(BuildOutput {
        mesh: CombinedMesh {
            verts: all_verts,
            uvs: all_uvs,
            tris: all_tris,
            atlas_rect: Rect { x: minx, y: miny, w: maxx - minx, h: maxy - miny },
        },
        part_ranges,
    })
}

#[cfg(test)]
fn apply_affine(v: [f32; 2], a: Affine) -> [f32; 2] {
    // T · R · S applied to v: scale → rotate → translate.
    let sx = v[0] * a.sx;
    let sy = v[1] * a.sy;
    let (rs, rc) = a.rot_deg_ccw.to_radians().sin_cos();
    let rx = sx * rc - sy * rs;
    let ry = sx * rs + sy * rc;
    [rx + a.tx, ry + a.ty]
}

// Per-vert transform: per-part affine (scale + rotate) followed by the
// pre-scaled world-unit offset, then the affine translate.
//
// The CSA-era `× ui_scale × canvas_scale` two-multiply chain is split:
//   - `ui_scale` is now part of each node's `scale` (composed by the walker
//     into `affine.sx/sy`).
//   - `canvas_scale` is pre-multiplied into the leaf's `offset` and (for
//     size-fitted methods) `size` at the bridge layer
//     (`Output::canvas_scale_implicit`).
// Each vertex therefore traverses one multiply (scale), one optional rotate,
// and one add (offset + translate). For SpriteRenderer / Box prefabs
// `offset = (0, 0)` and `affine = identity` collapses this to passthrough.
fn apply_transform(v: [f32; 2], ctx: &SliceCtx) -> [f32; 2] {
    let a = ctx.affine;
    // 1. scale (per-part sx, sy) — magnitude carries `old uiScale × old canvasScale`.
    let mut x = v[0] * a.sx;
    let mut y = v[1] * a.sy;
    // 2. rotate.
    if a.rot_deg_ccw != 0.0 {
        let (rs, rc) = a.rot_deg_ccw.to_radians().sin_cos();
        let nx = x * rc - y * rs;
        let ny = x * rs + y * rc;
        x = nx; y = ny;
    }
    // 3. + offset (world-unit; bridge pre-applied mode-implicit canvas factor).
    x += ctx.offset.0;
    y += ctx.offset.1;
    // 4. + (tx, ty) world-unit affine translate.
    [x + a.tx, y + a.ty]
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
    fn polygon_setup_quad_layout_uses_canonical_indices() {
        // UISolid stamps 4 verts in `MeshBuilder.SetUp_Quad` order:
        //   [0]=BL, [1]=BR, [2]=TL, [3]=TR — a bowtie ring that ear-clip
        // sees as zero-area and returns no triangles for. Without the
        // override, fab.json color quads would render as nothing. The
        // polygon_mesh path detects this layout and emits the canonical
        // `QuadIndexCache.Single` indices `[0, 2, 3, 3, 1, 0]`.
        let verts = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
        let m = polygon_mesh(&verts, Affine::default(), rect(0, 0, 2, 2), ATLAS);
        assert_eq!(m.tris, vec![0, 2, 3, 3, 1, 0]);
    }

    #[test]
    fn polygon_ring_ordered_quad_still_ear_clips() {
        // CCW ring [BL, BR, TR, TL] is a non-degenerate polygon — the
        // SetUp_Quad detector skips it (vert[2].x != vert[0].x) and the
        // ear-clip triangulator handles it normally.
        let verts = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let m = polygon_mesh(&verts, Affine::default(), rect(0, 0, 2, 2), ATLAS);
        assert_eq!(m.tris.len(), 6, "ring-ordered quad: 2 triangles");
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
        let a = Affine { rot_deg_ccw: 90.0, ..Affine::default() };
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
            &entry, Method::Id, None, [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 100, height: 100 }, 100.0, 1.0, (0.0, 0.0),
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
            &entry, Method::Id, None, [0.5, 0.5], 1.0,
            Affine { tx: 5.0, ty: 7.0, ..Affine::default() },
            AtlasSize { width: 16, height: 16 },
            1.0, 1.0, (0.0, 0.0),
        );
        // pivot pixel = (1, 1). local verts pre-translate:
        //   (-1, -1), (1, -1), (-1, 1), (1, 1).
        // After translate:
        assert_eq!(m.verts, vec![
            [4.0, 6.0], [6.0, 6.0], [4.0, 8.0], [6.0, 8.0],
        ]);
    }

    // --- mirror methods (MX / MY / MXY) ---

    #[test]
    fn mx_doubles_verts_and_indices() {
        // 2×2 sprite, PPU 1, pivot center. Target rect 4×2 → MX produces two
        // 2×2 halves sitting side-by-side, target rect centered at origin.
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mx, Some((4.0, 2.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
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
            &entry, Method::Mx, Some((4.0, 2.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
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
            &entry, Method::Mx, Some((4.0, 2.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 100, height: 100 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.uvs[..4], m.uvs[4..]);
    }

    #[test]
    fn my_doubles_along_y() {
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::My, Some((2.0, 4.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
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
            &entry, Method::Mxy, Some((4.0, 4.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
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

    // --- calc_rect_and_pivot ---

    #[test]
    fn calc_rect_and_pivot_centered_unit_square() {
        // Verts (-0.5, -0.5) to (0.5, 0.5). PPU 100 ⇒ rect 100×100, pivot (0.5, 0.5).
        let verts = [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]];
        let ((w, h), (px, py)) = calc_rect_and_pivot(&verts, 100.0);
        assert_eq!(w, 100.0);
        assert_eq!(h, 100.0);
        assert!((px - 0.5).abs() < 1e-6, "{px}");
        assert!((py - 0.5).abs() < 1e-6, "{py}");
    }

    #[test]
    fn calc_rect_and_pivot_origin_at_min_corner() {
        // Verts (0, 0) to (2, 4). PPU 100 ⇒ rect 200×400, pivot (0, 0).
        let verts = [[0.0, 0.0], [2.0, 0.0], [0.0, 4.0], [2.0, 4.0]];
        let ((w, h), (px, py)) = calc_rect_and_pivot(&verts, 100.0);
        assert_eq!(w, 200.0);
        assert_eq!(h, 400.0);
        assert_eq!(px, 0.0);
        assert_eq!(py, 0.0);
    }

    #[test]
    fn calc_rect_and_pivot_off_center_y() {
        // Verts span X [-1.4125, 1.4125] (centered), Y [-3.1225, 4.5775]
        // (offset). PPU 100. Matches Silloutte1.asset's m_Rect (282.5, 770)
        // and m_Pivot (0.5, 0.40551946) within f32 precision.
        let verts = [
            [-1.4125, -3.1225], [1.4125, -3.1225],
            [-1.4125, 4.5775],  [1.4125, 4.5775],
        ];
        let ((w, h), (px, py)) = calc_rect_and_pivot(&verts, 100.0);
        assert_eq!(w, 282.5);
        assert_eq!(h, 770.0);
        assert!((px - 0.5).abs() < 1e-6);
        // 3.1225 / 7.7 = 0.40551946...
        assert!((py - 0.40551946).abs() < 1e-6, "got {py}");
    }

    // --- tiling ---

    #[test]
    fn calc_tile_layout_exact_2_tiles_per_side_no_tail() {
        // rect_w = 4, tile_w = 1 ⇒ tiles_per_side = 4/1 * 0.5 = 2. No tail.
        let l = calc_tile_layout(1.0, 4.0);
        assert!(!l.has_tail);
        assert_eq!(l.cols_half, 2);
        assert_eq!(l.vert_half, 5);  // 2 cols * 2 + 1 (centre)
        assert_eq!(l.vert_count, 10);
    }

    #[test]
    fn calc_tile_layout_with_partial_tail() {
        // rect_w = 5, tile_w = 1 ⇒ tiles_per_side = 2.5 ⇒ 2 full + 1 partial tail.
        let l = calc_tile_layout(1.0, 5.0);
        assert!(l.has_tail);
        assert!((l.tail_fract - 0.5).abs() < 1e-5);
        assert_eq!(l.cols_half, 3);
        assert_eq!(l.vert_half, 7);  // 3 cols * 2 + 1
        assert_eq!(l.vert_count, 14);
    }

    #[test]
    fn tx_basic_two_tiles_each_side_no_tail() {
        // 1×1 sprite at atlas (0,0,1,1), PPU 1. Target rect 4×2 ⇒ 2 tiles per
        // side ⇒ vert_count = 10 (5 vert pairs along X).
        let entry = quad_entry(0, 0, 1, 1, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Tx, Some((4.0, 2.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 10, "5 vert pairs for cols_half=2");
        // BuildStrapIndices: 2 forward + 1 centre-seam + 1 backward = 4 quads
        // (the backward loop starts at i=1, so cols_half=2 yields just 1 backward
        // quad). 4 quads * 6 indices = 24.
        assert_eq!(m.tris.len(), 24);
        // Verts span the full target rect on X (-2..2) at Y bounds (-1..1).
        let xs: Vec<f32> = m.verts.iter().map(|v| v[0]).collect();
        let ys: Vec<f32> = m.verts.iter().map(|v| v[1]).collect();
        assert!((xs.iter().copied().fold(f32::INFINITY, f32::min) + 2.0).abs() < 1e-5);
        assert!((xs.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 2.0).abs() < 1e-5);
        assert!((ys.iter().copied().fold(f32::INFINITY, f32::min) + 1.0).abs() < 1e-5);
        assert!((ys.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn ty_mirrors_tx_along_y_axis() {
        // 1×1 sprite, target 2×4. Should produce the same shape rotated 90°
        // (5 vert pairs along Y).
        let entry = quad_entry(0, 0, 1, 1, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Ty, Some((2.0, 4.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 10);
        assert_eq!(m.tris.len(), 24);
        let xs: Vec<f32> = m.verts.iter().map(|v| v[0]).collect();
        let ys: Vec<f32> = m.verts.iter().map(|v| v[1]).collect();
        assert!((xs.iter().copied().fold(f32::INFINITY, f32::min) + 1.0).abs() < 1e-5);
        assert!((xs.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 1.0).abs() < 1e-5);
        assert!((ys.iter().copied().fold(f32::INFINITY, f32::min) + 2.0).abs() < 1e-5);
        assert!((ys.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn tx_centre_seam_uv_is_atlas_u_min() {
        // The centre seam (verts 0, 1) should sample uMin of the source rect.
        let entry = quad_entry(10, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Tx, Some((4.0, 2.0)), [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 100, height: 100 }, 1.0, 1.0, (0.0, 0.0),
        );
        // u_min = rect.x / atlas_w = 10 / 100 = 0.1.
        assert!((m.uvs[0][0] - 0.1).abs() < 1e-6);
        assert!((m.uvs[1][0] - 0.1).abs() < 1e-6);
    }

    // --- TX_MC3 ---

    fn quad_entry_with_border(
        rx: u32, ry: u32, w: u32, h: u32,
        pivot: (f32, f32),
        border_left: i32, border_right: i32,
    ) -> SpriteEntry {
        let mut e = quad_entry(rx, ry, w, h, pivot);
        e.border.left = border_left;
        e.border.right = border_right;
        e
    }

    #[test]
    fn tx_mc3_validates_border_left_zero() {
        // Left border != 0 → SliceConstraint error.
        let entry = quad_entry_with_border(0, 0, 10, 4, (0.5, 0.5), 2, 2);
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::TxMc3,
            size: Some((6.0, 4.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(
            &combined, |_| Some((entry.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0,
        ).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { method: Method::TxMc3, .. }), "{err:?}");
    }

    #[test]
    fn tx_mc3_validates_border_right_positive() {
        let entry = quad_entry_with_border(0, 0, 10, 4, (0.5, 0.5), 0, 0);
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::TxMc3,
            size: Some((6.0, 4.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(
            &combined, |_| Some((entry.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0,
        ).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { .. }), "{err:?}");
    }

    #[test]
    fn tx_mc3_emits_4_columns_when_centre_collapses() {
        // 4×4 sprite with border.right = 4 (full width) ⇒ tile_size_rect = 0
        // ⇒ no centre tiles. Total cols = 4 (leftOuter, leftInner, rightInner,
        // rightOuter) ⇒ 8 verts, 3 quads ⇒ 18 indices.
        let entry = quad_entry_with_border(0, 0, 4, 4, (0.5, 0.5), 0, 4);
        let m = atlas_sprite_mesh(
            &entry, Method::TxMc3, Some((8.0, 4.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 8);
        assert_eq!(m.tris.len(), 18);
        // Bounds: target rect 8×4 centred on origin → X ∈ [-4, 4], Y ∈ [-2, 2].
        let xs: Vec<f32> = m.verts.iter().map(|v| v[0]).collect();
        assert!((xs.iter().copied().fold(f32::INFINITY, f32::min) + 4.0).abs() < 1e-5);
        assert!((xs.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 4.0).abs() < 1e-5);
    }

    #[test]
    fn tx_mc3_left_edge_mirrors_uv_to_u_max() {
        // Col 0 should sample u_max (mirrored); col 1 samples inner_u; col N
        // samples u_max (right edge outer).
        let entry = quad_entry_with_border(0, 0, 4, 4, (0.5, 0.5), 0, 4);
        let m = atlas_sprite_mesh(
            &entry, Method::TxMc3, Some((8.0, 4.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        // u_max = (0 + 4) / 16 = 0.25. Verts 0/1 are col 0 (left outer).
        assert!((m.uvs[0][0] - 0.25).abs() < 1e-6, "{:?}", m.uvs[0]);
        // Verts 6/7 are col 3 (right outer).
        assert!((m.uvs[6][0] - 0.25).abs() < 1e-6);
        // Verts 2/3 are col 1 (left inner). inner_u = lerp(uMin, uMax, 0/4) = uMin = 0.
        assert!((m.uvs[2][0]).abs() < 1e-6);
    }

    // --- slice grids: R1C3 / R3C3 / R3C3_NF ---

    #[test]
    fn r1c3_emits_8_verts_18_indices() {
        // 4×4 sprite, border (2, 0, 2, 0) ⇒ borderSumX = 4 = rect.w. Target 8×2.
        let mut entry = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        entry.border = crate::tpsheet::Border { left: 2, bottom: 0, right: 2, top: 0 };
        let m = atlas_sprite_mesh(
            &entry, Method::R1c3, Some((8.0, 2.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 8);
        assert_eq!(m.tris.len(), 18, "3 quads × 6 indices");
    }

    #[test]
    fn r1c3_centre_column_shares_u_coordinate() {
        let mut entry = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        entry.border = crate::tpsheet::Border { left: 2, bottom: 0, right: 2, top: 0 };
        let m = atlas_sprite_mesh(
            &entry, Method::R1c3, Some((8.0, 2.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        // Verts 1 and 2 (inner cols on bottom row) share U.
        assert_eq!(m.uvs[1][0], m.uvs[2][0]);
        // Same for verts 5 and 6 on top row.
        assert_eq!(m.uvs[5][0], m.uvs[6][0]);
    }

    #[test]
    fn r1c3_rejects_border_sum_mismatch() {
        // border.left + border.right != rect.w ⇒ SliceConstraint.
        let mut entry = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        entry.border = crate::tpsheet::Border { left: 1, bottom: 0, right: 2, top: 0 };
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::R1c3,
            size: Some((8.0, 2.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(
            &combined, |_| Some((entry.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0,
        ).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { method: Method::R1c3, .. }), "{err:?}");
    }

    // --- slice grids: MXY_R3C3 / MXY_R3C3_NF ---

    fn mxy_satisfying_entry() -> SpriteEntry {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 2, top: 2 };
        e
    }

    #[test]
    fn mxy_r3c3_rejects_constraint_violations() {
        // bottom != 0 → SliceConstraint.
        let mut e = mxy_satisfying_entry();
        e.border.bottom = 1;
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::MxyR3c3,
            size: Some((8.0, 8.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(&combined, |_| Some((e.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { method: Method::MxyR3c3, .. }));
    }

    #[test]
    fn mxy_r3c3_emits_16_verts_54_indices() {
        let e = mxy_satisfying_entry();
        let m = atlas_sprite_mesh(
            &e, Method::MxyR3c3, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 16);
        assert_eq!(m.tris.len(), 54);
    }

    #[test]
    fn mxy_r3c3_nf_omits_centre_quad() {
        let e = mxy_satisfying_entry();
        let m = atlas_sprite_mesh(
            &e, Method::MxyR3c3Nf, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.tris.len(), 48, "8 quads × 6 indices");
    }

    #[test]
    fn mxy_r3c3_uvs_use_only_inner_max_and_outer_max() {
        // Per SetUp_MXY_R3C3: every UV is one of (u2|u3, v2|v3).
        let e = mxy_satisfying_entry();
        let m = atlas_sprite_mesh(
            &e, Method::MxyR3c3, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        let grid = UvGrid::for_entry(&e, AtlasSize { width: 16, height: 16 }, 1.0);
        for uv in &m.uvs {
            assert!(uv[0] == grid.u[2] || uv[0] == grid.u[3],
                "U must be inner-max or outer-max, got {uv:?}");
            assert!(uv[1] == grid.v[2] || uv[1] == grid.v[3],
                "V must be inner-max or outer-max, got {uv:?}");
        }
        // Outer corners (0, 3, 12, 15) must hit (u3, v3).
        for &i in &[0, 3, 12, 15] {
            assert_eq!(m.uvs[i], [grid.u[3], grid.v[3]], "outer corner {i}");
        }
        // Inner corners (5, 6, 9, 10) must hit (u2, v2).
        for &i in &[5, 6, 9, 10] {
            assert_eq!(m.uvs[i], [grid.u[2], grid.v[2]], "inner corner {i}");
        }
    }

    // --- slice grids: MX_R1C4 / MX_R3C2 / MX_R3C4 / MX_R3C6 ---

    #[test]
    fn mx_r1c4_emits_10_verts_24_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 2, top: 0 };
        let m = atlas_sprite_mesh(
            &e, Method::MxR1c4, Some((10.0, 4.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 10);
        assert_eq!(m.tris.len(), 24, "4 quads × 6 indices");
    }

    #[test]
    fn mx_r1c4_uv_mirrors_about_centre() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 2, top: 0 };
        let m = atlas_sprite_mesh(
            &e, Method::MxR1c4, Some((10.0, 4.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        // Bottom row: u3, u2, u0, u2, u3 (mirror around col index 2).
        let grid = UvGrid::for_entry(&e, AtlasSize { width: 16, height: 16 }, 1.0);
        let expected_u = [grid.u[3], grid.u[2], grid.u[0], grid.u[2], grid.u[3]];
        for (col, &want) in expected_u.iter().enumerate() {
            assert_eq!(m.uvs[col][0], want);
            assert_eq!(m.uvs[5 + col][0], want);
        }
    }

    #[test]
    fn mx_r3c2_emits_12_verts_36_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 1, right: 4, top: 3 };
        let m = atlas_sprite_mesh(
            &e, Method::MxR3c2, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 12);
        assert_eq!(m.tris.len(), 36);
    }

    #[test]
    fn mx_r3c4_emits_20_verts_72_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 1, right: 2, top: 1 };
        let m = atlas_sprite_mesh(
            &e, Method::MxR3c4, Some((10.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 20);
        assert_eq!(m.tris.len(), 72, "12 quads × 6 indices");
    }

    #[test]
    fn mx_r3c6_emits_28_verts_108_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 1, bottom: 1, right: 1, top: 1 };
        let m = atlas_sprite_mesh(
            &e, Method::MxR3c6, Some((10.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 28);
        assert_eq!(m.tris.len(), 108, "18 quads × 6 indices");
    }

    #[test]
    fn mx_r1c4_rejects_nonzero_left() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 1, bottom: 0, right: 2, top: 0 };
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::MxR1c4,
            size: Some((10.0, 4.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(&combined, |_| Some((e.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { method: Method::MxR1c4, .. }));
    }

    // --- slice grids: MY_R3C1 / MY_R2C2 / MY_R2C3 / MY_R3C2 ---

    #[test]
    fn my_r3c1_emits_8_verts_18_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 0, top: 4 };
        let m = atlas_sprite_mesh(
            &e, Method::MyR3c1, Some((4.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 8);
        assert_eq!(m.tris.len(), 18);
    }

    #[test]
    fn my_r3c1_uv_rows_mirror() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 0, top: 4 };
        let m = atlas_sprite_mesh(
            &e, Method::MyR3c1, Some((4.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        let grid = UvGrid::for_entry(&e, AtlasSize { width: 16, height: 16 }, 1.0);
        // Outer rows (0/3) sample v3; inner rows (1/2) sample v0.
        for col_pair in 0..2 {
            assert_eq!(m.uvs[col_pair][1], grid.v[3]);   // row 0
            assert_eq!(m.uvs[2 + col_pair][1], grid.v[0]); // row 1
            assert_eq!(m.uvs[4 + col_pair][1], grid.v[0]); // row 2
            assert_eq!(m.uvs[6 + col_pair][1], grid.v[3]); // row 3
        }
    }

    #[test]
    fn my_r2c2_emits_9_verts_24_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 4, top: 4 };
        let m = atlas_sprite_mesh(
            &e, Method::MyR2c2, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 9);
        assert_eq!(m.tris.len(), 24, "4 quads × 6 indices");
    }

    #[test]
    fn my_r2c2_col0_col1_share_u_col2_uses_u3() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 4, top: 4 };
        let m = atlas_sprite_mesh(
            &e, Method::MyR2c2, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        let grid = UvGrid::for_entry(&e, AtlasSize { width: 16, height: 16 }, 1.0);
        for row in 0..3 {
            let base = row * 3;
            assert_eq!(m.uvs[base][0], grid.u[0]);
            assert_eq!(m.uvs[base + 1][0], grid.u[0]);
            assert_eq!(m.uvs[base + 2][0], grid.u[3]);
        }
    }

    #[test]
    fn my_r2c3_emits_12_verts_36_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 1, bottom: 0, right: 3, top: 4 };
        let m = atlas_sprite_mesh(
            &e, Method::MyR2c3, Some((8.0, 4.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 12);
        assert_eq!(m.tris.len(), 36, "6 quads × 6 indices");
    }

    #[test]
    fn my_r3c2_emits_12_verts_36_indices() {
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: 4, top: 4 };
        let m = atlas_sprite_mesh(
            &e, Method::MyR3c2, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 12);
        assert_eq!(m.tris.len(), 36);
    }

    #[test]
    fn my_constraints_surface_at_build_time() {
        // MY_R3C1: requires left == 0 AND right == 0.
        let mut e = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 1, bottom: 0, right: 0, top: 4 };
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::MyR3c1,
            size: Some((4.0, 8.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(&combined, |_| Some((e.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { method: Method::MyR3c1, .. }));
    }

    // --- slice grids: MY_R3C3 ---

    fn my_satisfying_entry(rect_h: u32) -> SpriteEntry {
        let mut e = quad_entry(0, 0, 4, rect_h, (0.5, 0.5));
        e.border = crate::tpsheet::Border {
            left: 1, bottom: 0, right: 1, top: rect_h as i32,
        };
        e
    }

    #[test]
    fn my_r3c3_rejects_bottom_border_nonzero() {
        let mut e = my_satisfying_entry(4);
        e.border.bottom = 1;
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::MyR3c3,
            size: Some((8.0, 8.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(&combined, |_| Some((e.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { method: Method::MyR3c3, .. }));
    }

    #[test]
    fn my_r3c3_emits_16_verts_54_indices() {
        let e = my_satisfying_entry(4);
        let m = atlas_sprite_mesh(
            &e, Method::MyR3c3, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 16);
        assert_eq!(m.tris.len(), 54);
    }

    #[test]
    fn my_r3c3_uv_rows_mirror_about_centre() {
        let e = my_satisfying_entry(4);
        let m = atlas_sprite_mesh(
            &e, Method::MyR3c3, Some((8.0, 8.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        let grid = UvGrid::for_entry(&e, AtlasSize { width: 16, height: 16 }, 1.0);
        // Row 0 (bottom-of-target) samples v3 (atlas-top); same for row 3 (top).
        for col in 0..4 {
            assert_eq!(m.uvs[col][1], grid.v[3], "row 0 col {col}");
            assert_eq!(m.uvs[12 + col][1], grid.v[3]);
        }
        // Rows 1 and 2 sample v0 (atlas-bottom).
        for col in 0..4 {
            assert_eq!(m.uvs[4 + col][1], grid.v[0]);
            assert_eq!(m.uvs[8 + col][1], grid.v[0]);
        }
        // U progresses u0..u3 within each row.
        for row in 0..4 {
            for col in 0..4 {
                assert_eq!(m.uvs[row * 4 + col][0], grid.u[col], "row {row} col {col}");
            }
        }
    }

    // --- slice grids: MX_R1C3 / MX_R3C3 ---

    fn mx_satisfying_entry(rect_w: u32) -> SpriteEntry {
        let mut e = quad_entry(0, 0, rect_w, 4, (0.5, 0.5));
        e.border = crate::tpsheet::Border { left: 0, bottom: 0, right: rect_w as i32, top: 0 };
        e
    }

    #[test]
    fn mx_r1c3_rejects_left_border_nonzero() {
        let mut e = mx_satisfying_entry(4);
        e.border.left = 1;
        let combined = make_combined("BX", vec![Part::AtlasSprite {
            sprite: "A".into(), method: Method::MxR1c3,
            size: Some((8.0, 2.0)), part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0, affine: Affine::default(),
            offset: [0.0, 0.0],
        }]);
        let err = build_combined(&combined, |_| Some((e.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 }, 1.0).unwrap_err();
        assert!(matches!(err, CombineError::SliceConstraint { method: Method::MxR1c3, .. }));
    }

    #[test]
    fn mx_r1c3_outer_cols_sample_u3_inner_cols_sample_u0() {
        let e = mx_satisfying_entry(4);
        let m = atlas_sprite_mesh(
            &e, Method::MxR1c3, Some((8.0, 2.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        let grid = UvGrid::for_entry(&e, AtlasSize { width: 16, height: 16 }, 1.0);
        // Bottom row: outer-cols 0,3 → u3; inner-cols 1,2 → u0.
        assert_eq!(m.uvs[0][0], grid.u[3]);
        assert_eq!(m.uvs[1][0], grid.u[0]);
        assert_eq!(m.uvs[2][0], grid.u[0]);
        assert_eq!(m.uvs[3][0], grid.u[3]);
        // Top row mirrors with v[3].
        assert_eq!(m.uvs[4][0], grid.u[3]);
        assert_eq!(m.uvs[7][0], grid.u[3]);
    }

    #[test]
    fn mx_r3c3_emits_16_verts_54_indices() {
        let e = mx_satisfying_entry(4);
        let m = atlas_sprite_mesh(
            &e, Method::MxR3c3, Some((8.0, 4.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 16);
        assert_eq!(m.tris.len(), 54);
    }

    #[test]
    fn mx_r3c3_uv_pattern_outer_u3_inner_u0_per_row() {
        let e = mx_satisfying_entry(4);
        let m = atlas_sprite_mesh(
            &e, Method::MxR3c3, Some((8.0, 4.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        let grid = UvGrid::for_entry(&e, AtlasSize { width: 16, height: 16 }, 1.0);
        // Each row's UVs: outer cols → u3, inner cols → u0.
        for row in 0..4 {
            let base = row * 4;
            assert_eq!(m.uvs[base][0], grid.u[3], "row {row} col 0");
            assert_eq!(m.uvs[base + 1][0], grid.u[0]);
            assert_eq!(m.uvs[base + 2][0], grid.u[0]);
            assert_eq!(m.uvs[base + 3][0], grid.u[3]);
            // V coord picks grid.v[row] for every col.
            for col in 0..4 {
                assert_eq!(m.uvs[base + col][1], grid.v[row]);
            }
        }
    }

    // --- slice grids: R3C3 / R3C3_NF ---

    #[test]
    fn r3c3_emits_16_verts_and_54_indices() {
        // 8×8 sprite with border (1,1,1,1), target 16×16. Centred at origin
        // (part_pivot 0.5, 0.5).
        let entry = quad_entry_with_border(0, 0, 8, 8, (0.5, 0.5), 1, 1);
        // Override bottom/top borders too via direct manipulation.
        let entry = SpriteEntry {
            border: crate::tpsheet::Border { left: 1, bottom: 1, right: 1, top: 1 },
            ..entry
        };
        let m = atlas_sprite_mesh(
            &entry, Method::R3c3, Some((16.0, 16.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 16, "4×4 grid");
        // 3×3 = 9 quads, 54 indices.
        assert_eq!(m.tris.len(), 54);
        assert_eq!(m.uvs.len(), 16);
    }

    #[test]
    fn r3c3_corner_verts_form_target_rect_bounds() {
        let entry = quad_entry_with_border(0, 0, 8, 8, (0.5, 0.5), 1, 1);
        let entry = SpriteEntry {
            border: crate::tpsheet::Border { left: 1, bottom: 1, right: 1, top: 1 },
            ..entry
        };
        let m = atlas_sprite_mesh(
            &entry, Method::R3c3, Some((16.0, 16.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        // Bottom-left corner of grid (index 0) = target rect BL = (-8, -8).
        assert_eq!(m.verts[0], [-8.0, -8.0]);
        // Top-right corner (index 15) = (+8, +8).
        assert_eq!(m.verts[15], [8.0, 8.0]);
        // Bottom-left inner border vert (index 5) at (-8 + bL, -8 + bB)
        // with bL/bB = 1 (border) / 1 (ppu) * 1 (borderMult) = 1.
        assert_eq!(m.verts[5], [-7.0, -7.0]);
        assert_eq!(m.verts[10], [7.0, 7.0]);
    }

    #[test]
    fn r3c3_uvs_form_outer_inner_grid() {
        // 8×8 sprite at atlas (0, 0), border (1,1,1,1), atlas 16×16.
        // Outer UV: (0..8/16) = (0..0.5). Inner: ((0+1)/16, (8-1)/16) = (0.0625, 0.4375).
        let entry = quad_entry_with_border(0, 0, 8, 8, (0.5, 0.5), 1, 1);
        let entry = SpriteEntry {
            border: crate::tpsheet::Border { left: 1, bottom: 1, right: 1, top: 1 },
            ..entry
        };
        let m = atlas_sprite_mesh(
            &entry, Method::R3c3, Some((16.0, 16.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        // (col=0, row=0) outer-min, outer-min
        assert_eq!(m.uvs[0], [0.0, 0.0]);
        // (col=3, row=3) outer-max, outer-max
        assert_eq!(m.uvs[15], [0.5, 0.5]);
        // (col=1, row=1) inner-min, inner-min
        assert_eq!(m.uvs[5], [0.0625, 0.0625]);
    }

    #[test]
    fn r3c3_nf_omits_centre_quad() {
        let entry = quad_entry_with_border(0, 0, 8, 8, (0.5, 0.5), 1, 1);
        let entry = SpriteEntry {
            border: crate::tpsheet::Border { left: 1, bottom: 1, right: 1, top: 1 },
            ..entry
        };
        let m = atlas_sprite_mesh(
            &entry, Method::R3c3Nf, Some((16.0, 16.0)), [0.5, 0.5], 1.0,
            Affine::default(), AtlasSize { width: 16, height: 16 }, 1.0, 1.0, (0.0, 0.0),
        );
        // 8 quads * 6 indices = 48 (vs R3C3's 54).
        assert_eq!(m.tris.len(), 48);
        // None of the emitted triangles cover the centre verts 5/6/9/10 in
        // the {5,6,10,9} winding — those would be the centre quad's tri pair.
        // Easier check: the centre quad's vertex pair (5,6) doesn't appear as
        // an edge in any triangle.
        let mut centre_pair_seen = false;
        for tri in m.tris.chunks(3) {
            let set: std::collections::HashSet<u16> = tri.iter().copied().collect();
            if set.contains(&5) && set.contains(&6) && set.contains(&10) {
                centre_pair_seen = true; break;
            }
        }
        assert!(!centre_pair_seen, "NF should omit the {{5,6,10}} centre triangle");
    }

    #[test]
    fn icon_mx_duplicates_with_native_origin_mirror() {
        // 2×2 sprite at pivot (0.5, 0.5), PPU 1, no target size.
        // Source verts (pivot-relative): (-1,-1), (1,-1), (-1,1), (1,1).
        // First copy: same. Second copy: X-flipped about origin (0,0).
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mx, None, [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
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
            &entry, Method::My, None, [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 8);
        assert_eq!(m.verts[4..], [[-1.0, 1.0], [1.0, 1.0], [-1.0, -1.0], [1.0, -1.0]]);
    }

    #[test]
    fn icon_mxy_quadruples_about_origin() {
        let entry = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let m = atlas_sprite_mesh(
            &entry, Method::Mxy, None, [0.5, 0.5], 1.0, Affine::default(),
            AtlasSize { width: 8, height: 8 }, 1.0, 1.0, (0.0, 0.0),
        );
        assert_eq!(m.verts.len(), 16);
        assert_eq!(m.tris.len(), 24);
    }

    // --- multi-part combine ---

    fn make_combined(name: &str, parts: Vec<Part>) -> fab::Combined {
        fab::Combined {
            name: name.into(), pivot: [0.5, 0.5], border: [0.0; 4],
            parts,
        }
    }

    fn id_part(sprite: &str, affine: Affine) -> Part {
        Part::AtlasSprite {
            sprite: sprite.into(),
            method: Method::Id,
            size: None,
            part_pivot: Some([0.5, 0.5]),
            border_mult: 1.0,
            affine,
            offset: [0.0, 0.0],
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
            |n| match n { "A" => Some((a.clone(), 1.0)), "B" => Some((b.clone(), 1.0)), _ => None },
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
            |n| match n { "A" => Some((a.clone(), 1.0)), "B" => Some((b.clone(), 1.0)), _ => None },
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
            |_| Some((a.clone(), 1.0)),
            AtlasSize { width: 16, height: 16 },
            1.0,
        ).unwrap();
        // First part's verts get tx=100; second part's stay near origin.
        // Confirms parts are NOT deduped by source name.
        assert!(m.verts[0][0] > 50.0, "{:?}", m.verts);
        assert!(m.verts[4][0] < 5.0, "{:?}", m.verts);
    }

    #[test]
    fn build_with_ranges_partitions_verts_per_part() {
        // Two ID quads, 4 verts each. Ranges should be (0..4) and (4..8).
        let a = quad_entry(0, 0, 2, 2, (0.5, 0.5));
        let b = quad_entry(4, 4, 2, 2, (0.5, 0.5));
        let combined = make_combined("BX", vec![
            id_part("A", Affine::default()),
            id_part("B", Affine { tx: 10.0, ..Affine::default() }),
        ]);
        let out = build_combined_with_ranges(
            &combined,
            |n| match n { "A" => Some((a.clone(), 1.0)), "B" => Some((b.clone(), 1.0)), _ => None },
            AtlasSize { width: 16, height: 16 },
            1.0,
        ).unwrap();
        assert_eq!(out.part_ranges, vec![(0, 4), (4, 8)]);
        assert_eq!(out.mesh.verts.len(), 8);
    }

    #[test]
    fn build_with_ranges_handles_zero_vert_part() {
        // Triangulator returns no triangles for a degenerate polygon; verts
        // still flow through. The contract is: range length always equals
        // the part's vert count.
        let a = quad_entry(0, 0, 4, 4, (0.5, 0.5));
        let combined = make_combined("BX", vec![
            id_part("A", Affine::default()),
            id_part("A", Affine { tx: 5.0, ..Affine::default() }),
            id_part("A", Affine { tx: 10.0, ..Affine::default() }),
        ]);
        let out = build_combined_with_ranges(
            &combined,
            |_| Some((a.clone(), 1.0)),
            AtlasSize { width: 64, height: 64 },
            1.0,
        ).unwrap();
        // 3 quad parts, 4 verts each → contiguous ranges.
        assert_eq!(out.part_ranges, vec![(0, 4), (4, 8), (8, 12)]);
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
    fn polygon_combined_transform_order_is_t_after_rs() {
        // Order: scale, then rotate, then translate. Check that (1,0)
        // under (sx=2, rot=90, tx=10, ty=0) lands at (10, 2).
        //   scale  → (2, 0)
        //   rotate → (0, 2)
        //   trans  → (10, 2)
        let a = Affine { tx: 10.0, ty: 0.0, sx: 2.0, sy: 1.0, rot_deg_ccw: 90.0 };
        let out = apply_affine([1.0, 0.0], a);
        assert!((out[0] - 10.0).abs() < 1e-5, "{out:?}");
        assert!((out[1] - 2.0).abs() < 1e-5, "{out:?}");
    }

    // --- apply_transform op-order regression guards ---

    // Builds a SliceCtx for apply_transform-only tests; the slice/mirror
    // fields are unread on that code path so they get filler values.
    fn ctx_for_transform(affine: Affine, offset: (f32, f32)) -> SliceCtx {
        SliceCtx {
            sprite_pivot_norm: (0.5, 0.5),
            sprite_bound_size: (1.0, 1.0),
            part_pivot: (0.5, 0.5),
            border_mult: 1.0,
            affine,
            offset,
        }
    }

    #[test]
    fn apply_transform_identity_offset_collapses_to_apply_affine() {
        // offset=(0,0) collapses apply_transform to apply_affine, byte-exact
        // across non-trivial affine values. Pins the SpriteRenderer / Box-prefab
        // identity path that bypasses CSA-style offsets.
        let affine = Affine { tx: 1.5, ty: -2.25, sx: 1.25, sy: -0.75, rot_deg_ccw: 30.0 };
        let ctx = ctx_for_transform(affine, (0.0, 0.0));
        let v = [3.5, 4.5];
        let from_transform = apply_transform(v, &ctx);
        let from_affine = apply_affine(v, affine);
        assert_eq!(from_transform[0].to_bits(), from_affine[0].to_bits());
        assert_eq!(from_transform[1].to_bits(), from_affine[1].to_bits());
    }

    #[test]
    fn apply_transform_offset_added_after_scale() {
        // Composed leaf scale folded into affine.sx, offset pre-scaled by
        // the mode-implicit canvas factor: e.g. CSA leaf at pos=[100,-50]
        // gives offset=[1.0,-0.5], magnitude scale 1.0. v=2 → ×1 → 2 → +1.0 → 3.0.
        let ctx = ctx_for_transform(
            Affine { sx: 1.0, sy: 1.0, ..Affine::default() },
            (1.0, -0.5),
        );
        let out = apply_transform([2.0, 4.0], &ctx);
        assert_eq!(out, [3.0, 3.5]);
    }

    #[test]
    fn apply_transform_affine_translate_applied_last() {
        // tx/ty add in world units AFTER the per-part scale + offset.
        let ctx = ctx_for_transform(
            Affine { tx: 7.0, ty: 0.0, ..Affine::default() },
            (0.5, 0.0),
        );
        let out = apply_transform([0.0, 0.0], &ctx);
        assert_eq!(out, [7.5, 0.0]);
    }

    #[test]
    fn apply_transform_affine_negative_scale_flips_first() {
        // sx=-1 flips the input axis before the offset+translate; the
        // leaf's flip semantics now ride entirely on affine.sx/sy sign.
        let ctx = ctx_for_transform(
            Affine { sx: -1.0, ..Affine::default() },
            (0.0, 0.0),
        );
        let out = apply_transform([0.5, 0.0], &ctx);
        assert_eq!(out[0].to_bits(), (-0.5_f32).to_bits());
    }
}
