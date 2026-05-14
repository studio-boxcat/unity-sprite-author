// SpriteMeshAuthor (SMA) byte-exact Mesh `.asset` emit.
//
// > **Related:** [[sma-migration.md]], [[CLAUDE.md]]
//
// Implementation in progress. Currently:
// - `MeshAsset` struct + builder
// - YAML emit for one Mesh sub-asset (CanvasRenderer layout, single
//   submesh). Byte-exact against `Box_29_Ghost.asset` Mesh entries —
//   pinned by `tests/golden_sma_mesh.rs::box_29_ghost_first_mesh_byte_exact`.
//
// Deferred for next iterations:
// - SpriteRenderer VBO layout (f32 + half2, m_IsReadable=0).
// - Float16 conversion (`Mathf.FloatToHalf` byte-port).
// - Multi-mesh asset header sequencing (`--- !u!43 &<fileID>`).
// - Tiled mesh generator + SpriteRenderer hierarchy dumper.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::mesh_manifest::{DrawMode, MeshCombined};
use crate::tpsheet::SpriteEntry;
use crate::yaml;

/// A single Mesh sub-asset within a multi-mesh `.asset` file.
#[derive(Debug, Clone)]
pub struct MeshAsset {
    pub file_id: i64,
    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub colors: Vec<[u8; 4]>,
    pub indices: Vec<u16>,
    pub used_in_canvas: bool,
    /// Mirrors Unity's per-mesh `m_KeepVertices` / `m_KeepIndices`. SMA's
    /// SetMesh leaves them at 1 for most meshes but at 0 when Unity drops
    /// CPU-side copies (empirically: large or non-canvas meshes). Stored
    /// per-mesh so round-trip mirrors the asset bit-for-bit.
    pub keep_vertices: bool,
    pub keep_indices: bool,
    pub aabb_center: [f32; 3],
    pub aabb_extent: [f32; 3],
}

impl MeshAsset {
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }
}

/// Emit one Mesh sub-asset's YAML body (everything after the
/// `--- !u!43 &<fileID>` header line; the `Mesh:` block itself).
///
/// CanvasRenderer layout only for v1: position f32x3 + color Color32 +
/// uv0 f32x2 interleaved per-vertex (24-byte stride). SpriteRenderer
/// layout (half2 UV) deferred.
pub fn emit_mesh_body_canvas(m: &MeshAsset) -> String {
    assert!(
        m.used_in_canvas,
        "SpriteRenderer layout (used_in_canvas=false) not yet implemented"
    );
    assert_eq!(m.vertices.len(), m.uvs.len());
    assert_eq!(m.vertices.len(), m.colors.len());

    let vc = m.vertices.len();
    let ic = m.indices.len();
    let data_size = vc * 24; // 12 + 4 + 8 per vert
    let mut s = String::with_capacity(2048 + vc * 56);

    s.push_str("Mesh:\n");
    s.push_str("  m_ObjectHideFlags: 0\n");
    s.push_str("  m_CorrespondingSourceObject: {fileID: 0}\n");
    s.push_str("  m_PrefabInstance: {fileID: 0}\n");
    s.push_str("  m_PrefabAsset: {fileID: 0}\n");
    // Name: empty in the corpus — trailing space then LF reproduces the
    // YAML emitter's behavior for an empty scalar.
    s.push_str("  m_Name: \n");
    s.push_str("  serializedVersion: 12\n");
    s.push_str("  m_SubMeshes:\n");
    s.push_str("  - serializedVersion: 2\n");
    s.push_str("    firstByte: 0\n");
    let _ = writeln!(s, "    indexCount: {ic}");
    s.push_str("    topology: 0\n");
    s.push_str("    baseVertex: 0\n");
    s.push_str("    firstVertex: 0\n");
    let _ = writeln!(s, "    vertexCount: {vc}");
    s.push_str("    localAABB:\n");
    let _ = writeln!(
        s,
        "      m_Center: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = writeln!(
        s,
        "      m_Extent: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_extent[0]),
        yaml::float(m.aabb_extent[1]),
        yaml::float(m.aabb_extent[2])
    );
    s.push_str("  m_Shapes:\n");
    s.push_str("    vertices: []\n");
    s.push_str("    shapes: []\n");
    s.push_str("    channels: []\n");
    s.push_str("    fullWeights: []\n");
    s.push_str("  m_BindPose: []\n");
    s.push_str("  m_BoneNameHashes: \n");
    s.push_str("  m_RootBoneNameHash: 0\n");
    s.push_str("  m_BonesAABB: []\n");
    s.push_str("  m_VariableBoneCountWeights:\n");
    s.push_str("    m_Data: \n");
    s.push_str("  m_MeshCompression: 0\n");
    s.push_str("  m_IsReadable: 1\n");
    let _ = writeln!(s, "  m_KeepVertices: {}", m.keep_vertices as u8);
    let _ = writeln!(s, "  m_KeepIndices: {}", m.keep_indices as u8);
    s.push_str("  m_IndexFormat: 0\n");
    s.push_str("  m_IndexBuffer: ");
    encode_index_buffer(&m.indices, &mut s);
    s.push('\n');
    s.push_str("  m_VertexData:\n");
    s.push_str("    serializedVersion: 3\n");
    let _ = writeln!(s, "    m_VertexCount: {vc}");
    s.push_str("    m_Channels:\n");
    emit_channels_canvas(&mut s);
    let _ = writeln!(s, "    m_DataSize: {data_size}");
    s.push_str("    _typelessdata: ");
    encode_vbo_canvas(&m.vertices, &m.colors, &m.uvs, &mut s);
    s.push('\n');
    s.push_str("  m_CompressedMesh:\n");
    s.push_str(EMPTY_COMPRESSED_MESH);
    s.push_str("  m_LocalAABB:\n");
    let _ = writeln!(
        s,
        "    m_Center: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = writeln!(
        s,
        "    m_Extent: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_extent[0]),
        yaml::float(m.aabb_extent[1]),
        yaml::float(m.aabb_extent[2])
    );
    s.push_str("  m_MeshUsageFlags: 0\n");
    s.push_str("  m_CookingOptions: 30\n");
    s.push_str("  m_BakedConvexCollisionMesh: \n");
    s.push_str("  m_BakedTriangleCollisionMesh: \n");
    s.push_str("  'm_MeshMetrics[0]': 1\n");
    s.push_str("  'm_MeshMetrics[1]': 1\n");
    s.push_str("  m_MeshOptimizationFlags: 1\n");
    s.push_str(STREAM_AND_LOD_TAIL);
    s
}

