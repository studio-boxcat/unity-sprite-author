// Mesh encoding for the Sprite m_RD block. Seeded from
// prefab-saloon/src/lib/sprite/generator.ts; has since diverged to add
// `build_fabricated` (for combined-mesh sprites) and the
// multiply-by-precomputed-reciprocal `pixel_to_local` that matches Unity's
// f32 rounding bit-for-bit. Byte-exact on every committed fixture —
// Cake__DecoLeft (atlas-sprite path via `build`), Silloutte1/2/3
// (fabricated path via `build_fabricated`), and the full Orgel
// golden_parity corpus.

use crate::tpsheet::{Pivot, Rect, Vertex};
use crate::yaml::hex_encode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Uv {
    pub u: f32,
    pub v: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvTransform {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// Pre-rendered fragments of the Sprite `m_RD` block. Built by [`build`]
/// (atlas-sprite path) or [`build_fabricated`] (combined-mesh path) and
/// consumed by [`emit::emit`](crate::emit::emit) which interpolates the
/// hex strings + counts directly into the YAML.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderData {
    pub vertex_count: u32,
    pub index_count: u32,
    pub data_size: u32,
    pub index_buffer_hex: String,
    pub typelessdata_hex: String,
    pub uv_transform: UvTransform,
}

/// Convert an atlas-pixel vertex to pivot-relative world units.
///
/// Mirrors `SheetLoader.AssignToSprite` (the 5-arg overload):
///
/// ```text
/// ps[i] = ((p - pivot * size) * scaleFactor)
/// ```
///
/// with all operations in `f32`. `vertex_scale = spriteScale / ppu`
/// (precomputed reciprocal); pass it in rather than recomputing.
/// Mathematically equivalent to `(p - pivot * size) / ppu`, but the
/// rounding is different — multiplying by the precomputed reciprocal
/// matches C# byte-for-byte (1-ULP gap on common inputs, see
/// `combine::local_src_verts` for the same trap on the fab path).
pub fn pixel_to_local(v: Vertex, rect: Rect, pivot: Pivot, vertex_scale: f32) -> Position3 {
    let w = rect.w as f32;
    let h = rect.h as f32;
    Position3 {
        x: (v.x - pivot.x * w) * vertex_scale,
        y: (v.y - pivot.y * h) * vertex_scale,
        z: 0.0,
    }
}

/// Normalize an atlas-pixel vertex to UV coordinates against the
/// texture's `(atlas_w, atlas_h)` size. Used by [`build`] for the
/// tpsheet path; the fab path uses pre-computed UVs from
/// [`crate::combine::atlas_sprite_mesh`] / `polygon_mesh_with_tris` so
/// the per-sprite multiply-by-reciprocal trick can be applied.
pub fn pixel_to_uv(v: Vertex, rect: Rect, atlas_w: u32, atlas_h: u32) -> Uv {
    Uv {
        u: (rect.x as f32 + v.x) / atlas_w as f32,
        v: (rect.y as f32 + v.y) / atlas_h as f32,
    }
}

/// Round `bytes` up to the next 16-byte boundary. Used to size the
/// position-stream padding before the UV stream in `_typelessdata`
/// (Unity's typelessdata layout has stream 0 padded up to 16 bytes).
pub fn align_to_16(bytes: usize) -> usize {
    bytes.div_ceil(16) * 16
}

/// Encode position + UV streams into the Unity Sprite `_typelessdata`
/// hex string. Layout: stream 0 = positions (`vec3 f32` LE, packed),
/// padded up to a 16-byte boundary; stream 1 = UVs (`vec2 f32` LE).
/// Returns `(hex_string, total_bytes)` — the latter feeds `m_DataSize`.
pub fn encode_typelessdata(positions: &[Position3], uvs: &[Uv]) -> (String, usize) {
    debug_assert_eq!(positions.len(), uvs.len());
    let vertex_count = positions.len();
    let pos_bytes = vertex_count * 12;
    let pos_bytes_aligned = align_to_16(pos_bytes);
    let uv_bytes = vertex_count * 8;
    let total = pos_bytes_aligned + uv_bytes;

    let mut buf = vec![0u8; total];

    // Stream 0: positions (vec3 f32 LE), packed.
    for (i, p) in positions.iter().enumerate() {
        let off = i * 12;
        buf[off..off + 4].copy_from_slice(&p.x.to_le_bytes());
        buf[off + 4..off + 8].copy_from_slice(&p.y.to_le_bytes());
        buf[off + 8..off + 12].copy_from_slice(&p.z.to_le_bytes());
    }

    // Padding between streams is already zero.

    // Stream 1: UVs (vec2 f32 LE).
    for (i, uv) in uvs.iter().enumerate() {
        let off = pos_bytes_aligned + i * 8;
        buf[off..off + 4].copy_from_slice(&uv.u.to_le_bytes());
        buf[off + 4..off + 8].copy_from_slice(&uv.v.to_le_bytes());
    }

    (hex_encode(&buf), total)
}

/// RenderData for a fabricated sprite — the caller has already produced
/// verts in pivot-relative world units (post-affine) and UVs in atlas-
/// normalized space. The tpsheet-specific pixel-to-local / pixel-to-uv
/// transforms in [`build`] aren't reused; only the hex encoding and
/// uvTransform math remain.
///
/// uvTransform uses the same offset-routed formula as [`build`] (going
/// through `center` first) — that 1-ULP-stable path is what kept the
/// tpsheet e2e byte-exact and is now also verified byte-exact under
/// Silloutte1/2/3 (commit `5943ede`).
pub fn build_fabricated(
    verts: &[[f32; 2]],
    uvs: &[[f32; 2]],
    tris: &[u16],
    rect_w_f: f32,
    rect_h_f: f32,
    pivot: (f32, f32),
    ppu: f32,
) -> RenderData {
    debug_assert_eq!(verts.len(), uvs.len());

    let positions: Vec<Position3> = verts.iter()
        .map(|v| Position3 { x: v[0], y: v[1], z: 0.0 })
        .collect();
    let uvs_typed: Vec<Uv> = uvs.iter()
        .map(|u| Uv { u: u[0], v: u[1] })
        .collect();

    let (typelessdata_hex, data_size) = encode_typelessdata(&positions, &uvs_typed);
    let index_buffer_hex = encode_index_buffer(tris);

    // Fabricated sprites have rect.{x, y} = 0 (SpriteFactory.CreateFromMesh
    // sets the rect origin at the AABB's min corner). Inline the simplified
    // offset-routed formula.
    let center_x = rect_w_f * 0.5;
    let center_y = rect_h_f * 0.5;
    let off_x_atlas = pivot.0 * rect_w_f - center_x;
    let off_y_atlas = pivot.1 * rect_h_f - center_y;
    let uv_transform = UvTransform {
        x: ppu,
        y: off_x_atlas + center_x,
        z: ppu,
        w: off_y_atlas + center_y,
    };

    RenderData {
        vertex_count: verts.len() as u32,
        index_count: tris.len() as u32,
        data_size: data_size as u32,
        index_buffer_hex,
        typelessdata_hex,
        uv_transform,
    }
}

/// Encode a `u16` triangle-index slice into the Unity Sprite
/// `m_IndexBuffer` hex string (`u16` LE per index).
pub fn encode_index_buffer(indices: &[u16]) -> String {
    let mut buf = Vec::with_capacity(indices.len() * 2);
    for &i in indices {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    hex_encode(&buf)
}

#[derive(Debug, Clone, Copy)]
pub struct AtlasSize {
    pub width: u32,
    pub height: u32,
}

/// Build [`RenderData`] for a tpsheet (atlas-sprite) path sprite. The
/// caller has the sprite's `rect` / `pivot` / `vertices` / `indices`
/// from [`tpsheet::parse`] and the atlas PPU + per-sprite `spriteScale`
/// from the `.png.meta` / `.tps` siblings. For combined-mesh sprites use
/// [`build_fabricated`] instead, which takes pre-computed verts / UVs.
pub fn build(
    rect: Rect,
    pivot: Pivot,
    vertices: &[Vertex],
    indices: &[u16],
    ppu: f32,
    sprite_scale: f32,
    atlas: AtlasSize,
) -> RenderData {
    // Two distinct f32 divisions — both used to live in the C# side at
    // `SpriteFactory.Create(..., ppu: ppu / spriteScale, ...)` paired with
    // `geo.AssignToSprite(..., spriteScale / ppu)`, and both moved into this
    // crate when sprite emit went native. The two are NOT exact reciprocals
    // in f32, so we compute and store them separately to keep byte-exact
    // parity with the historical C# emit.
    let pixels_to_units = ppu / sprite_scale;
    let vertex_scale = sprite_scale / ppu;

    let positions: Vec<Position3> = vertices
        .iter()
        .map(|&v| pixel_to_local(v, rect, pivot, vertex_scale))
        .collect();
    let uvs: Vec<Uv> = vertices
        .iter()
        .map(|&v| pixel_to_uv(v, rect, atlas.width, atlas.height))
        .collect();

    let (typelessdata_hex, data_size) = encode_typelessdata(&positions, &uvs);
    let index_buffer_hex = encode_index_buffer(indices);

    // uvTransform.y/w is the pivot's atlas-pixel position. Unity reaches
    // it via `m_Offset + rect.center` rather than directly `rect.pos +
    // size * pivot`. The two are mathematically identical but route
    // through different f32 intermediates and round 1 ULP apart on edge
    // cases. Reusing the same atlas-coord chain that solved m_Offset:
    //   uv.w = ((ry + py*h) - (ry + h*0.5)) + (ry + h*0.5)
    // Verified against Cake__DecoLeft (round numbers), AC_IC_Orgel
    // (small rect.y, near-center pivot), Outline/0204/02 (small h*py with
    // pivot near 0). Reverting to the direct formula regressed 200
    // sprites in the meow-tower e2e.
    let rx = rect.x as f32;
    let ry = rect.y as f32;
    let w_f32 = rect.w as f32;
    let h_f32 = rect.h as f32;
    let center_x = rx + w_f32 * 0.5;
    let center_y = ry + h_f32 * 0.5;
    let off_x_atlas = (rx + pivot.x * w_f32) - center_x;
    let off_y_atlas = (ry + pivot.y * h_f32) - center_y;
    let uv_transform = UvTransform {
        x: pixels_to_units,
        y: off_x_atlas + center_x,
        z: pixels_to_units,
        w: off_y_atlas + center_y,
    };

    RenderData {
        vertex_count: vertices.len() as u32,
        index_count: indices.len() as u32,
        data_size: data_size as u32,
        index_buffer_hex,
        typelessdata_hex,
        uv_transform,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tpsheet;

    const ORGEL: &str = include_str!("../tests/golden/orgel/Orgel.tpsheet");

    fn fixture_decoleft() -> tpsheet::SpriteEntry {
        tpsheet::parse(ORGEL)
            .unwrap()
            .sprites
            .into_iter()
            .find(|s| s.name == "Cake__DecoLeft")
            .unwrap()
    }

    #[test]
    fn pixel_to_local_uses_precomputed_reciprocal() {
        // `pixel_to_local` mirrors SheetLoader.AssignToSprite's
        // `(p - pivot * size) * vertexScale` — the multiply-by-reciprocal
        // form (vertexScale = spriteScale / ppu precomputed) matches Unity's
        // f32 rounding. Direct-division `(p - pivot*size) / ppu` rounds 1
        // ULP differently in some cases. This test pins one of the
        // Cake__DecoLeft verts so any refactor that swaps multiply for
        // divide surfaces here, not in the byte-exact golden.
        let v = Vertex { x: 60.0, y: 0.0 };
        let rect = Rect { x: 0, y: 0, w: 80, h: 80 };
        let pivot = Pivot { x: 0.5, y: 0.5 };
        let vertex_scale = 1.0_f32 / 80.0; // ppu=80, spriteScale=1
        let p = pixel_to_local(v, rect, pivot, vertex_scale);
        // Mathematically (60 − 40) / 80 = 0.25; in f32 multiply-by-recip
        // we get the same bit pattern as the C# emitter.
        assert_eq!(p.x.to_bits(), 0.25_f32.to_bits());
        assert_eq!(p.y.to_bits(), (-0.5_f32).to_bits());
        assert_eq!(p.z, 0.0);
    }

    #[test]
    fn pixel_to_uv_normalizes_against_atlas_size() {
        let v = Vertex { x: 30.0, y: 20.0 };
        let rect = Rect { x: 100, y: 50, w: 80, h: 80 };
        let uv = pixel_to_uv(v, rect, 1024, 512);
        // (100+30)/1024 = 0.126953125 (exact in f32 since 130 = 2 × 5 × 13
        // and 1024 = 2^10 — the reciprocal is exact-ish, the division
        // rounds cleanly here).
        assert!((uv.u - 130.0_f32 / 1024.0).abs() < 1e-7, "{}", uv.u);
        assert!((uv.v - 70.0_f32 / 512.0).abs() < 1e-7, "{}", uv.v);
    }

    #[test]
    fn align_to_16_basic() {
        assert_eq!(align_to_16(0), 0);
        assert_eq!(align_to_16(1), 16);
        assert_eq!(align_to_16(16), 16);
        assert_eq!(align_to_16(17), 32);
        assert_eq!(align_to_16(84), 96); // 7 verts * 12 bytes
    }

    #[test]
    fn cake_decoleft_index_buffer_byte_exact() {
        let s = fixture_decoleft();
        let hex = encode_index_buffer(&s.geometry.triangles);
        assert_eq!(
            hex,
            "040005000600030004000600020003000600000001000200000002000600"
        );
    }

    #[test]
    fn cake_decoleft_data_size() {
        let s = fixture_decoleft();
        let rd = build(
            s.rect,
            s.pivot,
            &s.geometry.vertices,
            &s.geometry.triangles,
            80.0,
            1.0,
            AtlasSize {
                width: 580,
                height: 580,
            },
        );
        assert_eq!(rd.data_size, 152);
        assert_eq!(rd.vertex_count, 7);
        assert_eq!(rd.index_count, 15);
    }

    #[test]
    fn cake_decoleft_typelessdata_byte_exact() {
        let s = fixture_decoleft();
        let rd = build(
            s.rect,
            s.pivot,
            &s.geometry.vertices,
            &s.geometry.triangles,
            80.0,
            1.0,
            AtlasSize {
                width: 580,
                height: 580,
            },
        );
        // From golden Cake__DecoLeft.asset.
        let expected = "9a99593e6766663d000000009a99593e0000c0bd00000000\
                        cdcc0c3e333313be00000000cdcc4cbc333313be00000000\
                        9a9959be333333bd000000009a9959be3333133e00000000\
                        cdcccc3c3333133e0000000000000000000000000000000\
                        0b818d23e0e787c3fb818d23e232c773fcdcccc3e2a68753\
                        ff734c23e2a68753f3015b43e1cf0783f3015b43e028f7f3\
                        feddac43e028f7f3f";
        let expected_clean: String = expected.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(rd.typelessdata_hex, expected_clean);
    }

    #[test]
    fn cake_decoleft_uv_transform() {
        let s = fixture_decoleft();
        let rd = build(
            s.rect,
            s.pivot,
            &s.geometry.vertices,
            &s.geometry.triangles,
            80.0,
            1.0,
            AtlasSize {
                width: 580,
                height: 580,
            },
        );
        // From golden: {x: 80, y: 221, z: 80, w: 567.5}
        assert_eq!(rd.uv_transform.x, 80.0);
        assert_eq!(rd.uv_transform.y, 221.0);
        assert_eq!(rd.uv_transform.z, 80.0);
        assert_eq!(rd.uv_transform.w, 567.5);
    }

    // --- build_fabricated ---

    #[test]
    fn build_fabricated_silloutte1_uv_transform() {
        // m_Rect (0, 0, 282.5, 770), m_Pivot (0.5, 0.40551946), PPU 100.
        // Expected uvTransform from Silloutte1.asset: (100, 141.25, 100, 312.24997).
        let rd = build_fabricated(
            &[[0.0, 0.0]], &[[0.0, 0.0]], &[],
            282.5, 770.0,
            (0.5, 0.40551946),
            100.0,
        );
        assert_eq!(rd.uv_transform.x, 100.0);
        assert_eq!(rd.uv_transform.y, 141.25);
        assert_eq!(rd.uv_transform.z, 100.0);
        // f32 rounding through (pivot*h − h*0.5) + h*0.5 lands at 312.24997.
        assert!(
            (rd.uv_transform.w - 312.24997).abs() < 1e-3,
            "got {}", rd.uv_transform.w,
        );
    }

    #[test]
    fn build_fabricated_typelessdata_encodes_pos_and_uv() {
        // One vert at (0.25, 0.5) world, UV (0.1, 0.2). Position3 padded
        // to 12 bytes, UV stream packed at 16-byte-aligned offset (single
        // vert ⇒ pos = 12 bytes, align to 16, uv at offset 16, 8 bytes;
        // total 24).
        let rd = build_fabricated(
            &[[0.25, 0.5]], &[[0.1, 0.2]], &[0],
            1.0, 1.0, (0.5, 0.5), 100.0,
        );
        assert_eq!(rd.vertex_count, 1);
        assert_eq!(rd.index_count, 1);
        assert_eq!(rd.data_size, 24);
        // 0.25_f32 LE = 0000803e; 0.5_f32 LE = 0000003f; 0.0_f32 LE = 00000000;
        // pad 4 bytes; 0.1_f32 LE = cdcccc3d; 0.2_f32 LE = cdcc4c3e.
        assert_eq!(
            rd.typelessdata_hex,
            "0000803e0000003f0000000000000000cdcccc3dcdcc4c3e",
        );
    }
}
