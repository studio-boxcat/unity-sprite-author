// Mesh encoding for the Sprite m_RD block. Ported from
// prefab-saloon/src/lib/sprite/generator.ts. Verified byte-exact for
// m_IndexBuffer (4 fixtures); typelessdata is byte-exact for IEEE-754
// inputs that round-trip.

use crate::tpsheet::{Pivot, Rect, Vertex};

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

#[derive(Debug, Clone, PartialEq)]
pub struct RenderData {
    pub vertex_count: u32,
    pub index_count: u32,
    pub data_size: u32,
    pub index_buffer_hex: String,
    pub typelessdata_hex: String,
    pub uv_transform: UvTransform,
}

// Mirrors SheetLoader.AssignToSprite (the 5-arg overload):
//   ps[i] = ((p - pivot * size) * scaleFactor)
// with all operations in f32. scaleFactor = spriteScale / ppu (precomputed
// f32 reciprocal). Mathematically equivalent to division by ppu, but the
// rounding is different — multiplying matches C# byte-for-byte.
pub fn pixel_to_local(v: Vertex, rect: Rect, pivot: Pivot, vertex_scale: f32) -> Position3 {
    let w = rect.w as f32;
    let h = rect.h as f32;
    Position3 {
        x: (v.x - pivot.x * w) * vertex_scale,
        y: (v.y - pivot.y * h) * vertex_scale,
        z: 0.0,
    }
}

pub fn pixel_to_uv(v: Vertex, rect: Rect, atlas_w: u32, atlas_h: u32) -> Uv {
    Uv {
        u: (rect.x as f32 + v.x) / atlas_w as f32,
        v: (rect.y as f32 + v.y) / atlas_h as f32,
    }
}

pub fn align_to_16(bytes: usize) -> usize {
    bytes.div_ceil(16) * 16
}

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

    (hex_lower(&buf), total)
}

pub fn encode_index_buffer(indices: &[u16]) -> String {
    let mut buf = Vec::with_capacity(indices.len() * 2);
    for &i in indices {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    hex_lower(&buf)
}

fn hex_lower(bytes: &[u8]) -> String {
    static HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct AtlasSize {
    pub width: u32,
    pub height: u32,
}

pub fn build(
    rect: Rect,
    pivot: Pivot,
    vertices: &[Vertex],
    indices: &[u16],
    ppu: f32,
    sprite_scale: f32,
    atlas: AtlasSize,
) -> RenderData {
    // Two distinct f32 divisions, mirroring TPSheetPostprocessor.cs:130-140:
    //   SpriteFactory.Create(..., ppu: ppu / spriteScale, ...)
    //   geo.AssignToSprite(..., spriteScale / ppu)
    // ptu and vertex_scale are NOT exact reciprocals in f32; both are stored.
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

    let uv_transform = UvTransform {
        x: pixels_to_units,
        y: rect.x as f32 + rect.w as f32 * pivot.x,
        z: pixels_to_units,
        w: rect.y as f32 + rect.h as f32 * pivot.y,
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
    use crate::tpsheet::{self, Border, SpriteAlignment};

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

    #[test]
    fn alignment_unused_silences_warning() {
        // Defensive: keep a reference so unused-import warnings don't flag
        // the alignment/border imports needed in higher-level tests later.
        let _ = SpriteAlignment::Center;
        let _ = Border::default();
    }
}