fn emit_channels_canvas(s: &mut String) {
    // 14 channels total. Active: 0 (pos f32x3, off 0), 3 (color u8x4, off 12),
    // 4 (uv0 f32x2, off 16). All others empty.
    for i in 0..14 {
        let (off, fmt, dim) = match i {
            0 => (0u32, 0u32, 3u32),
            3 => (12, 2, 4),
            4 => (16, 0, 2),
            _ => (0, 0, 0),
        };
        s.push_str("    - stream: 0\n");
        let _ = writeln!(s, "      offset: {off}");
        let _ = writeln!(s, "      format: {fmt}");
        let _ = writeln!(s, "      dimension: {dim}");
    }
}

fn encode_index_buffer(indices: &[u16], s: &mut String) {
    for &i in indices {
        let _ = write!(s, "{:02x}{:02x}", i as u8, (i >> 8) as u8);
    }
}

fn encode_vbo_canvas(
    verts: &[[f32; 3]],
    colors: &[[u8; 4]],
    uvs: &[[f32; 2]],
    s: &mut String,
) {
    for i in 0..verts.len() {
        // position: 3 × f32 LE
        for &c in &verts[i] {
            let bytes = c.to_le_bytes();
            for b in bytes {
                let _ = write!(s, "{:02x}", b);
            }
        }
        // color: 4 × u8 (R, G, B, A) — Color32 byte order
        for &b in &colors[i] {
            let _ = write!(s, "{:02x}", b);
        }
        // uv0: 2 × f32 LE
        for &c in &uvs[i] {
            let bytes = c.to_le_bytes();
            for b in bytes {
                let _ = write!(s, "{:02x}", b);
            }
        }
    }
}

const EMPTY_COMPRESSED_MESH: &str = "    m_Vertices:
      m_NumItems: 0
      m_Range: 0
      m_Start: 0
      m_Data: 
      m_BitSize: 0
    m_UV:
      m_NumItems: 0
      m_Range: 0
      m_Start: 0
      m_Data: 
      m_BitSize: 0
    m_Normals:
      m_NumItems: 0
      m_Range: 0
      m_Start: 0
      m_Data: 
      m_BitSize: 0
    m_Tangents:
      m_NumItems: 0
      m_Range: 0
      m_Start: 0
      m_Data: 
      m_BitSize: 0
    m_Weights:
      m_NumItems: 0
      m_Data: 
      m_BitSize: 0
    m_NormalSigns:
      m_NumItems: 0
      m_Data: 
      m_BitSize: 0
    m_TangentSigns:
      m_NumItems: 0
      m_Data: 
      m_BitSize: 0
    m_FloatColors:
      m_NumItems: 0
      m_Range: 0
      m_Start: 0
      m_Data: 
      m_BitSize: 0
    m_BoneIndices:
      m_NumItems: 0
      m_Data: 
      m_BitSize: 0
    m_Triangles:
      m_NumItems: 0
      m_Data: 
      m_BitSize: 0
    m_UVInfo: 0
";

const STREAM_AND_LOD_TAIL: &str = "  m_StreamData:
    serializedVersion: 2
    offset: 0
    size: 0
    path: 
  m_MeshLodInfo:
    serializedVersion: 2
    m_LodSelectionCurve:
      serializedVersion: 1
      m_LodSlope: 0
      m_LodBias: 0
    m_NumLevels: 1
    m_SubMeshes:
    - serializedVersion: 2
      m_Levels:
      - serializedVersion: 1
        m_IndexStart: 0
        m_IndexCount: 0
";

