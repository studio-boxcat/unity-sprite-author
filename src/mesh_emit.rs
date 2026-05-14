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

use std::fmt::Write as _;

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
    let _ = write!(s, "    indexCount: {ic}\n");
    s.push_str("    topology: 0\n");
    s.push_str("    baseVertex: 0\n");
    s.push_str("    firstVertex: 0\n");
    let _ = write!(s, "    vertexCount: {vc}\n");
    s.push_str("    localAABB:\n");
    let _ = write!(
        s,
        "      m_Center: {{x: {}, y: {}, z: {}}}\n",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = write!(
        s,
        "      m_Extent: {{x: {}, y: {}, z: {}}}\n",
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
    let _ = write!(s, "  m_KeepVertices: {}\n", m.keep_vertices as u8);
    let _ = write!(s, "  m_KeepIndices: {}\n", m.keep_indices as u8);
    s.push_str("  m_IndexFormat: 0\n");
    s.push_str("  m_IndexBuffer: ");
    encode_index_buffer(&m.indices, &mut s);
    s.push('\n');
    s.push_str("  m_VertexData:\n");
    s.push_str("    serializedVersion: 3\n");
    let _ = write!(s, "    m_VertexCount: {vc}\n");
    s.push_str("    m_Channels:\n");
    emit_channels_canvas(&mut s);
    let _ = write!(s, "    m_DataSize: {data_size}\n");
    s.push_str("    _typelessdata: ");
    encode_vbo_canvas(&m.vertices, &m.colors, &m.uvs, &mut s);
    s.push('\n');
    s.push_str("  m_CompressedMesh:\n");
    s.push_str(EMPTY_COMPRESSED_MESH);
    s.push_str("  m_LocalAABB:\n");
    let _ = write!(
        s,
        "    m_Center: {{x: {}, y: {}, z: {}}}\n",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = write!(
        s,
        "    m_Extent: {{x: {}, y: {}, z: {}}}\n",
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
        let _ = write!(s, "      offset: {off}\n");
        let _ = write!(s, "      format: {fmt}\n");
        let _ = write!(s, "      dimension: {dim}\n");
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
    let _ = write!(s, "    indexCount: {ic}\n");
    s.push_str("    topology: 0\n");
    s.push_str("    baseVertex: 0\n");
    s.push_str("    firstVertex: 0\n");
    let _ = write!(s, "    vertexCount: {vc}\n");
    s.push_str("    localAABB:\n");
    let _ = write!(
        s,
        "      m_Center: {{x: {}, y: {}, z: {}}}\n",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = write!(
        s,
        "      m_Extent: {{x: {}, y: {}, z: {}}}\n",
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
    let _ = write!(s, "  m_KeepVertices: {}\n", m.keep_vertices as u8);
    let _ = write!(s, "  m_KeepIndices: {}\n", m.keep_indices as u8);
    s.push_str("  m_IndexFormat: 0\n");
    s.push_str("  m_IndexBuffer: ");
    encode_index_buffer(&m.indices, &mut s);
    s.push('\n');
    s.push_str("  m_VertexData:\n");
    s.push_str("    serializedVersion: 3\n");
    let _ = write!(s, "    m_VertexCount: {vc}\n");
    s.push_str("    m_Channels:\n");
    emit_channels_sprite(&mut s);
    let _ = write!(s, "    m_DataSize: {data_size}\n");
    s.push_str("    _typelessdata: ");
    encode_vbo_sprite(&m.vertices, &m.uvs, &mut s);
    s.push('\n');
    s.push_str("  m_CompressedMesh:\n");
    s.push_str(EMPTY_COMPRESSED_MESH);
    s.push_str("  m_LocalAABB:\n");
    let _ = write!(
        s,
        "    m_Center: {{x: {}, y: {}, z: {}}}\n",
        yaml::float(m.aabb_center[0]),
        yaml::float(m.aabb_center[1]),
        yaml::float(m.aabb_center[2])
    );
    let _ = write!(
        s,
        "    m_Extent: {{x: {}, y: {}, z: {}}}\n",
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
        let _ = write!(s, "      offset: {off}\n");
        let _ = write!(s, "      format: {fmt}\n");
        let _ = write!(s, "      dimension: {dim}\n");
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

/// Emit a full multi-mesh `.asset` file. Header is the standard two-line
/// Unity YAML preamble; each `MeshAsset` becomes a `--- !u!43 &<file_id>`
/// section whose body is dispatched on `used_in_canvas`.
pub fn emit_mesh_asset(meshes: &[MeshAsset]) -> Vec<u8> {
    let mut s = String::with_capacity(128 + meshes.len() * 6000);
    s.push_str("%YAML 1.1\n");
    s.push_str("%TAG !u! tag:unity3d.com,2011:\n");
    for m in meshes {
        let _ = write!(s, "--- !u!43 &{}\n", m.file_id);
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
}
