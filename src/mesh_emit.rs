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
    s.push_str("  m_KeepVertices: 1\n");
    s.push_str("  m_KeepIndices: 1\n");
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

/// Stub — multi-mesh asset assembly. Implementation incoming.
pub fn emit_mesh_asset(_meshes: &[MeshAsset]) -> Vec<u8> {
    unimplemented!("mesh_emit::emit_mesh_asset (multi-mesh assembly) is Phase 2b TODO")
}