/// IEEE 754 binary16 (half) from f32, round-to-nearest-even. Mirrors
/// `Mathf.FloatToHalf` in Unity (which is in turn the standard IL2CPP
/// `Float.ToHalf` byte-stable port). Pinned by `tests::float_to_half_*`
/// against reference vectors covering normals / subnormals / zero /
/// ±inf / NaN.
pub fn float_to_half(f: f32) -> u16 {
    let x = f.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let mant = x & 0x7f_ffff;
    let exp_raw = ((x >> 23) & 0xff) as i32;

    if exp_raw == 0xff {
        // Inf or NaN.
        if mant == 0 {
            return sign | 0x7c00; // ±Inf
        }
        // NaN: preserve sign + signal-bit, top 10 bits of mantissa.
        let m16 = (mant >> 13) as u16;
        return sign | 0x7c00 | m16.max(1);
    }

    let exp = exp_raw - 112; // bias adjust: 127 (f32) - 15 (f16) = 112

    if exp >= 31 {
        // Overflow → ±Inf.
        return sign | 0x7c00;
    }
    if exp <= 0 {
        // Subnormal or underflow.
        if exp < -10 {
            return sign;
        }
        // Construct full mantissa with the implicit leading 1, then shift.
        let mant_full = mant | 0x80_0000;
        let shift = (14 - exp) as u32;
        // Round to nearest even.
        let half = 1u32 << (shift - 1);
        let mask = (1u32 << shift) - 1;
        let mut m = mant_full >> shift;
        let rem = mant_full & mask;
        if rem > half || (rem == half && (m & 1) == 1) {
            m += 1;
        }
        return sign | (m as u16);
    }

    // Normal range. Round mantissa from 23 bits to 10.
    let half = 1u32 << 12;
    let mask = (1u32 << 13) - 1;
    let mut m10 = mant >> 13;
    let rem = mant & mask;
    let mut e = exp;
    if rem > half || (rem == half && (m10 & 1) == 1) {
        m10 += 1;
        if m10 == 0x400 {
            // mantissa overflow → bump exponent
            m10 = 0;
            e += 1;
            if e >= 31 {
                return sign | 0x7c00;
            }
        }
    }
    sign | ((e as u16) << 10) | (m10 as u16)
}

/// Emit one Mesh sub-asset's YAML body for the **SpriteRenderer** layout:
/// position f32x3 + uv0 half2 interleaved per-vertex (16-byte stride),
/// `m_IsReadable: 0`. Mirrors `BuildMeshForSpriteRenderer` in
/// `SpriteMeshBuilder.cs`.
pub fn emit_mesh_body_sprite(m: &MeshAsset) -> String {
    assert!(
        !m.used_in_canvas,
        "CanvasRenderer layout (used_in_canvas=true) routes through emit_mesh_body_canvas"
    );
    assert_eq!(m.vertices.len(), m.uvs.len());

    let vc = m.vertices.len();
    let ic = m.indices.len();
    let data_size = vc * 16; // 12 pos + 4 half2 uv
    let mut s = String::with_capacity(2048 + vc * 40);

    s.push_str("Mesh:\n");
    s.push_str("  m_ObjectHideFlags: 0\n");
    s.push_str("  m_CorrespondingSourceObject: {fileID: 0}\n");
    s.push_str("  m_PrefabInstance: {fileID: 0}\n");
    s.push_str("  m_PrefabAsset: {fileID: 0}\n");
    s.push_str("  m_Name: \n");
    s.push_str("  serializedVersion: 12\n");
    s.push_str("  m_SubMeshes:\n");
    s.push_str("  - serializedVersion: 2\n");
    s.push_str("    firstByte: 0\n");
    let _ = writeln!(s, "    indexCount: {ic}");
    s.push_str("    topology: 0\n");
    s.push_str("    baseVertex: 0\n");
    s.push_str("    firstVertex: 0\n");
    let _ = writeln!(s, "    vertexCount: {vc}");
    s.push_str("    localAABB:\n");
    let _ = writeln!(
        s,
        "      m_Center: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = writeln!(
        s,
        "      m_Extent: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_extent[0]),
        yaml::float(m.aabb_extent[1]),
        yaml::float(m.aabb_extent[2])
    );
    s.push_str("  m_Shapes:\n");
    s.push_str("    vertices: []\n");
    s.push_str("    shapes: []\n");
    s.push_str("    channels: []\n");
    s.push_str("    fullWeights: []\n");
    s.push_str("  m_BindPose: []\n");
    s.push_str("  m_BoneNameHashes: \n");
    s.push_str("  m_RootBoneNameHash: 0\n");
    s.push_str("  m_BonesAABB: []\n");
    s.push_str("  m_VariableBoneCountWeights:\n");
    s.push_str("    m_Data: \n");
    s.push_str("  m_MeshCompression: 0\n");
    s.push_str("  m_IsReadable: 0\n");
    let _ = writeln!(s, "  m_KeepVertices: {}", m.keep_vertices as u8);
    let _ = writeln!(s, "  m_KeepIndices: {}", m.keep_indices as u8);
    s.push_str("  m_IndexFormat: 0\n");
    s.push_str("  m_IndexBuffer: ");
    encode_index_buffer(&m.indices, &mut s);
    s.push('\n');
    s.push_str("  m_VertexData:\n");
    s.push_str("    serializedVersion: 3\n");
    let _ = writeln!(s, "    m_VertexCount: {vc}");
    s.push_str("    m_Channels:\n");
    emit_channels_sprite(&mut s);
    let _ = writeln!(s, "    m_DataSize: {data_size}");
    s.push_str("    _typelessdata: ");
    encode_vbo_sprite(&m.vertices, &m.uvs, &mut s);
    s.push('\n');
    s.push_str("  m_CompressedMesh:\n");
    s.push_str(EMPTY_COMPRESSED_MESH);
    s.push_str("  m_LocalAABB:\n");
    let _ = writeln!(
        s,
        "    m_Center: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = writeln!(
        s,
        "    m_Extent: {{x: {}, y: {}, z: {}}}",
        yaml::float(m.aabb_extent[0]),
        yaml::float(m.aabb_extent[1]),
        yaml::float(m.aabb_extent[2])
    );
    s.push_str("  m_MeshUsageFlags: 0\n");
    s.push_str("  m_CookingOptions: 30\n");
    s.push_str("  m_BakedConvexCollisionMesh: \n");
    s.push_str("  m_BakedTriangleCollisionMesh: \n");
    s.push_str("  'm_MeshMetrics[0]': 1\n");
    s.push_str("  'm_MeshMetrics[1]': 1\n");
    s.push_str("  m_MeshOptimizationFlags: 1\n");
    s.push_str(STREAM_AND_LOD_TAIL);
    s
}

