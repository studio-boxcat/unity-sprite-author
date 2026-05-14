// SpriteMeshAuthor (SMA) byte-exact Mesh `.asset` emit.
//
// > **Related:** [[sma-migration.md]], [[CLAUDE.md]]
//
// Phase 2b foundation. SMA in meow-tower produces Mesh `.asset` files
// (not Sprite), one or more Mesh sub-objects per file. The bar is the
// same byte-exact contract this crate already meets for Sprite emit.
//
// # The on-disk shape
//
// Captured ground-truth at `tests/golden/sma/box_29_ghost/Box_29_Ghost.asset`
// (6165 lines, 32 Mesh sub-objects). Each Mesh sub-object is ~180 lines
// of YAML keyed `!u!43 &<fileID>`. Per-mesh structure (canvas-renderer
// path, `UsedInCanvas = true`):
//
//   m_SubMeshes:
//   - firstByte / indexCount / topology(0=Tri) / baseVertex / firstVertex
//     vertexCount / localAABB { m_Center, m_Extent }
//   m_Shapes / m_BindPose / m_BoneNameHashes / etc. : all empty
//   m_MeshCompression: 0
//   m_IsReadable: 1   (Canvas) / 0 (SpriteRenderer)
//   m_KeepVertices: 1 / m_KeepIndices: 1
//   m_IndexFormat: 0  (u16)
//   m_IndexBuffer: u16 LE hex
//   m_VertexData:
//     m_VertexCount, m_Channels[14], m_DataSize, _typelessdata (interleaved VBO)
//   m_CompressedMesh: all-empty fixture
//   m_LocalAABB:   duplicates submesh AABB
//   m_MeshUsageFlags / m_CookingOptions / m_BakedConvex/TriangleCollisionMesh
//   'm_MeshMetrics[0..1]': 1 / m_MeshOptimizationFlags / m_StreamData / m_MeshLodInfo
//
// # VBO layouts
//
// Two variants per `SpriteMeshAuthor.UsedInCanvas`:
//
// - **CanvasRenderer** (`UsedInCanvas = true`): channels 0/3/4 active —
//   position(f32x3, offset 0) + color(Color32, offset 12) + uv0(f32x2, offset 16).
//   Total stride 24 bytes. m_IsReadable: 1.
//
// - **SpriteRenderer** (`UsedInCanvas = false`): channels 0/4 active —
//   position(f32x3, offset 0) + uv0(Float16x2, offset 12). Total stride 16 bytes.
//   m_IsReadable: 0 (UploadMeshData(markNoLongerReadable: true)).
//
//   Float16 = IEEE 754 binary16, round-to-nearest-even; must byte-match
//   Unity's `Mathf.FloatToHalf`. Pin via fixtures once implemented.
//
// # Build path (mirrors SpriteMeshBuilder.ExtractMeshDataByHierarchicalOrder)
//
// 1. DFS-collect SpriteRenderers under SMA root (active only).
// 2. Pick tex/material from first white-colored renderer; validate
//    homogeneity (same tex+material+color across renderers).
// 3. Per renderer, build matrix chain:
//        flip = diag(flipX ? -1 : 1, flipY ? -1 : 1, 1)
//        cis  = root.worldToLocal × renderer.localToWorld × flip
// 4. Per renderer, dispatch on drawMode:
//    - Simple: copy sprite.vertices/uv/triangles, MultiplyPoint per vert.
//    - Tiled (Continuous only, axis-aligned quad sprite): tile the quad
//      across renderer.size with partial last-tile UV slicing. Assert
//      `tileRepeatX * tileRepeatY < 200`. See SpriteMeshBuilder.cs:183.
// 5. Append into combined (ps, uvs, tris) buffers.
//
// # Manifest schema (deferred)
//
// Either extend `.tps.fab.json` with a `type: "mesh"` discriminator on
// each combined entry, or introduce a sibling `<atlas>.tps.mesh.json`.
// Decision pending; the dumper (`examples/sma_dumper.cs`, to be written)
// will pick the encoding.
//
// # Sub-task checklist (TDD-friendly bites)
//
// 1. `MeshAsset` struct + serde for inputs.
// 2. f32 → IEEE 754 binary16 conversion, byte-port `Mathf.FloatToHalf`.
//    Pin against a Unity-side scratch dump of (f32, half) pairs across
//    +∞, −∞, 0, ±max-normal, ±min-normal, NaN, denormal.
// 3. YAML emitter for an empty single Mesh sub-asset (no verts/tris) —
//    match every byte of one mesh extracted from the fixture above.
// 4. Channels[14] emit (most slots empty, two/three active).
// 5. Interleaved VBO emit for CanvasRenderer layout (f32 + Color32 + f32).
// 6. Index buffer emit (u16 LE hex).
// 7. localAABB emit (matches SubMesh AABB).
// 8. Multi-mesh asset (cat sub-asset blocks separated by `--- !u!43 &<fileID>`).
// 9. SpriteRenderer VBO layout (f32 + half2).
// 10. Tiled mesh generator.
//
// Each step has a clear byte-diff oracle in the fixture. Currently nothing
// is implemented — this module is a stub.

#![allow(dead_code)]

/// Single Mesh sub-asset within a multi-mesh `.asset` file. Mirrors Unity's
/// `Mesh` Inspector fields that round-trip through `EditorUtility.CopySerialized`.
#[derive(Debug, Clone)]
pub struct MeshAsset {
    /// Fileid for the sub-asset block header (`--- !u!43 &<file_id>`).
    /// Stable hash derived from the prefab's mesh name (see SMA's
    /// `CreateOrGetSubAssetMesh`).
    pub file_id: i64,

    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u16>,

    /// `true` → CanvasRenderer layout (position + color + uv f32, readable).
    /// `false` → SpriteRenderer layout (position + uv f16, mark-not-readable).
    pub used_in_canvas: bool,

    /// AABB center + extent in mesh-local coordinates. Computed by Unity
    /// from the vertex set; we compute it the same way and pin via tests.
    pub aabb_center: [f32; 3],
    pub aabb_extent: [f32; 3],
}

/// Stub — implementation incoming per the checklist in this file's header.
pub fn emit_mesh_asset(_meshes: &[MeshAsset]) -> Vec<u8> {
    unimplemented!("mesh_emit::emit_mesh_asset is a Phase 2b stub")
}