fn emit_channels_sprite(s: &mut String) {
    // Active: 0 (pos f32x3, off 0), 4 (uv0 half2, off 12, format=1).
    for i in 0..14 {
        let (off, fmt, dim) = match i {
            0 => (0u32, 0u32, 3u32),
            4 => (12, 1, 2),
            _ => (0, 0, 0),
        };
        s.push_str("    - stream: 0\n");
        let _ = writeln!(s, "      offset: {off}");
        let _ = writeln!(s, "      format: {fmt}");
        let _ = writeln!(s, "      dimension: {dim}");
    }
}

fn encode_vbo_sprite(verts: &[[f32; 3]], uvs: &[[f32; 2]], s: &mut String) {
    for i in 0..verts.len() {
        for &c in &verts[i] {
            for b in c.to_le_bytes() {
                let _ = write!(s, "{:02x}", b);
            }
        }
        for &c in &uvs[i] {
            let h = float_to_half(c);
            for b in h.to_le_bytes() {
                let _ = write!(s, "{:02x}", b);
            }
        }
    }
}

/// Tiled mesh generator — direct port of `SpriteMeshBuilder.GetMesh_Tiled`
/// (Continuous tileMode only). Inputs:
/// - `sprite_quad`: the source sprite's four pivot-relative world-unit
///   vertices in `(BL, BR, TL, TR)` order (Unity's `sprite.vertices` for
///   an axis-aligned quad).
/// - `sprite_uv`: the source sprite's outer-rect UV corners in
///   `(BL_min, TR_max)` order.
/// - `sprite_size_world`: `sprite.rect.size / sprite.pixelsPerUnit`.
/// - `sprite_pivot_norm`: normalized pivot `sprite.pivot / sprite.rect.size`.
/// - `draw_size`: the SpriteRenderer's `size` field (target rect, world units).
///
/// Returns interleaved (positions, uvs, triangles) in pivot-relative
/// world coords, ready for transform via the per-renderer matrix chain.
///
/// Asserts (mirroring the C# `Assert.IsTrue`s):
/// - `tileQuotientX < 0.999` and `tileQuotientY < 0.999`.
/// - `tileRepeatX * tileRepeatY < 200`.
pub fn tiled_mesh(
    sprite_quad: [[f32; 2]; 4],
    sprite_uv: [[f32; 2]; 2],
    sprite_size_world: [f32; 2],
    sprite_pivot_norm: [f32; 2],
    draw_size: [f32; 2],
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<u16>) {
    let pos_min = sprite_quad[0];
    let pos_max = sprite_quad[3];
    let pos_size = [pos_max[0] - pos_min[0], pos_max[1] - pos_min[1]];
    let uv_min = sprite_uv[0];
    let uv_max = sprite_uv[1];
    let uv_size = [uv_max[0] - uv_min[0], uv_max[1] - uv_min[1]];

    let tile_div = [
        draw_size[0] / sprite_size_world[0],
        draw_size[1] / sprite_size_world[1],
    ];
    let tile_repeat_x = tile_div[0].ceil() as i32;
    let tile_repeat_y = tile_div[1].ceil() as i32;
    let tile_quotient_x = if (tile_repeat_x as f32 - tile_div[0]) > 0.0001 {
        tile_div[0] - (tile_repeat_x - 1) as f32
    } else {
        0.0
    };
    let tile_quotient_y = if (tile_repeat_y as f32 - tile_div[1]) > 0.0001 {
        tile_div[1] - (tile_repeat_y - 1) as f32
    } else {
        0.0
    };
    assert!(tile_quotient_x < 0.999, "tile quotient X too large: {tile_quotient_x}");
    assert!(tile_quotient_y < 0.999, "tile quotient Y too large: {tile_quotient_y}");
    assert!(
        (tile_repeat_x * tile_repeat_y) < 200,
        "too many tiles: {tile_repeat_x} x {tile_repeat_y}"
    );

    let draw_pivot = sprite_pivot_norm;
    let tile_origin = [draw_size[0] * draw_pivot[0], draw_size[1] * draw_pivot[1]];
    let sprite_pivot_point = [
        draw_pivot[0] * sprite_size_world[0],
        draw_pivot[1] * sprite_size_world[1],
    ];
    let tile_offset = [
        -tile_origin[0] + sprite_pivot_point[0],
        -tile_origin[1] + sprite_pivot_point[1],
    ];

    let mut ps: Vec<[f32; 2]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut tris: Vec<u16> = Vec::new();
    let mut index_offset: u16 = 0;

    for y in 0..tile_repeat_y {
        for x in 0..tile_repeat_x {
            let mut p0 = pos_min;
            let mut p3 = pos_max;
            let uv0 = uv_min;
            let mut uv3 = uv_max;
            if x == tile_repeat_x - 1 && tile_quotient_x != 0.0 {
                p3[0] = pos_min[0] + pos_size[0] * tile_quotient_x;
                uv3[0] = uv_min[0] + uv_size[0] * tile_quotient_x;
            }
            if y == tile_repeat_y - 1 && tile_quotient_y != 0.0 {
                p3[1] = pos_min[1] + pos_size[1] * tile_quotient_y;
                uv3[1] = uv_min[1] + uv_size[1] * tile_quotient_y;
            }
            let tile_start = [
                tile_offset[0] + (x as f32) * sprite_size_world[0],
                tile_offset[1] + (y as f32) * sprite_size_world[1],
            ];
            p0[0] += tile_start[0];
            p0[1] += tile_start[1];
            p3[0] += tile_start[0];
            p3[1] += tile_start[1];

            ps.push([p0[0], p0[1]]);
            ps.push([p3[0], p0[1]]);
            ps.push([p0[0], p3[1]]);
            ps.push([p3[0], p3[1]]);

            uvs.push(uv0);
            uvs.push([uv3[0], uv0[1]]);
            uvs.push([uv0[0], uv3[1]]);
            uvs.push(uv3);

            // sprite.triangles for a quad is [0,1,2, 2,1,3] (Unity convention).
            for &t in &[0u16, 1, 2, 2, 1, 3] {
                tris.push(t + index_offset);
            }
            index_offset += 4;
        }
    }

    (ps, uvs, tris)
}

/// Build a single combined `MeshAsset` from a manifest entry + the
/// per-sprite source data the renderers reference.
///
/// `lookup` resolves each renderer's `sprite` field to a `SpriteEntry`
/// from the active `.tpsheet`. PPU and atlas size (for UV normalization)
/// must mirror what the SMA path produces — same as the per-tpsheet
/// emit. Errors are returned for unresolved sprite names; the caller
/// surfaces them with manifest context.
pub fn build_mesh<F>(
    combined: &MeshCombined,
    ppu: f32,
    atlas_size: (u32, u32),
    mut lookup: F,
) -> Result<MeshAsset, BuildMeshError>
where
    F: FnMut(&str) -> Option<SpriteEntry>,
{
    let mut all_verts: Vec<[f32; 3]> = Vec::new();
    let mut all_uvs: Vec<[f32; 2]> = Vec::new();
    let mut all_colors: Vec<[u8; 4]> = Vec::new();
    let mut all_tris: Vec<u16> = Vec::new();

    let inv_ppu = 1.0_f32 / ppu;
    let atlas_w = atlas_size.0 as f32;
    let atlas_h = atlas_size.1 as f32;

    for r in &combined.renderers {
        let entry = lookup(&r.sprite).ok_or_else(|| BuildMeshError::UnresolvedSprite {
            combined: combined.name.clone(),
            sprite: r.sprite.clone(),
        })?;

        // Source pivot-relative vertices and outer UV corners — derived
        // from the tpsheet entry. The SMA path always operates on
        // axis-aligned-quad sprites for Tiled, and on arbitrary sprite
        // meshes for Simple.
        let pw = entry.rect.w as f32;
        let ph = entry.rect.h as f32;
        let pivot_px = (pw * entry.pivot.x, ph * entry.pivot.y);
        let src_verts: Vec<[f32; 2]> = entry
            .geometry
            .vertices
            .iter()
            .map(|v| {
                [
                    (v.x - pivot_px.0) * inv_ppu,
                    (v.y - pivot_px.1) * inv_ppu,
                ]
            })
            .collect();
        let src_uvs: Vec<[f32; 2]> = entry
            .geometry
            .vertices
            .iter()
            .map(|v| {
                [
                    (entry.rect.x as f32 + v.x) / atlas_w,
                    (entry.rect.y as f32 + v.y) / atlas_h,
                ]
            })
            .collect();
        let src_tris: Vec<u16> = entry.geometry.triangles.clone();

        let (mut verts2, uvs, tris) = match r.draw_mode {
            DrawMode::Simple => (src_verts, src_uvs, src_tris),
            DrawMode::Tiled => {
                let size = r.size.expect("parser ensures tiled has size");
                let n = src_verts.len();
                if n != 4 {
                    return Err(BuildMeshError::TiledNonQuad {
                        combined: combined.name.clone(),
                        sprite: r.sprite.clone(),
                        vert_count: n,
                    });
                }
                let quad = [src_verts[0], src_verts[1], src_verts[2], src_verts[3]];
                let uv_min = src_uvs[0];
                let uv_max = src_uvs[3];
                let sprite_size_world = [pw * inv_ppu, ph * inv_ppu];
                let sprite_pivot_norm = [entry.pivot.x, entry.pivot.y];
                let (ps, uvs, tris) =
                    tiled_mesh(quad, [uv_min, uv_max], sprite_size_world, sprite_pivot_norm, size);
                (ps, uvs, tris)
            }
        };

        // Apply flip: negate x/y *before* the localToRoot matrix —
        // matches SpriteMeshBuilder.CalculateRendererToRootMatrix which
        // pre-multiplies by `diag(flipX ? -1 : 1, flipY ? -1 : 1, 1)`.
        if r.flip_x || r.flip_y {
            for v in verts2.iter_mut() {
                if r.flip_x {
                    v[0] = -v[0];
                }
                if r.flip_y {
                    v[1] = -v[1];
                }
            }
        }

        // Apply per-renderer 2D affine `local_to_root`. 8 floats, row-major:
        // [m00, m01, m02, m03, m10, m11, m12, m13]. z always 0.
        let m = r.local_to_root;
        let base = all_verts.len() as u16;
        for v in &verts2 {
            let x = m[0] * v[0] + m[1] * v[1] + m[3];
            let y = m[4] * v[0] + m[5] * v[1] + m[7];
            all_verts.push([x, y, 0.0]);
        }
        all_uvs.extend(uvs);
        if combined.used_in_canvas {
            for _ in &verts2 {
                all_colors.push([0xff, 0xff, 0xff, 0xff]);
            }
        }
        all_tris.extend(tris.iter().map(|i| i + base));
    }

    if all_verts.is_empty() {
        return Err(BuildMeshError::EmptyMesh(combined.name.clone()));
    }

    // AABB derives center/extent. Mirrors Unity's `Mesh.RecalculateBounds`
    // (center = midpoint, extent = half-size). Z stays 0.
    let mut min = all_verts[0];
    let mut max = all_verts[0];
    for v in &all_verts[1..] {
        for i in 0..3 {
            if v[i] < min[i] {
                min[i] = v[i];
            }
            if v[i] > max[i] {
                max[i] = v[i];
            }
        }
    }
    let center = [
        (max[0] + min[0]) * 0.5,
        (max[1] + min[1]) * 0.5,
        (max[2] + min[2]) * 0.5,
    ];
    let extent = [
        (max[0] - min[0]) * 0.5,
        (max[1] - min[1]) * 0.5,
        (max[2] - min[2]) * 0.5,
    ];

    Ok(MeshAsset {
        file_id: combined.file_id,
        name: combined.name.clone(),
        vertices: all_verts,
        uvs: all_uvs,
        colors: all_colors,
        indices: all_tris,
        used_in_canvas: combined.used_in_canvas,
        keep_vertices: combined.keep_vertices,
        keep_indices: combined.keep_indices,
        aabb_center: center,
        aabb_extent: extent,
    })
}

#[derive(Debug)]
pub enum BuildMeshError {
    UnresolvedSprite { combined: String, sprite: String },
    TiledNonQuad { combined: String, sprite: String, vert_count: usize },
    EmptyMesh(String),
}

impl std::fmt::Display for BuildMeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvedSprite { combined, sprite } => {
                write!(f, "mesh '{combined}' references unknown sprite '{sprite}'")
            }
            Self::TiledNonQuad { combined, sprite, vert_count } => write!(
                f,
                "mesh '{combined}' tiled renderer '{sprite}' has {vert_count} verts (need 4 for axis-aligned quad)"
            ),
            Self::EmptyMesh(n) => write!(f, "mesh '{n}' produced zero vertices"),
        }
    }
}

impl std::error::Error for BuildMeshError {}

/// Convenience for building a sprite-name index over a tpsheet.
pub fn sprite_index(entries: &[SpriteEntry]) -> HashMap<&str, &SpriteEntry> {
    entries.iter().map(|e| (e.name.as_str(), e)).collect()
}

/// Emit a full multi-mesh `.asset` file. Header is the standard two-line
/// Unity YAML preamble; each `MeshAsset` becomes a `--- !u!43 &<file_id>`
/// section whose body is dispatched on `used_in_canvas`.
pub fn emit_mesh_asset(meshes: &[MeshAsset]) -> Vec<u8> {
    let mut s = String::with_capacity(128 + meshes.len() * 6000);
    s.push_str("%YAML 1.1\n");
    s.push_str("%TAG !u! tag:unity3d.com,2011:\n");
    for m in meshes {
        let _ = writeln!(s, "--- !u!43 &{}", m.file_id);
        if m.used_in_canvas {
            s.push_str(&emit_mesh_body_canvas(m));
        } else {
            s.push_str(&emit_mesh_body_sprite(m));
        }
    }
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(f: f32) -> u16 {
        float_to_half(f)
    }

    #[test]
    fn float_to_half_zero_and_signs() {
        assert_eq!(h(0.0), 0x0000);
        assert_eq!(h(-0.0), 0x8000);
    }

    #[test]
    fn float_to_half_infinities() {
        assert_eq!(h(f32::INFINITY), 0x7C00);
        assert_eq!(h(f32::NEG_INFINITY), 0xFC00);
    }

    #[test]
    fn float_to_half_nan() {
        let n = h(f32::NAN);
        // Mantissa non-zero in the top 10 bits; exponent all-ones.
        assert_eq!(n & 0x7C00, 0x7C00);
        assert!(n & 0x03FF != 0);
    }

    #[test]
    fn float_to_half_one() {
        assert_eq!(h(1.0), 0x3C00);
        assert_eq!(h(-1.0), 0xBC00);
    }

    #[test]
    fn float_to_half_simple_fractions() {
        assert_eq!(h(0.5), 0x3800);
        assert_eq!(h(0.25), 0x3400);
        assert_eq!(h(2.0), 0x4000);
    }

    #[test]
    fn float_to_half_overflow_to_inf() {
        // 65520 is just above the largest representable half (65504); IEEE
        // 754 round-to-nearest sends it to +inf.
        assert_eq!(h(65520.0), 0x7C00);
        assert_eq!(h(-65520.0), 0xFC00);
    }

    #[test]
    fn float_to_half_max_half() {
        // 65504 = 0x7BFF (the largest finite half).
        assert_eq!(h(65504.0), 0x7BFF);
    }

    #[test]
    fn float_to_half_min_normal() {
        // 2^-14 = smallest positive normal half = 0x0400.
        assert_eq!(h(2.0f32.powi(-14)), 0x0400);
    }

    #[test]
    fn float_to_half_min_subnormal() {
        // 2^-24 = smallest positive subnormal half = 0x0001.
        assert_eq!(h(2.0f32.powi(-24)), 0x0001);
    }

    #[test]
    fn float_to_half_underflow_to_zero() {
        // Half of the smallest subnormal rounds to zero via round-to-even.
        assert_eq!(h(2.0f32.powi(-25)), 0x0000);
    }

    #[test]
    fn tiled_mesh_single_tile_unit_quad() {
        // 1×1 sprite tiled to 1×1 draw size: a single tile, no slicing.
        // Pivot at center → tile origin at (0.5, 0.5).
        let quad = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let uv = [[0.0, 0.0], [1.0, 1.0]];
        let (ps, uvs, tris) = tiled_mesh(quad, uv, [1.0, 1.0], [0.5, 0.5], [1.0, 1.0]);
        assert_eq!(ps.len(), 4);
        assert_eq!(uvs.len(), 4);
        assert_eq!(tris, vec![0, 1, 2, 2, 1, 3]);
        // Pivot-centered: tile_offset = (-0.5, -0.5) + (0.5, 0.5) = (0, 0).
        // tile_start = (0, 0). p0..p3 = source quad.
        assert_eq!(ps[0], [0.0, 0.0]);
        assert_eq!(ps[3], [1.0, 1.0]);
    }

    #[test]
    fn tiled_mesh_repeat_2x1_integer() {
        // 1×1 sprite, 2×1 draw. Two full tiles laid horizontally.
        let quad = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let uv = [[0.0, 0.0], [1.0, 1.0]];
        let (ps, uvs, tris) = tiled_mesh(quad, uv, [1.0, 1.0], [0.0, 0.0], [2.0, 1.0]);
        assert_eq!(ps.len(), 8);
        assert_eq!(uvs.len(), 8);
        assert_eq!(tris, vec![0, 1, 2, 2, 1, 3, 4, 5, 6, 6, 5, 7]);
        // Both tiles cover full UV [0,1].
        assert_eq!(uvs[0], [0.0, 0.0]);
        assert_eq!(uvs[3], [1.0, 1.0]);
        assert_eq!(uvs[4], [0.0, 0.0]);
        assert_eq!(uvs[7], [1.0, 1.0]);
    }

    #[test]
    fn tiled_mesh_partial_last_tile_x() {
        // 1×1 sprite, 1.5×1 draw. Two tiles in X, the last one half-sized.
        let quad = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let uv = [[0.0, 0.0], [1.0, 1.0]];
        let (ps, uvs, _tris) = tiled_mesh(quad, uv, [1.0, 1.0], [0.0, 0.0], [1.5, 1.0]);
        assert_eq!(ps.len(), 8);
        // First tile is full-size.
        assert_eq!(ps[0], [0.0, 0.0]);
        assert_eq!(ps[3], [1.0, 1.0]);
        // Second tile is half-width starting at x=1.
        assert_eq!(ps[4], [1.0, 0.0]);
        assert_eq!(ps[7], [1.5, 1.0]);
        // UV on the second tile is sliced to half on x.
        assert_eq!(uvs[7], [0.5, 1.0]);
    }

    use crate::mesh_manifest::{MeshCombined, MeshRenderer};
    use crate::tpsheet::{
        Border, Geometry, Pivot, Rect, SpriteAlignment, SpriteEntry, Vertex,
    };

    fn quad_entry(name: &str, rx: u32, ry: u32, w: u32, h: u32, px: f32, py: f32) -> SpriteEntry {
        // 4 verts in pixel coords relative to rect origin (BL, BR, TL, TR),
        // tris [0,1,2,2,1,3]. UVs derived later inside build_mesh.
        SpriteEntry {
            name: name.to_string(),
            rect: Rect { x: rx, y: ry, w, h },
            pivot: Pivot { x: px, y: py },
            alignment: SpriteAlignment::Custom,
            border: Border::default(),
            geometry: Geometry {
                vertices: vec![
                    Vertex { x: 0.0, y: 0.0 },
                    Vertex { x: w as f32, y: 0.0 },
                    Vertex { x: 0.0, y: h as f32 },
                    Vertex { x: w as f32, y: h as f32 },
                ],
                triangles: vec![0, 1, 2, 2, 1, 3],
            },
        }
    }

    fn renderer(sprite: &str, draw: DrawMode, size: Option<[f32; 2]>) -> MeshRenderer {
        MeshRenderer {
            sprite: sprite.to_string(),
            flip_x: false,
            flip_y: false,
            draw_mode: draw,
            size,
            local_to_root: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        }
    }

    #[test]
    fn build_mesh_simple_single_renderer_aabb_centered() {
        let entry = quad_entry("a", 0, 0, 100, 100, 0.5, 0.5);
        let mc = MeshCombined {
            file_id: 1,
            name: "test".into(),
            output_path: "out.asset".into(),
            used_in_canvas: true,
            keep_vertices: true,
            keep_indices: true,
            renderers: vec![renderer("a", DrawMode::Simple, None)],
        };
        let m = build_mesh(&mc, 100.0, (128, 128), |s| {
            if s == "a" {
                Some(entry.clone())
            } else {
                None
            }
        })
        .unwrap();
        assert_eq!(m.vertices.len(), 4);
        // 100px @ ppu=100 = 1.0 world; centered pivot ⇒ verts in [-0.5, 0.5]
        // → center=(0,0,0), extent=(0.5, 0.5, 0).
        assert_eq!(m.aabb_center, [0.0, 0.0, 0.0]);
        assert_eq!(m.aabb_extent, [0.5, 0.5, 0.0]);
        assert_eq!(m.colors.len(), 4);
        assert_eq!(m.colors[0], [0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn build_mesh_simple_translated_renderer() {
        let entry = quad_entry("a", 0, 0, 100, 100, 0.5, 0.5);
        let mut r = renderer("a", DrawMode::Simple, None);
        // translate by (10, 5) world units via the m03 / m13 slots.
        r.local_to_root = [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 5.0];
        let mc = MeshCombined {
            file_id: 1,
            name: "t".into(),
            output_path: "out.asset".into(),
            used_in_canvas: false,
            keep_vertices: false,
            keep_indices: false,
            renderers: vec![r],
        };
        let m = build_mesh(&mc, 100.0, (128, 128), |s| {
            (s == "a").then(|| entry.clone())
        })
        .unwrap();
        assert_eq!(m.aabb_center, [10.0, 5.0, 0.0]);
        assert!(m.colors.is_empty(), "SpriteRenderer layout doesn't carry colors");
    }

    #[test]
    fn build_mesh_flip_x_negates_before_matrix() {
        // sprite at native (0,0)..(1,1); flipX should put it at (-1, 0)..(0, 1).
        let entry = quad_entry("a", 0, 0, 100, 100, 0.0, 0.0);
        let mut r = renderer("a", DrawMode::Simple, None);
        r.flip_x = true;
        let mc = MeshCombined {
            file_id: 1,
            name: "flip".into(),
            output_path: "out.asset".into(),
            used_in_canvas: true,
            keep_vertices: true,
            keep_indices: true,
            renderers: vec![r],
        };
        let m = build_mesh(&mc, 100.0, (128, 128), |s| {
            (s == "a").then(|| entry.clone())
        })
        .unwrap();
        // verts span x in [-1, 0]: center.x = -0.5, extent.x = 0.5
        assert_eq!(m.aabb_center[0], -0.5);
        assert_eq!(m.aabb_extent[0], 0.5);
    }

    #[test]
    fn build_mesh_unresolved_sprite_errors() {
        let mc = MeshCombined {
            file_id: 1,
            name: "x".into(),
            output_path: "out.asset".into(),
            used_in_canvas: true,
            keep_vertices: true,
            keep_indices: true,
            renderers: vec![renderer("missing", DrawMode::Simple, None)],
        };
        let err = build_mesh(&mc, 100.0, (128, 128), |_| None).unwrap_err();
        assert!(matches!(err, BuildMeshError::UnresolvedSprite { .. }));
    }

    #[test]
    #[should_panic(expected = "too many tiles")]
    fn tiled_mesh_too_many_tiles_panics() {
        let quad = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let uv = [[0.0, 0.0], [1.0, 1.0]];
        // 20×20 tiles = 400 > 200 limit.
        tiled_mesh(quad, uv, [1.0, 1.0], [0.0, 0.0], [20.0, 20.0]);
    }
}
